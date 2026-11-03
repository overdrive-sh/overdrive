#!/usr/bin/env bash
# CONTRACT_SHAPE: bounded-change.
#
# E15 point-in-time capture. Invoked only by the expectation harness:
#   cargo xtask metal run -- verification/harness/run-expectation.sh E15
# E15 is the single sanctioned place that spawns the built `overdrive` binary
# (via the checked-in example); no test does. It links no overdrive-* crate,
# hand-programs no map, rule, Route, snapshot or identity, and appends to (never
# overwrites) every evidence file it writes.
set -euo pipefail

readonly gateway_address="${PIG_E15_GATEWAY_ADDRESS:-}"
readonly public_hostname="${PIG_E15_PUBLIC_HOSTNAME:-}"
readonly chain="${PIG_E15_CERTIFICATE_CHAIN:-}"
readonly key="${PIG_E15_PRIVATE_KEY:-}"

[[ -z "${PIG_E15_OVERDRIVE_BIN:-}" && -z "${PIG_E15_PUBLIC_CA:-}" ]] || {
  echo "E15: stale binary/public-CA inputs are forbidden; E15 builds clean HEAD and uses system trust" >&2
  exit 2
}
[[ -n "$gateway_address" && -n "$public_hostname" && -n "$chain" && -n "$key" ]] || {
  echo "E15 pending: set PIG_E15_GATEWAY_ADDRESS, PIG_E15_PUBLIC_HOSTNAME, PIG_E15_CERTIFICATE_CHAIN, and PIG_E15_PRIVATE_KEY" >&2
  exit 75
}
[[ -n "${REPO_ROOT:-}" && -n "${EVIDENCE_DIR:-}" ]] || {
  echo "E15: expectation harness did not supply REPO_ROOT/EVIDENCE_DIR" >&2
  exit 2
}

# Every observation of this run goes under its own never-reused directory; a
# second run (a retry, a rerun) can neither overwrite nor nest inside it.
RUN_ID="$(date -u +%Y%m%dT%H%M%S.%NZ)-$$"
readonly RUN_ID
readonly RUN_DIR="$EVIDENCE_DIR/runs/$RUN_ID"
install -d -m 0700 "$EVIDENCE_DIR/runs"
mkdir -m 0700 -- "$RUN_DIR" || {
  echo "E15: evidence directory for run $RUN_ID already exists; refusing to reuse it" >&2
  exit 2
}
printf 'run_id=%s phase=start timestamp=%s\n' "$RUN_ID" "$(date -u +%Y-%m-%dT%H:%M:%S.%NZ)" \
  >>"$RUN_DIR/run-record"
record_end() {
  printf 'run_id=%s phase=end exit=%s timestamp=%s\n' "$RUN_ID" "$1" \
    "$(date -u +%Y-%m-%dT%H:%M:%S.%NZ)" >>"$RUN_DIR/run-record"
}
trap 'record_end $?' EXIT

for named_path in "certificate-chain:$chain" "private-key:$key"; do
  path="${named_path#*:}"
  [[ "$path" = /* && -r "$path" && -f "$path" ]] || {
    echo "E15: supplied ${named_path%%:*} artifact is not an absolute readable regular file" >&2
    exit 2
  }
done
[[ "$(stat -c '%a' "$key")" == "600" ]] || {
  echo "E15: supplied private-key mode must be exactly 0600" >&2
  exit 2
}
for command in awk cargo cmp cp curl cut date git grep id install mktemp python3 rustc \
  sha256sum sleep ss stat systemd-detect-virt tail timeout uname; do
  command -v "$command" >/dev/null 2>&1 || {
    echo "E15: required external tool is unavailable: $command" >&2
    exit 2
  }
done

# --- native-metal substrate probes (fail closed) --------------------------------
[[ "$(uname -s)" == "Linux" && "$(uname -m)" == "x86_64" ]] || {
  echo "E15: substrate must be Linux x86_64" >&2
  exit 2
}
[[ "$(id -u)" -eq 0 ]] || {
  echo "E15: runner must execute as uid 0 (cargo xtask metal run --)" >&2
  exit 2
}
[[ -c /dev/kvm ]] || {
  echo "E15: /dev/kvm is not a character device" >&2
  exit 2
}
grep -Eqw 'vmx|svm' /proc/cpuinfo || {
  echo "E15: CPU virtualization extensions (vmx/svm) are absent" >&2
  exit 2
}
python3 - <<'PY' || { echo "E15: KVM API probe failed (open, KVM_GET_API_VERSION, KVM_CREATE_VM)" >&2; exit 2; }
import fcntl
import os

KVM_GET_API_VERSION = 0xAE00
KVM_CREATE_VM = 0xAE01
fd = os.open("/dev/kvm", os.O_RDWR | os.O_CLOEXEC)
try:
    assert fcntl.ioctl(fd, KVM_GET_API_VERSION) == 12
    vm = fcntl.ioctl(fd, KVM_CREATE_VM, 0)
    os.close(vm)
finally:
    os.close(fd)
PY
virt_result="$(systemd-detect-virt 2>/dev/null || true)"
[[ "$virt_result" == "none" ]] || {
  echo "E15: systemd-detect-virt reported '${virt_result:-<no result>}', not 'none' (virtualized or nested host)" >&2
  exit 2
}
[[ -e /sys/fs/cgroup/cgroup.controllers ]] || {
  echo "E15: host does not expose the required cgroup-v2 substrate" >&2
  exit 2
}
awk '$3 == "bpf" {found=1} END {exit found ? 0 : 1}' /proc/mounts || {
  echo "E15: host does not expose the required bpffs substrate" >&2
  exit 2
}

install -d -m 0700 "$RUN_DIR/observations/0001-platform"
{
  printf 'os=%s\n' "$(uname -s)"
  printf 'arch=%s\n' "$(uname -m)"
  printf 'kernel=%s\n' "$(uname -r)"
  printf 'uid=%s\n' "$(id -u)"
  printf 'kvm_char_device=true\n'
  printf 'systemd_detect_virt=%s\n' "$virt_result"
  printf 'cgroup_v2=true\n'
  printf 'bpffs=true\n'
} >>"$RUN_DIR/observations/0001-platform/substrate"

# --- harness-captured clean source -------------------------------------------
[[ "${EXPECTATION_SOURCE_DIRTY:-}" == "false" \
  && "${EXPECTATION_SOURCE_SHA:-}" =~ ^[0-9a-f]{40}$ ]] || {
  echo "E15: harness did not capture a clean valid source SHA" >&2
  exit 2
}
check_source_state() {
  local sha dirty
  sha="$(git -C "$REPO_ROOT" rev-parse HEAD)"
  dirty="$(git -C "$REPO_ROOT" status --porcelain --untracked-files=all -- \
    . ':(exclude)verification/expectations/**/evidence/**')"
  [[ "$sha" == "$EXPECTATION_SOURCE_SHA" && -z "$dirty" ]] || {
    echo "E15: source differs from the harness-captured clean HEAD ($1)" >&2
    exit 2
  }
}
check_source_state "before build"

# The example's scratch must share a filesystem with /srv/vm/overdrive-testing
# (reflinked rootfs) and be traversable by the unprivileged VMM uid.
run_root="$(mktemp -d /srv/vm/overdrive-testing/overdrive-e15.XXXXXX)"
readonly run_root
chmod 0711 "$run_root"
readonly build_root="$run_root/build"
readonly binary="$build_root/target/debug/overdrive"
readonly bpf_object="$REPO_ROOT/target/bpf/overdrive_bpf.o"
readonly provenance="$RUN_DIR/build-provenance"
readonly samples="$RUN_DIR/kernel-tls-samples"
sampler_pid=""
example_pid=""
cleanup() {
  local status=$?
  [[ -z "$sampler_pid" ]] || kill "$sampler_pid" 2>/dev/null || true
  if [[ -n "$example_pid" ]] && kill -0 "$example_pid" 2>/dev/null; then
    kill -TERM "$example_pid" 2>/dev/null || true
    wait "$example_pid" 2>/dev/null || true
  fi
  rm -rf -- "$run_root"
  record_end "$status"
  return "$status"
}
trap cleanup EXIT HUP INT TERM

install -d -m 0700 "$build_root" "$provenance" "$samples"
printf 'source_sha=%s\ndirty=false\n' "$EXPECTATION_SOURCE_SHA" >>"$provenance/source.txt"

# --- exact BPF producer -------------------------------------------------------
printf 'cargo xtask bpf-build\n' >>"$provenance/bpf-build-command.txt"
bpf_build_exit=0
(cd "$REPO_ROOT" && cargo xtask bpf-build) >>"$provenance/bpf-build.out" 2>&1 || bpf_build_exit=$?
printf 'exit=%s\n' "$bpf_build_exit" >>"$provenance/bpf-build.out"
[[ "$bpf_build_exit" -eq 0 && -f "$bpf_object" ]] || {
  echo "E15: clean-HEAD bpf-build failed" >&2
  exit 1
}
printf '%s\n' "$bpf_object" >>"$provenance/bpf-object.txt"
sha256sum "$bpf_object" >>"$provenance/overdrive_bpf.sha256"
recorded_bpf_digest="$(cut -d ' ' -f 1 "$provenance/overdrive_bpf.sha256" | tail -n 1)"
readonly recorded_bpf_digest
check_source_state "after bpf-build"
printf 'source_sha=%s\ndirty=false\n' "$EXPECTATION_SOURCE_SHA" \
  >>"$provenance/source-after-bpf-build.txt"

# --- default-feature product ---------------------------------------------------
# Omitting --features, --all-features and --no-default-features is load-bearing.
printf 'cargo build --locked --package overdrive-cli --bin overdrive --target-dir %s/target\n' \
  "$build_root" >>"$provenance/build-command.txt"
build_exit=0
cargo build --locked --package overdrive-cli --bin overdrive \
  --target-dir "$build_root/target" >>"$provenance/build.out" 2>&1 || build_exit=$?
printf 'exit=%s\n' "$build_exit" >>"$provenance/build.out"
[[ "$build_exit" -eq 0 && -f "$binary" && -x "$binary" ]] || {
  echo "E15: clean-HEAD default-feature product build failed" >&2
  exit 1
}
{ cargo -V; rustc -Vv; } >>"$provenance/toolchain.txt"
printf '%s\n' "$binary" >>"$provenance/artifact.txt"
sha256sum "$binary" >>"$provenance/overdrive.sha256"
recorded_digest="$(cut -d ' ' -f 1 "$provenance/overdrive.sha256" | tail -n 1)"
readonly recorded_digest
check_source_state "after product build"

python3 - "$bpf_object" "$binary" "$recorded_bpf_digest" \
  >>"$provenance/bpf-consumption.txt" <<'PY'
import hashlib
import pathlib
import sys

object_path = pathlib.Path(sys.argv[1])
binary_path = pathlib.Path(sys.argv[2])
object_bytes = object_path.read_bytes()
binary_bytes = binary_path.read_bytes()
digest = hashlib.sha256(object_bytes).hexdigest()
assert digest == sys.argv[3], "BPF object digest differs from the recorded digest"
offset = binary_bytes.find(object_bytes)
assert offset >= 0, "default-feature product does not embed the captured BPF object"
assert binary_bytes.find(object_bytes, offset + 1) < 0, "BPF object is embedded more than once"
print(f"object_path={object_path}")
print(f"object_sha256={digest}")
print(f"binary_path={binary_path}")
print("embedded_exactly_once=true")
print(f"embedded_offset={offset}")
PY

# Recompute both digests immediately before the example.
[[ "$(sha256sum "$binary" | cut -d ' ' -f 1)" == "$recorded_digest" ]] || {
  echo "E15: built product digest changed before execution" >&2
  exit 1
}
[[ "$(sha256sum "$bpf_object" | cut -d ' ' -f 1)" == "$recorded_bpf_digest" ]] || {
  echo "E15: BPF object digest changed before execution" >&2
  exit 1
}

# --- independent kernel-TLS oracle ------------------------------------------------
# The checked-in example owns the public request and takes no pause hook, so the
# runner samples from outside the product: a pre-run /proc/net/tls_stat, a
# timestamped poll of `ss -tni` and tls_stat for the whole life of the example
# (the guest streams /stream for about three seconds), and a post-run tls_stat.
# Samples are observations of a polled system, not a complete history.
sample_once() {
  printf '=== sample %s ===\n' "$(date -u +%Y-%m-%dT%H:%M:%S.%NZ)"
  printf '--- ss -tni ---\n'
  ss -tni 2>&1 || true
  printf '--- /proc/net/tls_stat ---\n'
  cat /proc/net/tls_stat 2>&1 || true
}
sample_once >>"$samples/pre-run.txt"
(
  while :; do
    sample_once >>"$samples/during-run.txt"
    sleep 0.1
  done
) &
sampler_pid=$!

example_exit=0
"$REPO_ROOT/examples/public-ingress-gateway/run-example.sh" \
  "$binary" "$run_root/product" "$gateway_address" \
  --public-hostname "$public_hostname" \
  --certificate-chain "$chain" \
  --private-key "$key" \
  >>"$RUN_DIR/product-run.out" 2>>"$RUN_DIR/product-run.err" &
example_pid=$!
wait "$example_pid" || example_exit=$?
example_pid=""
kill "$sampler_pid" 2>/dev/null || true
wait "$sampler_pid" 2>/dev/null || true
sampler_pid=""
sample_once >>"$samples/post-run.txt"
printf 'example_exit=%s\n' "$example_exit" >>"$RUN_DIR/product-run.exit"

# --- retain the example's own records verbatim --------------------------------------
if [[ -d "$run_root/product" ]]; then
  install -d -m 0700 "$RUN_DIR/product"
  for item in serve.log cleanup-deltas example-exit gateway-status.json route-deploy.out \
    service-deploy.out stream-headers.out stream-body.out stream-timing.out \
    stream-verbose.err exact-headers.out exact-body.out count-body.out; do
    if [[ -e "$run_root/product/$item" ]]; then
      cp -n -- "$run_root/product/$item" "$RUN_DIR/product/$item"
    fi
  done
  if [[ -d "$run_root/product/observations" ]]; then
    install -d -m 0700 "$RUN_DIR/product/observations"
    cp -R -n -- "$run_root/product/observations/." "$RUN_DIR/product/observations/"
  fi
fi

[[ "$example_exit" -eq 0 ]] || {
  echo "E15: checked-in example exited $example_exit; inspect captured product-run.err" >&2
  exit 1
}

# --- mechanical pre-checks (the verdict is a different reviewer's) --------------------
if grep -Eq 'health\.startup\.refused.*gateway\.(relay|connect)_probe|gateway\.(relay|connect)_probe.*health\.startup\.refused' \
  "$RUN_DIR/product/serve.log"; then
  echo "E15: serve refused startup on a gateway probe" >&2
  exit 1
fi
grep -Fq 'http_code=200 ssl_verify_result=0' "$RUN_DIR/product/stream-timing.out" || {
  echo "E15: /stream did not return 200 with a verified system-trust certificate" >&2
  exit 1
}
grep -Fq 'tcp-ulp-tls version: 1.3' "$samples/during-run.txt" || {
  echo "E15: no kernel-TLS 1.3 socket was observed while the example ran" >&2
  exit 1
}
grep -Eq 'public-ingress teardown deltas: vm=0 network=0 cgroup=0 run-directory=0 mount=0 loop=0 public-443-sockets=0 ktls-sockets=0' \
  "$RUN_DIR/product/cleanup-deltas" || {
  echo "E15: owned-resource deltas after cleanup are not all zero" >&2
  exit 1
}
