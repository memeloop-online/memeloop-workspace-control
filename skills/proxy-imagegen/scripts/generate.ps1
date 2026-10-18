#!/usr/bin/env bash
# Keep previously injected command paths usable; this file is a Bash shim.
set -euo pipefail
exec "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/generate.sh" "$@"
