#!/bin/bash
# Runs only inside the disposable TCG guest, never on a Kubernetes node.
set -euo pipefail
exec > /dev/ttyS0 2>&1
inputs=/mnt/inputs
out=/var/lib/mwc-full-output
mkdir -p "$out/artifacts" "$out/bin" "$out/fixtures"
finish() {
 status=$?
 trap - EXIT
 # Receipts and logs cross the serial channel; never copy guest disks or keys.
 tar -C "$out/artifacts" -czf "$out/evidence.tgz" .
 if [ "$(stat -c %s "$out/evidence.tgz")" -le 8388608 ]; then
  printf '\nMWC_FULL_ARTIFACT_BEGIN\n'
  base64 -w 76 "$out/evidence.tgz"
  printf '\nMWC_FULL_ARTIFACT_END\n'
 else status=1; fi
 if [ "$status" = 0 ]; then accepted=true; else accepted=false; fi
 printf '\nMWC_VM_FULL_RECEIPT {"accepted":%s,"exit_status":%s,"actual_guest_boot":true,"full_harness_tested":true,"production_deployed":false}\n' "$accepted" "$status"
 sync
 /sbin/poweroff
}
trap finish EXIT
uname -a
findmnt /
findmnt "$inputs"
(cd "$inputs" && sha256sum -c SHA256SUMS)
# All packages and their dependency closure were downloaded from the pinned
# Debian snapshot by CI. No guest package manager can reach an external mirror.
install -m 0755 /dev/stdin /usr/sbin/policy-rc.d <<'POLICY'
#!/bin/sh
exit 101
POLICY
cp -a "$inputs/apt-lists/." /var/lib/apt/lists/
rm -f /etc/apt/sources.list /etc/apt/sources.list.d/*.sources
printf '%s\n' 'deb [check-valid-until=no] https://snapshot-cloudflare.debian.org/archive/debian/20261026T000000Z bookworm main' > /etc/apt/sources.list
cp "$inputs/debs/"*.deb /var/cache/apt/archives/
apt-get --assume-yes --no-download --no-install-recommends install \
 lvm2 thin-provisioning-tools xfsprogs jq openssh-client iproute2 iptables \
 conntrack socat ethtool ebtables kmod ca-certificates curl
rm /usr/sbin/policy-rc.d
rm -f /var/cache/apt/archives/*.deb
cp "$inputs/compiled/source-shas.json" "$out/artifacts/compiled-source-shas.json"
cp "$inputs/compiled/SHA256SUMS" "$out/artifacts/compiled-SHA256SUMS"
cp "$inputs/compiled/artifact-identity.json" "$out/artifacts/compiled-artifact-identity.json"
cp "$inputs/compiled/bin/"* "$out/bin/"
chmod 0755 "$out/bin/"*
cp "$inputs/compiled/fixtures/"*.json "$out/fixtures/"
export GITHUB_ACTIONS=true GITHUB_WORKSPACE="$out" RUNNER_TEMP="$out" MWC_PRODUCT_STORAGE_INPUTS="$inputs"
mv "$out/bin" "$out/product-storage-bin"
mv "$out/fixtures" "$out/product-storage-fixtures"
PRODUCT_RENDERER_SHA="$(jq -er .renderer_sha "$inputs/compiled/source-shas.json")"
WRAPPER_SHA="$(jq -er .wrapper_sha "$inputs/compiled/source-shas.json")"
export PRODUCT_RENDERER_SHA WRAPPER_SHA
bash "$inputs/runtime_product_storage.sh"
jq -e '.accepted==true and .real_csi==true and .production_deployed==false' "$out/artifacts/acceptance.json"
