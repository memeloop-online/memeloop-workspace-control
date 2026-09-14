# Kali browser desktop workspace image

This image implements the workspace Image Contract v1 and adds an optional
browser desktop. It starts SSH on `2222`, TigerVNC on Pod loopback
`127.0.0.1:5901`, and noVNC/websockify on Pod loopback `127.0.0.1:6080`.
The controller publishes only the noVNC port through its authenticated HTTPS
desktop mapping.

Use it only with a template desktop endpoint on port `6080`:

```yaml
image: ghcr.io/OWNER/REPOSITORY-kali-browser-desktop@sha256:REPLACE_WITH_CI_DIGEST
desktop:
  internal_port: 6080
  display_name: Kali browser desktop
runtime_class_name: gvisor
```

The image contains Kali rolling, XFCE, TigerVNC, noVNC and websockify. Add the
assessment tools required by each team in a derived image. Tools that use raw
sockets, kernel modules, host networking or elevated Linux capabilities need a
template and runtime designed for those capabilities.

On gVisor, the launcher keeps image decoding inside the workspace's runsc
sandbox instead of asking Glycin to create an unsupported nested bubblewrap
network namespace. On ordinary runtimes, Glycin continues to use bubblewrap.

Build from the repository root, because the Image Contract bootstrap is shared
with the standard workspace image:

```sh
docker build -f images/kali-browser-desktop/Dockerfile -t mwc-kali-browser-desktop .
```
