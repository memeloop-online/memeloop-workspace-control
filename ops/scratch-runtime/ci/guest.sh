#!/usr/bin/env bash
set -euo pipefail
bundle=/run/k3s/containerd/io.containerd.runtime.v2.task/k8s.io
logs=/var/log/pods
case $1 in
  install)
    src=/home/ubuntu/mwc-ci
    install -m 0755 "$src/k3s" /usr/local/bin/k3s
    install -Dm 0755 "$src/mwc-scratch-mounts.sh" /usr/local/libexec/mwc-scratch-mounts.sh
    install -Dm 0644 "$src/mwc-scratch-mounts.service" /etc/systemd/system/mwc-scratch-mounts.service
    install -Dm 0644 "$src/30-mwc-scratch-mounts.conf" /etc/systemd/system/k3s.service.d/30-mwc-scratch-mounts.conf
    install -d -m 0750 "$logs"
    stat -c '%d:%i' "$logs" > /var/lib/mwc-ci-original-log-inode
    cat > /etc/systemd/system/k3s.service <<'UNIT'
[Unit]
Wants=network-online.target
After=network-online.target

[Service]
Type=notify
KillMode=process
Delegate=yes
ExecStart=/usr/local/bin/k3s server --prefer-bundled-bin --disable=traefik,servicelb,metrics-server,local-storage,coredns

[Install]
WantedBy=multi-user.target
UNIT
    systemctl daemon-reload
    systemctl enable --now k3s.service
    exit
    ;;
  rollback)
    k3s kubectl delete pod ordinary-before-reboot --ignore-not-found --timeout=60s
    rm -f /etc/systemd/system/k3s.service.d/30-mwc-scratch-mounts.conf
    exit
    ;;
  mounted|unmounted) ;;
  *) exit 2 ;;
esac

k() { k3s kubectl "$@"; }
for _ in {1..75}; do
  if k get node --request-timeout=2s >/dev/null 2>&1; then break; fi
  sleep 1
done
k wait --for=condition=Ready nodes --all --timeout=90s >&2
# Kubelet owns the directory contents and may collect loose entries at boot.
# Preserve the real directory inode and prove an ordinary Pod can log to it.
test "$(stat -c '%d:%i' "$logs")" = "$(cat /var/lib/mwc-ci-original-log-inode)"
if [[ $1 == mounted ]]; then
  test "$(findmnt -n -M "$bundle" -o PROPAGATION)" = private
  test "$(findmnt -n -M "$logs" -o TARGET)" = "$logs"
  active=$(systemctl show -p ActiveEnterTimestampMonotonic --value mwc-scratch-mounts.service)
  started=$(systemctl show -p ExecMainStartTimestampMonotonic --value k3s.service)
  ((active > 0 && active < started))
else
  if mountpoint -q "$bundle" || mountpoint -q "$logs" || systemctl is-active --quiet mwc-scratch-mounts.service; then
    echo 'The cold rollback retained an MWC mount or active boot unit' >&2
    exit 1
  fi
fi

pod=$2
prior_container=${3:-}
image=docker.io/library/alpine@sha256:4bcff63911fcb4448bd4fdacec207030997caf25e9bea4045fa6c8c44de311d1
if ! k get pod "$pod" >/dev/null 2>&1; then
  k wait --for=create serviceaccount/default --timeout=45s >&2
  k run "$pod" --restart=Always --image="$image" --overrides='{"spec":{"automountServiceAccountToken":false}}' \
    --command -- sh -ec 'echo ordinary-container-log; exec sleep 3600' >&2
fi
k wait --for=condition=Ready "pod/$pod" --timeout=100s >&2
# The API can retain pre-reboot Pod status after the kubelet proxy comes back.
# Prove a newly running container and its real logs and exec, not stale Ready.
candidate_ready=false
for _ in {1..60}; do
  container=$(k --request-timeout=3s get pod "$pod" -o jsonpath='{.status.containerStatuses[?(@.ready==true)].containerID}') || container=
  if [[ -n $container && $container != "$prior_container" ]] && \
     logs_output=$(k --request-timeout=5s logs "$pod" 2>/dev/null) && [[ $logs_output == ordinary-container-log ]] && \
     exec_output=$(timeout 7 k3s kubectl --request-timeout=5s exec "$pod" -- printf ordinary-exec 2>/dev/null) && \
     [[ $exec_output == ordinary-exec ]]; then
    candidate_ready=true
    break
  fi
  sleep 1
done
if [[ $candidate_ready != true ]]; then
  k get pod "$pod" -o wide >&2 || true
  k logs "$pod" >&2 || true
  journalctl -u k3s.service -n 25 --no-pager >&2 || true
  exit 1
fi
python3 - "$1" "$pod" "$(k get pod "$pod" -o jsonpath='{.metadata.uid}')" "$container" <<'PY'
import json, pathlib, sys
print(json.dumps({'boot_id':pathlib.Path('/proc/sys/kernel/random/boot_id').read_text().strip(),
                  'mount_state':sys.argv[1],'pod':sys.argv[2],'pod_uid':sys.argv[3],'container_id':sys.argv[4],
                  'original_log_inode_retained':True,'ordinary_log_and_exec':True}))
PY
