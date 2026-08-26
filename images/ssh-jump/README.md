# Standard OpenSSH jump host image

This image does not implement SSH. It runs the distribution OpenSSH server and
uses `AuthorizedKeysCommand` to ask the control plane for an `authorized_keys`
line. The returned line combines `restrict`, `port-forwarding`, and one exact
`permitopen="workspace.<namespace>.svc.cluster.local:2222"` destination.

OpenSSH resolves users before invoking `AuthorizedKeysCommand`. The entrypoint
therefore derives the currently valid `access+<workspace-short-id>` account list
from the internal API and exposes it through the packaged `libnss-wrapper`.
This list contains no credentials; the per-connection API lookup remains the
authority and immediately rejects stopped, deleted, revoked, or cross-workspace
connections.
