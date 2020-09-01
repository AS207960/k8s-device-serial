# Kubernetes Device Plugin for serial ports
## (or infact any character device)

### Building

```bash
$ cargo build --release
```

### Config file format

```yaml
ports:
- /dev/ttyS0
```

Ports is the list of character devices you want available to pods.

### Usage

Run the executable with enough permissions to connect to Unix sockets, and create Unix
sockets in `/var/lib/kubelet/device-plugins/`.

`-c` on the command line specifies the location of the config file.

### Usage in K8S definitions

Set pod resources as follows (to request 2 serial ports);
```yaml
apiVersion: v1
kind: Pod
metadata:
  name: demo-pod
spec:
  containers:
    - name: demo-container-1
      image: k8s.gcr.io/pause:2.0
      resources:
        limits:
          as207960.net/serial: 2
```

This will create two devices named `/dev/ttyS0` and `/dev/ttyS1` from the pool of available 
resources.

### Why?!

I want to run a BBS in k8s, is that not reason enough?