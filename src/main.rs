#[macro_use]
extern crate log;

use futures_util::future::FutureExt;
use futures_util::StreamExt;
use serde::Deserialize;

mod proto {
    tonic::include_proto!("v1beta1");
}

mod uds;

#[derive(Debug, Deserialize)]
struct Config {
    ports: Vec<String>
}

#[derive(Clone, Debug)]
struct AppState {
    ports: std::collections::BTreeMap<uuid::Uuid, Port>
}

impl From<&Config> for AppState {
    fn from(config: &Config) -> Self {
        let mut map = std::collections::BTreeMap::new();

        for port in &config.ports {
            map.insert(uuid::Uuid::new_v4(), Port {
                path: port.to_string()
            });
        }

        AppState {
            ports: map
        }
    }
}

#[derive(Clone, Debug)]
struct Port {
    path: String
}

struct DevicePlugin {
    state: std::sync::Arc<tokio::sync::RwLock<AppState>>
}

struct ListDevices {
    devices: Vec<proto::Device>,
    sent: bool
}

impl futures_util::stream::Stream for ListDevices {
    type Item = Result<proto::ListAndWatchResponse, tonic::Status>;

    fn poll_next(self: std::pin::Pin<&mut Self>, _cx: &mut std::task::Context<'_>) -> std::task::Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if !this.sent {
            this.sent = true;
            std::task::Poll::Ready(Some(Ok(proto::ListAndWatchResponse {
                devices: this.devices.clone(),
            })))
        } else {
            std::task::Poll::Pending
        }
    }
}

#[tonic::async_trait]
impl proto::device_plugin_server::DevicePlugin for DevicePlugin {
    type ListAndWatchStream = std::pin::Pin<Box<dyn futures_util::stream::Stream<Item=Result<proto::ListAndWatchResponse, tonic::Status>> + Send + Sync>>;

    async fn get_device_plugin_options(
        &self,
        _request: tonic::Request<proto::Empty>,
    ) -> Result<tonic::Response<proto::DevicePluginOptions>, tonic::Status> {
        Ok(tonic::Response::new(proto::DevicePluginOptions {
            pre_start_required: false
        }))
    }

    async fn list_and_watch(
        &self,
        _request: tonic::Request<proto::Empty>,
    ) -> Result<tonic::Response<Self::ListAndWatchStream>, tonic::Status> {
        let devices = self.state.read().await.ports.iter().map(|(id, _port)| {
            proto::Device {
                id: id.to_string(),
                health: "Healthy".to_string(),
                topology: None,
            }
        }).collect();
        Ok(tonic::Response::new(Box::pin(ListDevices {
            devices,
            sent: false
        })))
    }

    async fn allocate(
        &self,
        request: tonic::Request<proto::AllocateRequest>,
    ) -> Result<tonic::Response<proto::AllocateResponse>, tonic::Status> {
        let msg = request.into_inner();
        let resps = futures_util::stream::iter(msg.container_requests.into_iter()).then(|r| {
            async {
                let devices = futures_util::stream::iter(r.devices_i_ds.into_iter())
                    .enumerate().then(|(i, d)| {
                    async move {
                        let id = match uuid::Uuid::parse_str(&d) {
                            Ok(i) => i,
                            Err(e) => return Err(tonic::Status::invalid_argument(e.to_string()))
                        };
                        let ports = self.state.read().await;
                        let port = match ports.ports.get(&id) {
                            Some(p) => p,
                            None => return Err(tonic::Status::not_found("Unknown device ID"))
                        };
                        Ok(proto::DeviceSpec {
                            container_path: format!("/dev/ttyS{}", i),
                            host_path: port.path.to_string(),
                            permissions: "rw".to_string(),
                        })
                    }
                }).collect::<Vec<Result<_, _>>>().await.into_iter().collect::<Result<Vec<_>, _>>()?;

                Ok(proto::ContainerAllocateResponse {
                    envs: std::collections::HashMap::new(),
                    mounts: vec![],
                    annotations: std::collections::HashMap::new(),
                    devices,
                })
            }
        }).collect::<Vec<Result<_, _>>>().await.into_iter().collect::<Result<Vec<_>, tonic::Status>>()?;

        Ok(tonic::Response::new(proto::AllocateResponse {
            container_responses: resps
        }))
    }

    async fn pre_start_container(
        &self,
        _request: tonic::Request<proto::PreStartContainerRequest>,
    ) -> Result<tonic::Response<proto::PreStartContainerResponse>, tonic::Status> {
        Err(tonic::Status::unimplemented(""))
    }
}

#[tokio::main]
async fn main() {
    pretty_env_logger::init();

    let matches = clap::App::new("k8s-device-serial")
        .version("0.0.1")
        .about("K8S Device plugin for serial devices")
        .author("Q of AS207960 <q@as207960.net>")
        .arg(
            clap::Arg::with_name("conf")
                .short("c")
                .long("conf")
                .takes_value(true)
                .default_value("./conf.yaml")
                .help("Where to read config file from"),
        )
        .get_matches();

    let conf_file = std::fs::File::open(matches.value_of("conf").unwrap()).unwrap();
    let config: Config = serde_yaml::from_reader(conf_file).unwrap();

    let self_sock: &std::path::Path = std::path::Path::new("/var/lib/kubelet/device-plugins/as207960-serial.sock");

    let (mut register_send, register_recv) = tokio::sync::mpsc::channel(1);
    let should_exit = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (stops_send, mut stops_recv): (tokio::sync::mpsc::UnboundedSender<tokio::sync::oneshot::Sender<()>>, _) = tokio::sync::mpsc::unbounded_channel();
    let (request_stop_send, mut request_stop_recv) = tokio::sync::mpsc::unbounded_channel();
    let request_stop_send_ctrlc = request_stop_send.clone();
    let should_exit_ctrlc = should_exit.clone();
    ctrlc::set_handler(move || {
        should_exit_ctrlc.store(true, std::sync::atomic::Ordering::Relaxed);
        request_stop_send_ctrlc.send(()).unwrap();
    }).expect("Error setting Ctrl-C handler");

    tokio::spawn(async move {
        register_plugin(register_recv).await;
    });

    let mut check_interval = tokio::time::interval(tokio::time::Duration::from_secs(2));
    let app_state = std::sync::Arc::new(tokio::sync::RwLock::new(AppState::from(&config)));

    tokio::spawn(async move {
        loop {
            check_interval.tick().await;
            if !self_sock.exists() {
                info!("Socket deleted, recreating and registering...");
                request_stop_send.clone().send(()).unwrap();
            }
        }
    });
    tokio::spawn(async move {
        loop {
            request_stop_recv.recv().await;

            while let Some(stop_send) = stops_recv.recv().await {
                if let Err(_) = stop_send.send(()) {
                    info!("Already exiting...");
                }
            }
        }
    });

    loop {
        let mut listener = uds::DeleteOnDrop::bind(self_sock).unwrap();
        let _ = register_send.send(()).await;

        let svc = DevicePlugin {
            state: app_state.clone()
        };
        let (stop_send, stop_recv) = tokio::sync::oneshot::channel::<()>();
        stops_send.send(stop_send).unwrap();
        tonic::transport::Server::builder()
            .add_service(proto::device_plugin_server::DevicePluginServer::new(svc))
            .serve_with_incoming_shutdown(listener.tonic_incoming(), stop_recv.map(|_| ()))
            .await
            .unwrap();
        if should_exit.load(std::sync::atomic::Ordering::Relaxed) {
            break
        }
    }
}

async fn register_plugin(mut msg_chan: tokio::sync::mpsc::Receiver<()>) {
    while let Some(_) = msg_chan.recv().await {
        let sock = uds::UnixStream::connect("/var/lib/kubelet/device-plugins/kubelet.sock").await;
        let endpoint = tonic::transport::Endpoint::from_static("unix://null");
        let channel = match endpoint.connect_with_connector(sock).await {
            Ok(c) => c,
            Err(e) => {
                error!("Unable to connect to kubelet socket: {}", e);
                continue;
            }
        };
        let mut client = proto::registration_client::RegistrationClient::new(channel);
        match client.register(proto::RegisterRequest {
            version: "v1beta1".to_string(),
            endpoint: "as207960-serial.sock".to_string(),
            resource_name: "as207960.net/serial".to_string(),
            options: None,
        }).await {
            Ok(_) => {
                info!("Successfullyy registered with the kubelet")
            }
            Err(e) => {
                error!("Unable to connect register with kubelet: {}", e);
            }
        }
    }
}