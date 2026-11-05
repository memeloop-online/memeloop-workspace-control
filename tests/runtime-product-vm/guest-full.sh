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
 budget=false
 if [ -f "$out/artifacts/acceptance.json" ] && jq -e '.accepted==true and .full_budget_goal_complete==true and (.cases|length)==4' "$out/artifacts/acceptance.json" >/dev/null; then budget=true; fi
 printf '\nMWC_VM_FULL_RECEIPT {"accepted":%s,"exit_status":%s,"actual_guest_boot":true,"full_harness_tested":true,"full_budget_goal_complete":%s,"production_deployed":false}\n' "$accepted" "$status" "$budget"
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
 conntrack socat ethtool ebtables kmod ca-certificates curl bzip2
rm /usr/sbin/policy-rc.d
rm -f /var/cache/apt/archives/*.deb
cp "$inputs/compiled/source-shas.json" "$out/artifacts/compiled-source-shas.json"
cp "$inputs/compiled/SHA256SUMS" "$out/artifacts/compiled-SHA256SUMS"
cp "$inputs/compiled/artifact-identity.json" "$out/artifacts/compiled-artifact-identity.json"
cp -a "$inputs/compiled/wrapper-release" "$out/artifacts/"
(cd "$inputs/compiled" && sha256sum -c SHA256SUMS)
(cd "$inputs/compiled/bin" && sha256sum -c ../wrapper-release/SHA256SUMS)
printf '%s  %s\n' 79f8b2f0ba17f6b2df647908af438fe939602fce5bd86cd1a44f1927ed32e852 "$inputs/compiled/bin/mwc-runtime-wrapper" | sha256sum -c -
printf '%s  %s\n' bc1daf24021f6533513fbc2dfd0c691739fde2b286abcf432cf9c545bdd5d131 "$inputs/compiled/bin/mwc-runsc-wrapper" | sha256sum -c -
jq -e '.source_sha=="18b9e8ad3c461554d84746fccea4240597114107" and .run_id=="41770885611" and .run_attempt=="1" and .platform=="linux/amd64" and .variants==["native","runsc"]' "$inputs/compiled/wrapper-release/artifact-identity.json"
jq -e '.renderer_sha==.harness_sha and .wrapper_sha=="18b9e8ad3c461554d84746fccea4240597114107" and .wrapper_artifact=={run_id:"41770885611",run_attempt:"1",artifact_id:13294234740,artifact_zip_sha256:"067ee972868e0e95a33574271636694b1d2939c71e9bd62c7aae91f52d67ee8d"}' "$inputs/compiled/source-shas.json"
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
jq -e '.accepted==true and .real_csi==true and .full_budget_goal_complete==true and (.cases|length)==4 and .production_deployed==false' "$out/artifacts/acceptance.json"
