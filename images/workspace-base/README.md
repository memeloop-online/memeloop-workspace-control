# Standard workspace image

This image implements Image Contract v1. The control plane invokes
`/usr/local/bin/mwc-workspace-bootstrap prepare` from an init container and `serve` from the main
container. It runs stock OpenSSH on port 2222, accepts the platform-managed stable host identity
from a Kubernetes Secret, and installs the validated injection manifest without logging injected
values. The encrypted private host identity remains authoritative in the control-plane database;
the workspace PVC only stores user data and the derived `authorized_keys` file.

Third-party images must provide the same executable contract and the commands used by the image
(`ssh-keygen`, `jq`, `install`, and `sshd`).
