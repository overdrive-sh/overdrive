#!/usr/bin/env bash
# CONTRACT_SHAPE: bounded-change.
set -euo pipefail

readonly gateway_address="${PIG_E14_GATEWAY_ADDRESS:-}"
readonly public_hostname="${PIG_E14_PUBLIC_HOSTNAME:-}"
readonly chain="${PIG_E14_CERTIFICATE_CHAIN:-}"
readonly key="${PIG_E14_PRIVATE_KEY:-}"

[[ -z "${PIG_E14_OVERDRIVE_BIN:-}" && -z "${PIG_E14_PUBLIC_CA:-}" ]] || {
  echo "E14: stale binary/public-CA inputs are forbidden; E14 builds clean HEAD and uses system trust" >&2
  exit 2
}
[[ -n "$gateway_address" && -n "$public_hostname" && -n "$chain" && -n "$key" ]] || {
  echo "E14 pending: set PIG_E14_GATEWAY_ADDRESS, PIG_E14_PUBLIC_HOSTNAME, PIG_E14_CERTIFICATE_CHAIN, and PIG_E14_PRIVATE_KEY" >&2
  exit 75
}
[[ -n "${REPO_ROOT:-}" && -n "${EVIDENCE_DIR:-}" ]] || {
  echo "E14: expectation harness did not supply REPO_ROOT/EVIDENCE_DIR" >&2
  exit 2
}

for named_path in "certificate-chain:$chain" "private-key:$key"; do
  label="${named_path%%:*}"
  path="${named_path#*:}"
  [[ "$path" = /* && -r "$path" && -f "$path" ]] || {
    echo "E14: supplied $label artifact is not an absolute readable regular file" >&2
    exit 2
  }
done
for command in awk cargo cp curl cut date git grep hostname id install mktemp openssl python3 \
  rustc sha256sum sleep stat timeout uname; do
  command -v "$command" >/dev/null 2>&1 || {
    echo "E14: required external tool is unavailable: $command" >&2
    exit 2
  }
done
[[ "$(stat -c '%a' "$key")" == "600" ]] || {
  echo "E14: supplied private-key mode must be exactly 0600" >&2
  exit 2
}
[[ "$(uname -s)" == "Linux" && "$(id -u)" -eq 0 ]] || {
  echo "E14: public-ingress expectation requires real Linux and uid 0" >&2
  exit 2
}
readonly lima_marker="${LIMA_CIDATA_NAME:-${OVERDRIVE_LIMA_VM:-}}"
observed_hostname="$(hostname)"
readonly observed_hostname
[[ -n "$lima_marker" || "$observed_hostname" == lima-* ]] || {
  echo "E14: the required Lima execution marker is absent" >&2
  exit 2
}
[[ -e /sys/fs/cgroup/cgroup.controllers ]] || {
  echo "E14: host does not expose the required cgroup-v2 substrate" >&2
  exit 2
}
awk '$3 == "bpf" {found=1} END {exit found ? 0 : 1}' /proc/mounts || {
  echo "E14: host does not expose the required bpffs substrate" >&2
  exit 2
}

install -d -m 0700 "$EVIDENCE_DIR/observations/0001-platform"
{
  printf 'os=%s\n' "$(uname -s)"
  printf 'kernel=%s\n' "$(uname -r)"
  printf 'uid=%s\n' "$(id -u)"
  printf 'hostname=%s\n' "$observed_hostname"
  printf 'lima_marker=%s\n' "${lima_marker:-hostname-prefix}"
  printf 'cgroup_v2=true\n'
  printf 'bpffs=true\n'
} >"$EVIDENCE_DIR/observations/0001-platform/substrate"
printf 'verified=true\n' >"$EVIDENCE_DIR/observations/0001-platform/lima-verified"

[[ "${EXPECTATION_SOURCE_DIRTY:-}" == "false" \
  && "${EXPECTATION_SOURCE_SHA:-}" =~ ^[0-9a-f]{40}$ ]] || {
  echo "E14: harness did not capture a clean valid source SHA" >&2
  exit 2
}
observed_sha="$(git -C "$REPO_ROOT" rev-parse HEAD)"
readonly observed_sha
[[ "$observed_sha" == "$EXPECTATION_SOURCE_SHA" ]] || {
  echo "E14: checkout HEAD differs from harness-captured source" >&2
  exit 2
}
rechecked_dirty="$(
  git -C "$REPO_ROOT" status --porcelain --untracked-files=all -- \
    . ':(exclude)verification/expectations/**/evidence/**'
)"
readonly rechecked_dirty
[[ -z "$rechecked_dirty" ]] || {
  echo "E14: source checkout is dirty outside harness-owned evidence" >&2
  exit 2
}

run_root="$(mktemp -d "${TMPDIR:-/tmp}/overdrive-e14.XXXXXX")"
readonly run_root
readonly build_root="$run_root/build"
readonly binary="$build_root/target/debug/overdrive"
readonly bpf_object="$REPO_ROOT/target/bpf/overdrive_bpf.o"
readonly pre_request_gate="$run_root/pre-request"
example_pid=""
cleanup() {
  local status=$?
  rm -f -- "$pre_request_gate"
  if [[ -n "$example_pid" ]] && kill -0 "$example_pid" 2>/dev/null; then
    kill -TERM "$example_pid" 2>/dev/null || true
    wait "$example_pid" 2>/dev/null || true
  fi
  rm -rf -- "$run_root"
  return "$status"
}
trap cleanup EXIT HUP INT TERM

install -d -m 0700 "$build_root" "$EVIDENCE_DIR/build-provenance"
printf 'source_sha=%s\ndirty=false\n' "$EXPECTATION_SOURCE_SHA" \
  >"$EVIDENCE_DIR/build-provenance/source.txt"
printf 'cargo xtask bpf-build\n' \
  >"$EVIDENCE_DIR/build-provenance/bpf-build-command.txt"
bpf_build_exit=0
(
  cd "$REPO_ROOT"
  cargo xtask bpf-build
) >"$EVIDENCE_DIR/build-provenance/bpf-build.out" 2>&1 || bpf_build_exit=$?
printf 'exit=%s\n' "$bpf_build_exit" \
  >>"$EVIDENCE_DIR/build-provenance/bpf-build.out"
[[ "$bpf_build_exit" -eq 0 && -f "$bpf_object" ]] || {
  echo "E14: clean-HEAD cargo xtask bpf-build failed" >&2
  exit 1
}
printf '%s\n' "$bpf_object" >"$EVIDENCE_DIR/build-provenance/bpf-object.txt"
sha256sum "$bpf_object" >"$EVIDENCE_DIR/build-provenance/overdrive_bpf.sha256"
recorded_bpf_digest="$(cut -d ' ' -f 1 \
  "$EVIDENCE_DIR/build-provenance/overdrive_bpf.sha256")"
readonly recorded_bpf_digest

post_bpf_sha="$(git -C "$REPO_ROOT" rev-parse HEAD)"
post_bpf_dirty="$(
  git -C "$REPO_ROOT" status --porcelain --untracked-files=all -- \
    . ':(exclude)verification/expectations/**/evidence/**'
)"
readonly post_bpf_sha post_bpf_dirty
[[ "$post_bpf_sha" == "$EXPECTATION_SOURCE_SHA" && -z "$post_bpf_dirty" ]] || {
  echo "E14: source changed while producing the clean-HEAD BPF object" >&2
  exit 2
}
printf 'source_sha=%s\ndirty=false\n' "$post_bpf_sha" \
  >"$EVIDENCE_DIR/build-provenance/source-after-bpf-build.txt"

printf 'cargo build --locked --package overdrive-cli --bin overdrive --target-dir %s/target\n' \
  "$build_root" >"$EVIDENCE_DIR/build-provenance/build-command.txt"
build_exit=0
cargo build --locked --package overdrive-cli --bin overdrive \
  --target-dir "$build_root/target" \
  >"$EVIDENCE_DIR/build-provenance/build.out" 2>&1 || build_exit=$?
printf 'exit=%s\n' "$build_exit" >>"$EVIDENCE_DIR/build-provenance/build.out"
[[ "$build_exit" -eq 0 && -f "$binary" && -x "$binary" ]] || {
  echo "E14: clean-HEAD default-feature product build failed" >&2
  exit 1
}
{
  cargo -V
  rustc -Vv
} >"$EVIDENCE_DIR/build-provenance/toolchain.txt"
printf '%s\n' "$binary" >"$EVIDENCE_DIR/build-provenance/artifact.txt"
sha256sum "$binary" >"$EVIDENCE_DIR/build-provenance/overdrive.sha256"
recorded_digest="$(cut -d ' ' -f 1 "$EVIDENCE_DIR/build-provenance/overdrive.sha256")"
observed_digest="$(sha256sum "$binary" | cut -d ' ' -f 1)"
readonly recorded_digest observed_digest
[[ "$recorded_digest" == "$observed_digest" ]] || {
  echo "E14: built product digest changed before execution" >&2
  exit 1
}
observed_bpf_digest="$(sha256sum "$bpf_object" | cut -d ' ' -f 1)"
readonly observed_bpf_digest
[[ "$recorded_bpf_digest" == "$observed_bpf_digest" ]] || {
  echo "E14: BPF object changed between bpf-build and product build" >&2
  exit 1
}
python3 - "$bpf_object" "$binary" "$recorded_bpf_digest" \
  >"$EVIDENCE_DIR/build-provenance/bpf-consumption.txt" <<'PY'
import hashlib
import pathlib
import sys

object_path = pathlib.Path(sys.argv[1])
binary_path = pathlib.Path(sys.argv[2])
expected_digest = sys.argv[3]
object_bytes = object_path.read_bytes()
binary_bytes = binary_path.read_bytes()
digest = hashlib.sha256(object_bytes).hexdigest()
assert digest == expected_digest
offset = binary_bytes.find(object_bytes)
assert offset >= 0, "default-feature product does not embed the captured BPF object"
assert binary_bytes.find(object_bytes, offset + 1) < 0, "BPF object is embedded more than once"
print(f"object_path={object_path}")
print(f"object_sha256={digest}")
print(f"binary_path={binary_path}")
print(f"embedded_exactly_once=true")
print(f"embedded_offset={offset}")
PY

install -d -m 0700 "$EVIDENCE_DIR/observations/0002-public-trust"
openssl x509 -in "$chain" -noout -subject -issuer -dates \
  >"$EVIDENCE_DIR/observations/0002-public-trust/certificate"
openssl verify -show_chain -untrusted "$chain" "$chain" \
  >"$EVIDENCE_DIR/observations/0002-public-trust/verify.out" \
  2>"$EVIDENCE_DIR/observations/0002-public-trust/verify.err" || {
    echo "E14: supplied certificate chain is not trusted by the normal system trust store" >&2
    exit 1
  }

touch "$pre_request_gate"
env PIG_PRE_REQUEST_GATE="$pre_request_gate" \
  "$REPO_ROOT/examples/public-ingress-gateway/run-example.sh" \
  "$binary" "$run_root/product" "$gateway_address" \
  --public-hostname "$public_hostname" \
  --certificate-chain "$chain" \
  --private-key "$key" \
  >"$EVIDENCE_DIR/product-run.out" 2>"$EVIDENCE_DIR/product-run.err" &
example_pid=$!

deadline=$((SECONDS + 90))
while [[ ! -e "$pre_request_gate.ready" && "$SECONDS" -lt "$deadline" ]]; do
  kill -0 "$example_pid" 2>/dev/null || {
    echo "E14: checked-in product example exited; inspect captured product-run.err" >&2
    exit 1
  }
  sleep 0.1
done
[[ -e "$pre_request_gate.ready" ]] || {
  echo "E14: checked-in example did not reach the public-request boundary" >&2
  exit 1
}

request_started="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
request_exit=0
timeout 15s curl --silent --show-error --fail-with-body \
  --tlsv1.3 --tls-max 1.3 --http1.1 \
  --resolve "$public_hostname:443:$gateway_address" \
  --dump-header "$EVIDENCE_DIR/public-headers.out" \
  --output "$EVIDENCE_DIR/public-body.out" \
  --write-out '%{http_code} %{ssl_verify_result}\n' \
  >"$EVIDENCE_DIR/public-status.out" \
  "https://$public_hostname/stream" || request_exit=$?
request_ended="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
install -d -m 0700 "$EVIDENCE_DIR/observations/0003-public-request"
{
  printf 'command=curl --resolve <public-hostname>:443:<captured-gateway-ipv4> https://<public-hostname>/stream\n'
  printf 'status=%s/%s\n' "$request_exit" "$(<"$EVIDENCE_DIR/public-status.out")"
  printf 'started_at=%s\n' "$request_started"
  printf 'ended_at=%s\n' "$request_ended"
} >"$EVIDENCE_DIR/observations/0003-public-request/metadata"
[[ "$request_exit" -eq 0 ]] || {
  echo "E14: normal-system-trust public request failed" >&2
  exit 1
}
grep -Fxq '200 0' "$EVIDENCE_DIR/public-status.out"

rm -f -- "$pre_request_gate"
wait "$example_pid"
example_pid=""

python3 - "$run_root/product/gateway-status.json" "$chain" "$key" "$public_hostname" \
  "$gateway_address:443" <<'PY'
import json
import pathlib
import sys

raw = pathlib.Path(sys.argv[1]).read_text()
body = json.loads(raw)
key_marker = next(
    line[:16] for line in pathlib.Path(sys.argv[3]).read_text().splitlines()
    if not line.startswith("---") and len(line) >= 16
)
assert body["enabled"] is True
assert body["configured_address"] == sys.argv[5]
assert body["listener_bound"] is True
assert body["application"]["current"] is not None
assert body["application"]["listener"]["state"] == "bound"
assert body["application"]["connect_path"] == "ready"
assert body["application"]["gateway_identity"]["state"] == "current"
gateway_spiffe = body["application"]["gateway_identity"]["spiffe_id"]
assert gateway_spiffe.startswith("spiffe://overdrive.local/gateway/")
assert "/alloc/" not in gateway_spiffe
assert body["certified_key"]["state"]["state"] == "usable"
assert body["certified_key"]["state"]["hostname"] == sys.argv[4]
assert isinstance(body["hydration"], list) and body["hydration"]
for forbidden in (
    sys.argv[2].lower(), sys.argv[3].lower(), "certificate_chain_path",
    "private_key_path", "certificate_chain_der", "pem", "private_key",
    "ciphertext", "ciphertext_and_tag", "nonce", "salt",
    "begin certificate", "begin private key",
    key_marker.lower(),
):
    assert forbidden not in raw.lower()
PY

cp "$run_root/product/service-deploy.out" "$EVIDENCE_DIR/service-deploy.out"
cp "$run_root/product/route-deploy.out" "$EVIDENCE_DIR/route-deploy.out"
cp "$run_root/product/gateway-status.json" "$EVIDENCE_DIR/gateway-status.json"
