#!/usr/bin/env bash
# AT-PIG-E2E-1 — recurring built-product public-ingress composition.
# CONTRACT_SHAPE: bounded-change.
set -euo pipefail

[[ "$#" -eq 2 ]] || {
  echo "usage: $0 <absolute-overdrive-binary> <absolute-scratch-dir>" >&2
  exit 2
}

readonly overdrive_bin="$1"
readonly scratch_dir="$2"
[[ "$overdrive_bin" = /* && "$scratch_dir" = /* ]] || {
  echo "AT-PIG-E2E-1: both arguments must be absolute" >&2
  exit 2
}
[[ -x "$overdrive_bin" ]] || {
  echo "AT-PIG-E2E-1: binary is not executable: $overdrive_bin" >&2
  exit 2
}
[[ "$(uname -s)" == "Linux" && "$(id -u)" -eq 0 ]] || {
  echo "AT-PIG-E2E-1: run on Linux as root through the Tier-3 harness" >&2
  exit 2
}
for command in awk bpftool cmp curl date diff find grep install ip nft openssl python3 \
  setsid sort ss stat strace systemd-run tail tcpdump timeout tr wc; do
  command -v "$command" >/dev/null 2>&1 || {
    echo "AT-PIG-E2E-1: required external tool is unavailable: $command" >&2
    exit 2
  }
done
[[ -e /sys/fs/cgroup/cgroup.controllers ]] || {
  echo "AT-PIG-E2E-1: required cgroup-v2 substrate is unavailable" >&2
  exit 2
}
awk '$3 == "bpf" {found=1} END {exit found ? 0 : 1}' /proc/mounts || {
  echo "AT-PIG-E2E-1: required bpffs substrate is unavailable" >&2
  exit 2
}

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
readonly repo_root
readonly example="$repo_root/examples/public-ingress-gateway/run-example.sh"
# shellcheck source=/dev/null
source "$repo_root/tests/tier3/public-ingress-gateway/cleartext-guard.sh"
readonly pre_gate="$scratch_dir.pre-request"
readonly observation_gate="$scratch_dir.observe"
readonly baseline="$scratch_dir.baseline"
readonly post="$scratch_dir.post"
readonly public_pcap="$scratch_dir.public.pcap"
readonly upstream_pcap="$scratch_dir.upstream.pcap"
readonly example_log="$scratch_dir.example.log"
readonly connector_trace="$scratch_dir.connector-trace"
readonly map_trace="$scratch_dir.map-trace"

example_pid=""
public_capture_pid=""
upstream_capture_pid=""
control_server_pid=""

capture_probe() {
  local directory="$1"
  local label="$2"
  shift 2
  local started ended status
  install -d -m 0700 "$directory"
  started="$(date -u +%Y-%m-%dT%H:%M:%S.%NZ)"
  status=0
  "$@" >"$directory/$label.out" 2>"$directory/$label.err" || status=$?
  ended="$(date -u +%Y-%m-%dT%H:%M:%S.%NZ)"
  {
    printf 'command='
    printf '%q ' "$@"
    printf '\nstatus=%s\nstarted_at=%s\nended_at=%s\n' "$status" "$started" "$ended"
  } >"$directory/$label.metadata"
  return "$status"
}

snapshot_owned() {
  local target="$1"
  install -d -m 0700 "$target" "$target/probes"
  capture_probe "$target/probes" ip-netns ip netns list
  awk '$1 ~ /^ovd-ns-/ {print $1}' "$target/probes/ip-netns.out" \
    | LC_ALL=C sort >"$target/netns"
  capture_probe "$target/probes" ip-links ip -o link show
  awk -F': ' \
    '{name=$2; sub(/@.*/, "", name); if (name ~ /^ovd-/) print name}' \
    "$target/probes/ip-links.out" | LC_ALL=C sort >"$target/links"
  if [[ -d /sys/fs/cgroup/overdrive.slice/workloads.slice ]]; then
    capture_probe "$target/probes" cgroups find \
      /sys/fs/cgroup/overdrive.slice/workloads.slice -maxdepth 1 -mindepth 1 \
      -type d -name 'alloc-*.scope' -printf '%f\n'
    LC_ALL=C sort "$target/probes/cgroups.out" >"$target/cgroups"
  else
    : >"$target/cgroups"
    printf 'command=find workload cgroups\nstatus=benign_absent\n' \
      >"$target/probes/cgroups.metadata"
    : >"$target/probes/cgroups.out"
    : >"$target/probes/cgroups.err"
  fi
  capture_probe "$target/probes" bpf-net bpftool net show
  local filter_status=0
  grep -E 'overdrive|ovd-' "$target/probes/bpf-net.out" \
    >"$target/bpf-net.unsorted" || filter_status=$?
  case "$filter_status" in
    0) LC_ALL=C sort "$target/bpf-net.unsorted" >"$target/bpf-net" ;;
    1) : >"$target/bpf-net" ;;
    *) return "$filter_status" ;;
  esac
  if capture_probe "$target/probes" nft nft -nn list table inet overdrive-mtls; then
    sed -E 's/counter packets [0-9]+ bytes [0-9]+/counter packets N bytes N/g' \
      "$target/probes/nft.out" >"$target/nft"
  elif grep -Eqi 'No such file|does not exist' "$target/probes/nft.err"; then
    : >"$target/nft"
  else
    return 1
  fi
}

stop_capture() {
  local pid="$1"
  local label="${2:-capture}"
  [[ -n "$pid" ]] || return 0
  local status=0
  kill -INT "$pid" 2>"$map_trace/${label}-stop.err" || status=$?
  if [[ "$status" -eq 0 ]]; then
    wait "$pid" 2>>"$map_trace/${label}-stop.err" || status=$?
  fi
  printf 'command=tcpdump capture/stop\nstatus=%s\nended_at=%s\n' \
    "$status" "$(date -u +%Y-%m-%dT%H:%M:%S.%NZ)" \
    >>"$map_trace/${label}-capture.metadata"
  return "$status"
}

cleanup() {
  local status=$?
  set +e
  rm -f -- "$pre_gate" "$observation_gate"
  stop_capture "$public_capture_pid" public || true
  stop_capture "$upstream_capture_pid" upstream || true
  if [[ -n "$control_server_pid" ]]; then
    kill -TERM "$control_server_pid" 2>/dev/null || true
    wait "$control_server_pid" 2>/dev/null || true
  fi
  if [[ -n "$example_pid" ]] && kill -0 "$example_pid" 2>/dev/null; then
    kill -TERM "$example_pid" 2>/dev/null
    wait "$example_pid" 2>/dev/null
  fi
  return "$status"
}
trap cleanup EXIT HUP INT TERM

[[ ! -e "$scratch_dir" && ! -e "$baseline" && ! -e "$post" ]] || {
  echo "AT-PIG-E2E-1: scratch paths must not pre-exist" >&2
  exit 2
}
touch "$pre_gate" "$observation_gate"
snapshot_owned "$baseline"

env PIG_PRE_REQUEST_GATE="$pre_gate" PIG_OBSERVATION_GATE="$observation_gate" \
  PIG_CAPTURE_NO_BACKEND=1 PIG_CAPTURE_FAILURE_MATRIX=1 \
  PIG_SERVE_TRACE_DIR="$connector_trace" \
  RUST_LOG='info,overdrive::gateway_connector=debug' \
  "$example" "$overdrive_bin" "$scratch_dir" 127.0.0.1 >"$example_log" 2>&1 &
example_pid=$!

deadline=$((SECONDS + 90))
while [[ ! -e "$pre_gate.ready" && "$SECONDS" -lt "$deadline" ]]; do
  kill -0 "$example_pid" 2>/dev/null || {
    cat "$example_log" >&2
    exit 1
  }
  sleep 0.1
done
[[ -e "$pre_gate.ready" ]] || {
  echo "AT-PIG-E2E-1: example did not reach pre-request gate" >&2
  exit 1
}

install -d -m 0700 "$map_trace" "$map_trace/probes"
capture_probe "$map_trace/probes" preflight-ip-links ip -o link show
mapfile -t upstream_ifaces < <(
  awk -F': ' \
    '{name=$2; sub(/@.*/, "", name); if (name ~ /^ovd-hv-/) print name}' \
    "$map_trace/probes/preflight-ip-links.out"
)
[[ "${#upstream_ifaces[@]}" -eq 1 ]] || {
  echo "AT-PIG-E2E-1: expected one production workload host veth, got ${#upstream_ifaces[@]}" >&2
  exit 1
}

capture_probe "$map_trace/probes" gateway-map-show bpftool map show
cp -- "$map_trace/probes/gateway-map-show.out" "$map_trace/maps.txt"
awk '/ name GATEWAY_CONNECT/ {id=$1; sub(/:$/, "", id); print id}' \
  "$map_trace/maps.txt" >"$map_trace/intent-map-id"
awk '/ name GATEWAY_SELECT/ {id=$1; sub(/:$/, "", id); print id}' \
  "$map_trace/maps.txt" >"$map_trace/receipt-map-id"
[[ -s "$map_trace/intent-map-id" && -s "$map_trace/receipt-map-id" ]] || {
  echo "AT-PIG-E2E-1: production gateway intent/receipt maps are absent" >&2
  exit 1
}

# An unregistered socket in the production serve cgroup must retain the
# pre-feature non-service path. The control owns no gateway map entry and the
# LOCAL_BACKEND_MAP/XDP attachment surfaces are byte-identical around it.
install -d -m 0700 "$scratch_dir.control"
printf 'unregistered-non-service-ok\n' >"$scratch_dir.control/index.html"
python3 -m http.server 18081 --bind 127.0.0.1 --directory "$scratch_dir.control" \
  >"$scratch_dir.control/server.log" 2>&1 &
control_server_pid=$!
deadline=$((SECONDS + 10))
while ! curl --silent --fail http://127.0.0.1:18081/ >/dev/null 2>&1; do
  [[ "$SECONDS" -lt "$deadline" ]] || {
    echo "AT-PIG-E2E-1: unregistered control server did not bind" >&2
    exit 1
  }
done
capture_probe "$map_trace/probes" xdp-before-control bpftool net show
LC_ALL=C sort "$map_trace/probes/xdp-before-control.out" >"$map_trace/xdp-before-control"
capture_probe "$map_trace/probes" local-map-show bpftool map show
awk '/ name LOCAL_BACKEND/ {id=$1; sub(/:$/, "", id); print id}' \
  "$map_trace/probes/local-map-show.out" >"$map_trace/local-map-ids"
[[ -s "$map_trace/local-map-ids" ]] || {
  echo "AT-PIG-E2E-1: production LOCAL_BACKEND_MAP is absent" >&2
  exit 1
}
while IFS= read -r map_id; do
  capture_probe "$map_trace/probes" "local-$map_id-before" bpftool -j map dump id "$map_id"
  cp -- "$map_trace/probes/local-$map_id-before.out" \
    "$map_trace/local-$map_id-before.json"
done <"$map_trace/local-map-ids"
systemd-run --quiet --wait --pipe --scope --slice=overdrive.slice \
  curl --silent --show-error --fail http://127.0.0.1:18081/ \
  >"$scratch_dir.control/client.out"
grep -Fxq unregistered-non-service-ok "$scratch_dir.control/client.out"
while IFS= read -r map_id; do
  capture_probe "$map_trace/probes" "local-$map_id-after" bpftool -j map dump id "$map_id"
  cp -- "$map_trace/probes/local-$map_id-after.out" \
    "$map_trace/local-$map_id-after.json"
  cmp -s "$map_trace/local-$map_id-before.json" "$map_trace/local-$map_id-after.json" || {
    echo "AT-PIG-E2E-1: unregistered control changed LOCAL_BACKEND_MAP" >&2
    exit 1
  }
done <"$map_trace/local-map-ids"
capture_probe "$map_trace/probes" xdp-after-control bpftool net show
LC_ALL=C sort "$map_trace/probes/xdp-after-control.out" >"$map_trace/xdp-after-control"
cmp -s "$map_trace/xdp-before-control" "$map_trace/xdp-after-control" || {
  echo "AT-PIG-E2E-1: unregistered control changed XDP attachment behavior" >&2
  exit 1
}
kill -TERM "$control_server_pid"
wait "$control_server_pid" 2>/dev/null || true
control_server_pid=""

printf 'started_at=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%S.%NZ)" \
  >"$map_trace/public-capture.metadata"
tcpdump -U -i lo -s 0 -w "$public_pcap" 'tcp port 443' \
  >"$map_trace/public-capture.out" 2>"$map_trace/public-capture.err" &
public_capture_pid=$!
tcpdump -U -i "${upstream_ifaces[0]}" -s 0 -w "$upstream_pcap" \
  'tcp port 8080' >"$map_trace/upstream-capture.out" 2>"$map_trace/upstream-capture.err" &
printf 'started_at=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%S.%NZ)" \
  >"$map_trace/upstream-capture.metadata"
upstream_capture_pid=$!
rm -f -- "$pre_gate"

deadline=$((SECONDS + 90))
while [[ ! -e "$observation_gate.ready" && "$SECONDS" -lt "$deadline" ]]; do
  kill -0 "$example_pid" 2>/dev/null || {
    cat "$example_log" >&2
    exit 1
  }
  sleep 0.1
done
[[ -e "$observation_gate.ready" ]] || {
  echo "AT-PIG-E2E-1: example did not reach observation gate" >&2
  exit 1
}

stop_capture "$public_capture_pid" public
public_capture_pid=""
stop_capture "$upstream_capture_pid" upstream
upstream_capture_pid=""

capture_probe "$map_trace/probes" post-request-map-show bpftool map show
cp -- "$map_trace/probes/post-request-map-show.out" "$scratch_dir/bpf-maps.txt"
awk '/ name GATEWAY_/ {id=$1; sub(/:$/, "", id); print id}' \
  "$scratch_dir/bpf-maps.txt" >"$scratch_dir/gateway-map-ids.txt"
[[ "$(wc -l <"$scratch_dir/gateway-map-ids.txt")" -eq 2 ]] || {
  echo "AT-PIG-E2E-1: expected exactly two gateway transient maps" >&2
  exit 1
}
while IFS= read -r map_id; do
  capture_probe "$map_trace/probes" "gateway-map-$map_id-post-request" \
    bpftool map dump id "$map_id"
  cp -- "$map_trace/probes/gateway-map-$map_id-post-request.out" \
    "$scratch_dir/gateway-map-$map_id.txt"
  grep -Eq 'Found 0 elements|\\[\\]' "$scratch_dir/gateway-map-$map_id.txt"
done <"$scratch_dir/gateway-map-ids.txt"
capture_probe "$map_trace/probes" socket-state ss -ntie
cp -- "$map_trace/probes/socket-state.out" "$scratch_dir/socket-state.txt"

grep -Fq 'public-ingress-gateway-stream-ok' "$scratch_dir/public-body.out"
grep -Eq '^HTTP/1\.1 200([[:space:]]|$)' "$scratch_dir/public-headers.out"
grep -Fiq 'x-workload: api' "$scratch_dir/public-headers.out"
grep -Fiq 'via: 1.1 overdrive' "$scratch_dir/public-headers.out"
python3 - "$scratch_dir/gateway-status.json" "127.0.0.1:443" "$scratch_dir" \
  "$scratch_dir/public-pki/api-origin-key.pem" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
expected_address = sys.argv[2]
scratch = pathlib.Path(sys.argv[3])
key_path = pathlib.Path(sys.argv[4])
raw = path.read_text()
body = json.loads(raw)
key_marker = next(
    line[:16] for line in key_path.read_text().splitlines()
    if not line.startswith("---") and len(line) >= 16
)
assert body["enabled"] is True
assert body["configured_address"] == expected_address
assert body["listener_bound"] is True
assert body["application"]["current"] is not None
assert body["application"]["listener"]["state"] == "bound"
assert body["application"]["gateway_identity"]["state"] == "current"
gateway_spiffe = body["application"]["gateway_identity"]["spiffe_id"]
assert gateway_spiffe.startswith("spiffe://overdrive.local/gateway/")
assert "/alloc/" not in gateway_spiffe
assert body["application"]["connect_path"] == "ready"
assert body["certified_key"]["state"]["state"] == "usable"
assert isinstance(body["hydration"], list) and body["hydration"]
for forbidden in (
    "certificate_chain_path", "private_key_path", "certificate_chain_der",
    "pem", "private_key", "ciphertext", "ciphertext_and_tag", "nonce", "salt",
    str(scratch / "credentials").lower(), str(scratch / "public-pki").lower(),
    "begin certificate", "begin private key",
    key_marker.lower(),
):
    assert forbidden not in raw.lower(), forbidden
PY
grep -Fq 'Accepted.' "$scratch_dir/service-deploy.out"
grep -Eq 'Declared|Replaced|Unchanged' "$scratch_dir/route-deploy.out"
grep -Fxq '200' "$scratch_dir/public-status"
grep -Fxq '404' "$scratch_dir/route-miss-status"
grep -Fxq '502' "$scratch_dir/upstream-failure-status"
grep -Fxq '503' "$scratch_dir/no-backend-status"
[[ "$(grep -Fc 'workload: abort-request-received' "$example_log")" -eq 1 ]] || {
  echo "AT-PIG-E2E-1: the 502 path retried/replayed or never reached the selected workload" >&2
  exit 1
}

capture_probe "$map_trace/probes" public-packets tcpdump -nn -r "$public_pcap"
cp -- "$map_trace/probes/public-packets.out" "$scratch_dir/public-packets.txt"
capture_probe "$map_trace/probes" upstream-packets tcpdump -nn -r "$upstream_pcap"
cp -- "$map_trace/probes/upstream-packets.out" "$scratch_dir/upstream-packets.txt"
[[ -s "$scratch_dir/public-packets.txt" && -s "$scratch_dir/upstream-packets.txt" ]]
assert_capture_excludes_marker "$public_pcap" public-ingress-gateway-stream-ok public
assert_capture_excludes_marker "$upstream_pcap" public-ingress-gateway-stream-ok upstream
capture_probe "$map_trace/probes" upstream-hex tcpdump -xx -nn -r "$upstream_pcap"
tr '\n' ' ' <"$map_trace/probes/upstream-hex.out" \
  >"$map_trace/upstream-hex-flat.txt"
grep -Eq '1703[[:space:]]+03|1703[[:space:]]+[0-9a-f]{2}' \
  "$map_trace/upstream-hex-flat.txt"

python3 "$repo_root/tests/tier3/public-ingress-gateway/receipt-oracle.py" \
  "$connector_trace" "$scratch_dir/observations" \
  "$scratch_dir/service-describe.out"
[[ "$(grep -Ec 'spiffe://overdrive\.local/workload/api/alloc/[a-z0-9._-]+' \
  "$scratch_dir/service-describe.out")" -eq 1 ]] || {
  echo "AT-PIG-E2E-1: selected BackendId lacks one exact applied api workload identity" >&2
  exit 1
}

rm -f -- "$observation_gate"
wait "$example_pid" || {
  cat "$example_log" >&2
  exit 1
}
example_pid=""

deadline=$((SECONDS + 15))
cleanup_sequence=0
install -d -m 0700 "$scratch_dir/cleanup-observations"
while [[ "$SECONDS" -lt "$deadline" ]]; do
  cleanup_sequence=$((cleanup_sequence + 1))
  cleanup_started="$(date -u +%Y-%m-%dT%H:%M:%S.%NZ)"
  printf -v cleanup_record '%s/cleanup-observations/%04d-owned-state' \
    "$scratch_dir" "$cleanup_sequence"
  snapshot_owned "$cleanup_record"
  cleanup_ended="$(date -u +%Y-%m-%dT%H:%M:%S.%NZ)"
  {
    printf 'command=snapshot owned netns/link/cgroup/BPF/nft state\n'
    printf 'status=0\n'
    printf 'started_at=%s\n' "$cleanup_started"
    printf 'ended_at=%s\n' "$cleanup_ended"
  } >"$cleanup_record/metadata"
  install -d -m 0700 "$post"
  for surface in netns links cgroups bpf-net nft; do
    cp -- "$cleanup_record/$surface" "$post/$surface"
  done
  if cmp -s "$baseline/netns" "$cleanup_record/netns" \
    && cmp -s "$baseline/links" "$cleanup_record/links" \
    && cmp -s "$baseline/cgroups" "$cleanup_record/cgroups" \
    && cmp -s "$baseline/bpf-net" "$cleanup_record/bpf-net" \
    && cmp -s "$baseline/nft" "$cleanup_record/nft"; then
    break
  fi
  sleep 0.25
done

for surface in netns links cgroups bpf-net nft; do
  cmp -s "$baseline/$surface" "$post/$surface" || {
    diff -u "$baseline/$surface" "$post/$surface" >&2 || true
    echo "AT-PIG-E2E-1: cleanup changed owned $surface state" >&2
    exit 1
  }
done

echo "AT-PIG-E2E-1 PASS: external HTTPS reached the exact Service through BPF-selected gateway-SVID mTLS and cleanup converged"
