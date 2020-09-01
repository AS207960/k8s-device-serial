pub struct DeleteOnDrop {
    path: std::path::PathBuf,
    listener: tokio::net::UnixListener,
}

pub struct DeleteOnDropIncoming<'a> {
    incoming: tokio::net::unix::Incoming<'a>
}

pub struct DeleteOnDropStream {
    stream: tokio::net::UnixStream
}

impl DeleteOnDrop {
    pub fn bind(path: impl AsRef<std::path::Path>) -> std::io::Result<Self> {
        let path = path.as_ref().to_owned();
        tokio::net::UnixListener::bind(&path).map(|listener| DeleteOnDrop { path, listener })
    }

    pub fn tonic_incoming(&mut self) -> DeleteOnDropIncoming<'_> {
        DeleteOnDropIncoming {
            incoming: self.incoming()
        }
    }
}

impl Drop for DeleteOnDrop {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

impl std::ops::Deref for DeleteOnDrop {
    type Target = tokio::net::UnixListener;

    fn deref(&self) -> &Self::Target {
        &self.listener
    }
}

impl std::ops::DerefMut for DeleteOnDrop {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.listener
    }
}

impl tokio::stream::Stream for DeleteOnDropIncoming<'_> {
    type Item = std::io::Result<DeleteOnDropStream>;

    fn poll_next(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Option<Self::Item>> {
        let socket = futures::ready!(tokio::net::unix::Incoming::poll_accept(std::pin::Pin::new(&mut self.incoming), cx))?;
        std::task::Poll::Ready(Some(Ok(DeleteOnDropStream {
            stream: socket
        })))
    }
}

impl tonic::transport::server::Connected for DeleteOnDropStream {}

impl tokio::io::AsyncWrite for DeleteOnDropStream {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        tokio::net::UnixStream::poll_write(std::pin::Pin::new(&mut self.get_mut().stream), cx, buf)
    }

    fn poll_flush(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<std::io::Result<()>> {
        tokio::net::UnixStream::poll_flush(std::pin::Pin::new(&mut self.get_mut().stream), cx)
    }

    fn poll_shutdown(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<std::io::Result<()>> {
        tokio::net::UnixStream::poll_shutdown(std::pin::Pin::new(&mut self.get_mut().stream), cx)
    }
}

impl tokio::io::AsyncRead for DeleteOnDropStream {
    unsafe fn prepare_uninitialized_buffer(&self, buf: &mut [core::mem::MaybeUninit<u8>]) -> bool {
         tokio::net::UnixStream::prepare_uninitialized_buffer(&self.stream, buf)
    }

    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        tokio::net::UnixStream::poll_read(std::pin::Pin::new(&mut self.get_mut().stream), cx, buf)
    }
}

pub struct UnixStream {
    path: std::path::PathBuf,
}

impl UnixStream {
    pub async fn connect<P>(path: P) -> Self
        where
            P: AsRef<std::path::Path>,
    {
        Self { path: path.as_ref().to_path_buf() }
    }
}

impl tower_service::Service<tonic::transport::Uri> for UnixStream {
    type Response = tokio::net::UnixStream;
    type Error = std::io::Error;
    type Future = futures_util::future::BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut std::task::Context<'_>) -> std::task::Poll<Result<(), Self::Error>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn call(&mut self, _req: tonic::transport::Uri) -> Self::Future {
        let path = self.path.clone();
        let fut = async move {
            tokio::net::UnixStream::connect(path).await
        };

        Box::pin(fut)
    }
}