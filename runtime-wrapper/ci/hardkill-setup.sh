#!/bin/bash
# CI-only runtime entries. No production wrapper options or code hooks.
cri() { crictl --timeout 60s --runtime-endpoint "unix://$base/containerd.sock" --image-endpoint "unix://$base/containerd.sock" "$@"; }
for stage in prepared redirected runtime-created; do
 entry="$base/hardkill-$stage"
 if [ "$mode" = runsc ]; then binary="$base/mwc-runsc-wrapper"; else binary="$base/mwc-runtime-wrapper"; fi
 cat > "$entry" <<SCRIPT
#!/bin/sh
exec /usr/bin/python3 '$GITHUB_WORKSPACE/runtime-wrapper/ci/hardkill-observer.py' '$stage' '$binary' "\$@"
SCRIPT
 chmod 0755 "$entry"
 if [ "$mode" = runsc ]; then
  sed "s|binary_name = .*|binary_name = \"$entry\"|" "$base/runsc.toml" > "$base/hardkill-$stage.toml"
  cat >> "$base/containerd.toml" <<TOML
    [plugins.'io.containerd.cri.v1.runtime'.containerd.runtimes.'mwc-kill-$stage']
      runtime_type = 'io.containerd.runsc.v1'
      [plugins.'io.containerd.cri.v1.runtime'.containerd.runtimes.'mwc-kill-$stage'.options]
        ConfigPath = '$base/hardkill-$stage.toml'
TOML
 else
  cat >> "$base/containerd.toml" <<TOML
    [plugins.'io.containerd.cri.v1.runtime'.containerd.runtimes.'mwc-kill-$stage']
      runtime_type = 'io.containerd.runc.v2'
      [plugins.'io.containerd.cri.v1.runtime'.containerd.runtimes.'mwc-kill-$stage'.options]
        BinaryName = '$entry'
        SystemdCgroup = false
TOML
 fi
done
