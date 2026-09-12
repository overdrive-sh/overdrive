#!/usr/bin/env bash
# Observation-only invocation of checked-in product examples. Run ONLY through
# cargo xtask metal run, whose lease and fail-closed preflight precede this file.
set -euo pipefail
[[ $(uname -m) == x86_64 && $(id -u) == 0 ]] || exit 2
capture=$(mktemp -d /tmp/issue283-native.XXXXXX)
chmod 0755 "$capture"
printf 'native_capture=%s\n' "$capture"
exec > >(tee "$capture/stdout") 2> >(tee "$capture/stderr" >&2)
trap 'status=$?; printf "finished=%s exit=%s\n" "$(date -u +%FT%TZ)" "$status" >>"$capture/status"' EXIT
date -u +%FT%TZ
uname -a
cat .overdrive-metal-source
cat /run/lock/overdrive-metal-shared.owner
rustc -Vv
cargo -V
cloud-hypervisor --version
sha256sum /srv/vm/overdrive-testing/kernel /srv/vm/overdrive-testing/rootfs.ext4
debugfs -R 'ls -l /' /srv/vm/overdrive-testing/rootfs.ext4
debugfs -R 'ls -l /sbin' /srv/vm/overdrive-testing/rootfs.ext4
debugfs -R "dump /init $capture/guest-init" /srv/vm/overdrive-testing/rootfs.ext4
debugfs -R "dump /sbin/init $capture/guest-sbin-init" /srv/vm/overdrive-testing/rootfs.ext4
sha256sum "$capture/guest-sbin-init"
cmp "$capture/guest-init" "$capture/guest-sbin-init"
if [[ -f "$capture/guest-init" ]]; then
  sha256sum "$capture/guest-init"
  file "$capture/guest-init"
  nm -C "$capture/guest-init" >"$capture/guest-symbols"
  objdump -Cd "$capture/guest-init" >"$capture/guest-disassembly"
fi
[[ "${1:-trials}" != inventory ]] || exit 0
if [[ "${1:-trials}" == finalize ]]; then
  timeout 600s cargo build -p overdrive-cli --bin overdrive
  sha256sum target/debug/overdrive target/x86_64-unknown-linux-musl/release/overdrive-init
  cmp "$capture/guest-sbin-init" target/x86_64-unknown-linux-musl/release/overdrive-init
  [[ ! -e /srv/vm/overdrive-testing/svm-e08 ]]
  for alloc in alloc-service-vm-e08-0 alloc-service-vm-e08-client-0 alloc-service-vm-tcp-failure-0; do
    [[ ! -e "/run/overdrive/vm/$alloc" ]]
    [[ ! -e "/sys/fs/cgroup/overdrive.slice/workloads.slice/$alloc.scope" ]]
  done
  exit 0
fi
command -v strace
timeout 600s env OVERDRIVE_BPF_NATIVE=1 cargo xtask bpf-build
timeout 600s cargo build -p overdrive-cli --bin overdrive
sha256sum target/debug/overdrive target/bpf/overdrive_bpf.o
trials=(healthy failure)
[[ "${1:-trials}" != untraced ]] || trials=(healthy)
for trial in "${trials[@]}"; do
  dir="$capture/$trial"
  mkdir "$dir"
  spec=service.toml
  workload=service-vm-e08
  expected=stable
  args=(client-healthy.toml service-vm-e08-client)
  if [[ "$trial" == failure ]]; then
    spec=tcp-startup-failure.toml
    workload=service-vm-tcp-failure
    expected=startup-failed
    args=()
  fi
  printf 'trial=%s start=%s\n' "$trial" "$(date -u +%FT%TZ)"
  command_prefix=(strace -ff -ttt -s 256 -e trace=%process -o "$dir/process")
  observer=''
  if [[ "${1:-trials}" == untraced ]]; then
    command_prefix=()
    bash docs/analysis/observe-vm-console-283.sh "$dir" &
    observer=$!
  fi
  set +e
  env RUST_LOG=info SVM_E08_SKIP_BUILD=1 \
    SVM_E08_CASE_CAPTURE_DIR="$dir/logs" SVM_E08_OWNERSHIP_TOKEN="issue283-$trial-$$" \
    "${command_prefix[@]}" \
    bash examples/service-kind-vm-workloads/run-example.sh run case \
    "$spec" "$workload" "$expected" "${args[@]}" \
    >"$dir/stdout" 2>"$dir/stderr"
  status=$?
  set -e
  if [[ -n "$observer" ]]; then
    touch "$dir/observer-done"
    wait "$observer"
  fi
  printf 'trial=%s end=%s exit=%s\n' "$trial" "$(date -u +%FT%TZ)" "$status" | tee "$dir/status"
  cat "$dir/stdout"
  cat "$dir/stderr" >&2
  [[ "$status" == 0 ]] || exit "$status"
done
if [[ "${1:-trials}" == untraced ]]; then
  # Build a comparison artifact only; do not install it into any guest image.
  timeout 600s cargo build -p overdrive-init --release --target x86_64-unknown-linux-musl
  sha256sum target/x86_64-unknown-linux-musl/release/overdrive-init
  cmp "$capture/guest-init" target/x86_64-unknown-linux-musl/release/overdrive-init || true
fi
