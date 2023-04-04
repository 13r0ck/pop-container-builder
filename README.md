# Pop!_OS Container Builder


## How to use
```bash
sudo apt install -y \
       buildah
       chown \
       debootstrap \
       podman \
       rust-all
```

then
```
cargo run
```

After a minute or two `pop-container-runtime.tar` will appear in your current directory.
Import that into any container runtime of choice and use Pop!_OS in a container!
