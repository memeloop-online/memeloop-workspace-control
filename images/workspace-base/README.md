# Standard workspace image

This image implements Image Contract v1. The control plane initializes the scratch directories
and, when enabled, runs the BuildKit bootstrap before starting the main container. The main
container invokes `/etc/workspace-platform/mwc-workspace-bootstrap serve`, which prepares the
layout and workspace dependencies before starting stock OpenSSH on port 2222. It accepts the
platform-managed stable host identity from a Kubernetes Secret, and installs the validated injection
manifest without logging injected values. The encrypted private host identity remains authoritative
in the control-plane database;
the workspace PVC only stores user data and the derived `authorized_keys` file.

Third-party images must provide the same executable contract and the commands used by the image
(`ssh-keygen`, `jq`, `install`, and `sshd`).
