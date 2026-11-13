#!/usr/bin/env bash
# Run the checked-in public-ingress-gateway journey on qualified native metal.
#
#   run-example.sh <absolute-overdrive-binary> <absolute-scratch-dir> <gateway-ipv4> \
#     [--public-hostname <PUBLIC_HOSTNAME> \
#      --certificate-chain <absolute-chain-path> \
#      --private-key <absolute-key-path>]
#
# The contract is argv-only: no environment variable selects the gateway
# address, the Public Hostname, the certificate paths or the trust mode.
# With no named options the script runs hermetically (hostname api.example.com,
# a generated test CA, curl --cacert). With all three options it runs in
# production-trust mode (system trust; no --cacert, no verification bypass).
# The public request always uses `curl --resolve <hostname>:443:<gateway-ipv4>`;
# the script never edits /etc/hosts and never relies on DNS.
set -euo pipefail

usage() {
  echo "usage: $0 <absolute-overdrive-binary> <absolute-scratch-dir> <gateway-ipv4> [--public-hostname <hostname> --certificate-chain <absolute-path> --private-key <absolute-path>]" >&2
  exit 2
}

die() {
  echo "public-ingress example: $*" >&2
  exit 1
}

[[ "$#" -ge 3 ]] || usage
readonly overdrive_bin="$1"
readonly scratch_dir="$2"
readonly gateway_address="$3"
shift 3
[[ "$overdrive_bin" = /* && "$scratch_dir" = /* ]] || usage
[[ "$gateway_address" =~ ^([0-9]{1,3}\.){3}[0-9]{1,3}$ ]] || usage
[[ -x "$overdrive_bin" && -f "$overdrive_bin" ]] || {
  echo "public-ingress example: binary is not an executable file: $overdrive_bin" >&2
  exit 2
}

example_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly example_dir
readonly prepare="$example_dir/prepare.sh"
hostname="api.example.com"
certificate_chain_path=""
private_key_path=""
production_trust=0
if [[ "$#" -ne 0 ]]; then
  [[ "$#" -eq 6 ]] || usage
  seen_hostname=0
  seen_chain=0
  seen_key=0
  while [[ "$#" -gt 0 ]]; do
    case "$1" in
      --public-hostname)
        [[ "$seen_hostname" -eq 0 && "$#" -ge 2 ]] || usage
        hostname="$2"
        seen_hostname=1
        shift 2
        ;;
      --certificate-chain)
        [[ "$seen_chain" -eq 0 && "$#" -ge 2 ]] || usage
        certificate_chain_path="$2"
        seen_chain=1
        shift 2
        ;;
      --private-key)
        [[ "$seen_key" -eq 0 && "$#" -ge 2 ]] || usage
        private_key_path="$2"
        seen_key=1
        shift 2
        ;;
      *) usage ;;
    esac
  done
  [[ "$seen_hostname" -eq 1 && "$seen_chain" -eq 1 && "$seen_key" -eq 1 ]] || usage
  production_trust=1
fi
[[ "$hostname" =~ ^[A-Za-z0-9]([A-Za-z0-9.-]*[A-Za-z0-9])?$ ]] || usage
readonly hostname production_trust

readonly config_dir="$scratch_dir/config"
readonly data_dir="$scratch_dir/data"
readonly credentials_dir="$scratch_dir/credentials"
readonly ca_dir="$scratch_dir/public-pki"
readonly operator_dir="$scratch_dir/operator-mtls"
readonly observations_dir="$scratch_dir/observations"
readonly specs_dir="$scratch_dir/specs"
readonly serve_log="$scratch_dir/serve.log"
readonly config_file="$config_dir/.overdrive/config"
readonly route_spec="$specs_dir/route.toml"
readonly service_id="api"
readonly route_id="public-api"
readonly expected_stream_chunks=8
readonly expected_exact_body="PIG-EXACT-OK"

serve_pid=""
prepared=0
service_deployed=0
route_deployed=0
snapshot_dir=""
observation_seq=0

bounded() {
  local duration="$1"
  shift
  timeout --foreground --signal=TERM --kill-after=5s "$duration" "$@"
}

utc_now() {
  date -u +%Y-%m-%dT%H:%M:%SZ
}

require_native_metal() {
  [[ "$(uname -s)" == "Linux" && "$(uname -m)" == "x86_64" ]] \
    || die "runtime is restricted to native x86_64 Linux metal"
  [[ "$(id -u)" -eq 0 ]] || die "run through cargo xtask metal run -- (root context)"
  [[ -c /dev/kvm ]] || die "/dev/kvm is not an accessible character device"
  local command
  for command in awk cloud-hypervisor curl find findmnt grep ip keyctl losetup \
    python3 sed setsid sort ss stat timeout; do
    command -v "$command" >/dev/null 2>&1 || die "required command is unavailable: $command"
  done
}

probe_hypervisors() {
  local proc argv0
  for proc in /proc/[0-9]*; do
    [[ -r "$proc/cmdline" ]] || continue
    argv0=''
    IFS= read -r -d '' argv0 <"$proc/cmdline" || true
    [[ "$(basename "$argv0" 2>/dev/null || true)" == "cloud-hypervisor" ]] \
      && basename "$proc" || true
  done | sort -n
}

probe_scopes() {
  find /sys/fs/cgroup/overdrive.slice/workloads.slice -maxdepth 1 -mindepth 1 \
    -type d -name 'alloc-*.scope' -printf '%f\n' 2>/dev/null | sort
}

probe_run_dirs() {
  find /run/overdrive/vm -maxdepth 1 -mindepth 1 -type d -printf '%f\n' \
    2>/dev/null | sort
}

probe_network() {
  { ip -br link show; ip netns list; } 2>/dev/null | sort
}

probe_loops() {
  losetup -j "/srv/vm/overdrive-testing/pig-example/rootfs.ext4" 2>/dev/null || true
}

probe_mounts() {
  findmnt --raw --noheadings --output TARGET,SOURCE 2>/dev/null \
    | grep -F "/srv/vm/overdrive-testing/pig-example" || true
}

# Listeners bound to TCP/443, any state.
probe_public_sockets() {
  ss -Htan 'sport = :443' 2>/dev/null | sort || true
}

# Sockets with the kernel TLS ULP installed (public and upstream legs are both
# such sockets while a gateway exchange is open).
probe_ktls_sockets() {
  ss -Htni 2>/dev/null | grep -B1 'tcp-ulp-tls' | grep -E '^(ESTAB|FIN|CLOSE|LAST|TIME)' | sort || true
}

snapshot_before() {
  snapshot_dir="$(mktemp -d "${TMPDIR:-/tmp}/pig-example-snapshot.XXXXXX")"
  probe_hypervisors >"$snapshot_dir/hypervisors"
  probe_scopes >"$snapshot_dir/scopes"
  probe_run_dirs >"$snapshot_dir/run-dirs"
  probe_network >"$snapshot_dir/network"
  probe_loops >"$snapshot_dir/loops"
  probe_mounts >"$snapshot_dir/mounts"
  probe_public_sockets >"$snapshot_dir/public-sockets"
  probe_ktls_sockets >"$snapshot_dir/ktls-sockets"
}

new_delta_count() {
  local before="$1"
  shift
  comm -13 "$before" <("$@") | sed '/^$/d' | wc -l | tr -d ' '
}

owned_runtime_resources_released() {
  local count
  for pair in "hypervisors:probe_hypervisors" "scopes:probe_scopes" \
    "network:probe_network" "run-dirs:probe_run_dirs" "loops:probe_loops" \
    "mounts:probe_mounts"; do
    count="$(new_delta_count "$snapshot_dir/${pair%%:*}" "${pair#*:}")"
    [[ "$count" -eq 0 ]] || return 1
  done
}

wait_for_owned_runtime_cleanup() {
  local deadline=$((SECONDS + 45))
  while [[ "$SECONDS" -lt "$deadline" ]]; do
    owned_runtime_resources_released && return 0
    sleep 1
  done
  return 1
}

report_cleanup_deltas() {
  local vms scopes networks run_dirs loops mounts public ktls
  vms="$(new_delta_count "$snapshot_dir/hypervisors" probe_hypervisors)"
  scopes="$(new_delta_count "$snapshot_dir/scopes" probe_scopes)"
  networks="$(new_delta_count "$snapshot_dir/network" probe_network)"
  run_dirs="$(new_delta_count "$snapshot_dir/run-dirs" probe_run_dirs)"
  loops="$(new_delta_count "$snapshot_dir/loops" probe_loops)"
  mounts="$(new_delta_count "$snapshot_dir/mounts" probe_mounts)"
  public="$(new_delta_count "$snapshot_dir/public-sockets" probe_public_sockets)"
  ktls="$(new_delta_count "$snapshot_dir/ktls-sockets" probe_ktls_sockets)"
  printf 'public-ingress teardown deltas: vm=%s network=%s cgroup=%s run-directory=%s mount=%s loop=%s public-443-sockets=%s ktls-sockets=%s\n' \
    "$vms" "$networks" "$scopes" "$run_dirs" "$mounts" "$loops" "$public" "$ktls"
  [[ "$vms" -eq 0 && "$networks" -eq 0 && "$scopes" -eq 0 && "$run_dirs" -eq 0 \
    && "$mounts" -eq 0 && "$loops" -eq 0 && "$public" -eq 0 && "$ktls" -eq 0 ]]
}

record_observation() {
  local kind="$1" command_text="$2" command_status="$3" started_at="$4" ended_at="$5"
  shift 5
  observation_seq=$((observation_seq + 1))
  local record
  printf -v record '%s/%04d-%s' "$observations_dir" "$observation_seq" "$kind"
  install -d -m 0700 "$record"
  {
    printf 'command=%s\n' "$command_text"
    printf 'status=%s\n' "$command_status"
    printf 'started_at=%s\n' "$started_at"
    printf 'ended_at=%s\n' "$ended_at"
  } >"$record/metadata"
  while [[ "$#" -ge 2 ]]; do
    [[ -e "$2" ]] && cp -- "$2" "$record/$1"
    shift 2
  done
}

operator_curl() {
  local method="$1" path="$2"
  shift 2
  local endpoint
  endpoint="$(<"$operator_dir/endpoint")"
  bounded 10s curl --silent --show-error --fail-with-body \
    --request "$method" \
    --cacert "$operator_dir/ca.pem" \
    --cert "$operator_dir/client.pem" \
    --key "$operator_dir/client-key.pem" \
    "$@" "$endpoint$path"
}

first_service_alloc_state() {
  awk '
    /^Alloc[[:space:]]+State[[:space:]]+/ { in_table = 1; next }
    in_table && /^[-[:space:]]+$/ { next }
    in_table && $1 ~ /^alloc-/ { print $2; exit }
  '
}

wait_for_service_stop() {
  local output="$scratch_dir/service-stopped.out"
  local deadline=$((SECONDS + 45))
  while [[ "$SECONDS" -lt "$deadline" ]]; do
    bounded 10s env OVERDRIVE_CONFIG_DIR="$config_dir" \
      "$overdrive_bin" workload describe "$service_id" >"$output" 2>&1 || true
    [[ "$(first_service_alloc_state <"$output")" == "Terminated" ]] && return 0
    sleep 1
  done
  return 1
}

stop_serve() {
  [[ -n "$serve_pid" ]] || return 0
  if kill -0 "$serve_pid" 2>/dev/null; then
    kill -- "-$serve_pid" 2>/dev/null || kill "$serve_pid" 2>/dev/null || true
  fi
  local attempts=50
  while kill -0 "$serve_pid" 2>/dev/null && [[ "$attempts" -gt 0 ]]; do
    sleep 0.1
    attempts=$((attempts - 1))
  done
  if kill -0 "$serve_pid" 2>/dev/null; then
    kill -KILL -- "-$serve_pid" 2>/dev/null || kill -KILL "$serve_pid" 2>/dev/null || true
    wait "$serve_pid" 2>/dev/null || true
    serve_pid=""
    return 1
  fi
  wait "$serve_pid" 2>/dev/null || true
  serve_pid=""
}

cleanup() {
  # shellcheck disable=SC2320 # EXIT trap supplies the status being preserved.
  local incoming_rc=$?
  trap - EXIT HUP INT TERM
  set +e
  local failed=0
  if [[ "$route_deployed" -eq 1 && -s "$operator_dir/endpoint" ]]; then
    local started status=0
    started="$(utc_now)"
    operator_curl DELETE "/v1/routes/$route_id" \
      >"$scratch_dir/route-delete.out" 2>"$scratch_dir/route-delete.err" || status=$?
    record_observation cleanup-route-delete \
      "operator-mTLS DELETE /v1/routes/$route_id" "$status" "$started" "$(utc_now)" \
      stdout "$scratch_dir/route-delete.out" stderr "$scratch_dir/route-delete.err"
    [[ "$status" -eq 0 ]] || failed=1
  fi
  if [[ "$service_deployed" -eq 1 ]]; then
    local started status=0
    started="$(utc_now)"
    bounded 30s env OVERDRIVE_CONFIG_DIR="$config_dir" \
      "$overdrive_bin" job stop "$service_id" \
      >"$scratch_dir/service-stop.out" 2>"$scratch_dir/service-stop.err" || status=$?
    record_observation cleanup-service-stop "overdrive job stop $service_id" \
      "$status" "$started" "$(utc_now)" \
      stdout "$scratch_dir/service-stop.out" stderr "$scratch_dir/service-stop.err"
    [[ "$status" -eq 0 ]] || failed=1
    wait_for_service_stop || failed=1
  fi
  if [[ -n "$snapshot_dir" ]]; then
    wait_for_owned_runtime_cleanup || failed=1
  fi
  stop_serve || failed=1
  [[ "$prepared" -eq 0 ]] || bounded 45s "$prepare" cleanup || failed=1
  if [[ -n "$snapshot_dir" ]]; then
    report_cleanup_deltas | tee "$scratch_dir/cleanup-deltas" || failed=1
    rm -rf -- "$snapshot_dir"
  fi
  printf 'example_exit=%s cleanup_failed=%s\n' "$incoming_rc" "$failed" \
    >"$scratch_dir/example-exit"
  [[ "$incoming_rc" -ne 0 || "$failed" -eq 0 ]] || exit 1
  exit "$incoming_rc"
}

# --- preflight --------------------------------------------------------------
[[ ! -e "$scratch_dir" ]] || {
  echo "public-ingress example: scratch path already exists: $scratch_dir" >&2
  exit 2
}
require_native_metal
"$prepare" check-source

if [[ "$production_trust" -eq 1 ]]; then
  for named_path in "certificate-chain:$certificate_chain_path" "private-key:$private_key_path"; do
    supplied="${named_path#*:}"
    [[ "$supplied" = /* && -f "$supplied" && -r "$supplied" ]] || {
      echo "public-ingress example: supplied ${named_path%%:*} must be an absolute readable regular file" >&2
      exit 2
    }
  done
  [[ "$(stat -c '%a' "$private_key_path")" == "600" ]] || {
    echo "public-ingress example: supplied private-key mode must be exactly 0600" >&2
    exit 2
  }
fi

template="$example_dir/route.toml.template"
[[ "$(grep -Fo '__PUBLIC_HOSTNAME__' "$template" | wc -l | tr -d ' ')" -eq 1 ]] || {
  echo "public-ingress example: Route template must contain one hostname token" >&2
  exit 2
}

# --- scratch + owned state ---------------------------------------------------
# The VMM runs unprivileged and must traverse the serve data path.
install -d -m 0711 "$scratch_dir" "$data_dir"
install -d -m 0700 "$config_dir" "$credentials_dir" "$ca_dir" "$operator_dir" \
  "$observations_dir" "$specs_dir"
setpriv --reuid=4200 --regid=4200 --clear-groups test -x "$data_dir" || {
  echo "public-ingress example: uid 4200 cannot traverse the scratch data path and its ancestors" >&2
  exit 2
}
snapshot_before
trap cleanup EXIT
trap 'exit 130' HUP INT TERM

# Two operator KEKs: the workload CA root and the gateway's public
# certified-key custody (overdrive-host ca::public_certified_key).
for kek in overdrive-ca-root overdrive-public-certified-key; do
  head -c 32 /dev/urandom >"$credentials_dir/$kek"
  chmod 0400 "$credentials_dir/$kek"
done

public_trust_args=()
if [[ "$production_trust" -eq 0 ]]; then
  "$example_dir/prepare-test-ca.sh" "$ca_dir" "$hostname" >"$scratch_dir/test-ca-paths"
  certificate_chain_path="$ca_dir/api-origin-chain.pem"
  private_key_path="$ca_dir/api-origin-key.pem"
  public_trust_args=(--cacert "$ca_dir/test-ca.pem")
fi
readonly certificate_chain_path private_key_path

sed "s/__PUBLIC_HOSTNAME__/$hostname/" "$template" >"$route_spec"
[[ "$(grep -Fo '__PUBLIC_HOSTNAME__' "$route_spec" | wc -l | tr -d ' ')" -eq 0 ]] \
  || die "materialized Route retained a hostname token"

prepared=1
bounded 240s "$prepare" prepare
bounded 45s "$prepare" check
[[ "$(stat -c %d /srv/vm/overdrive-testing/pig-example/rootfs.ext4)" == "$(stat -c %d "$data_dir")" ]] \
  || die "the scratch directory must share a filesystem with /srv/vm/overdrive-testing (reflinked rootfs staging)"

# --- serve ---------------------------------------------------------------
env OVERDRIVE_CONFIG_DIR="$config_dir" CREDENTIALS_DIRECTORY="$credentials_dir" \
  setsid keyctl session - "$overdrive_bin" serve \
  --bind 127.0.0.1:0 \
  --data-dir "$data_dir" \
  --gateway-address "$gateway_address" \
  --gateway-certified-key-id api-origin \
  --gateway-certificate-chain "$certificate_chain_path" \
  --gateway-private-key "$private_key_path" \
  >>"$serve_log" 2>&1 &
serve_pid=$!
printf '%s\n' "$serve_pid" >"$scratch_dir/serve.pid"

deadline=$((SECONDS + 30))
while [[ ! -s "$config_file" && "$SECONDS" -lt "$deadline" ]]; do
  kill -0 "$serve_pid" 2>/dev/null || die "serve exited before operator config appeared"
  sleep 0.1
done
[[ -s "$config_file" ]] || die "operator config did not appear"

python3 - "$config_file" "$operator_dir" <<'PY'
import base64
import pathlib
import sys
import tomllib

data = tomllib.loads(pathlib.Path(sys.argv[1]).read_text())
out = pathlib.Path(sys.argv[2])
context = next(c for c in data["contexts"] if c["name"] == data["current-context"])
(out / "endpoint").write_text(context["endpoint"])
for field, name in (("ca", "ca.pem"), ("crt", "client.pem"), ("key", "client-key.pem")):
    (out / name).write_bytes(base64.b64decode(context[field], validate=True))
PY
chmod 0600 "$operator_dir/client-key.pem"

# --- Service ---------------------------------------------------------------
started="$(utc_now)"
rc=0
bounded 60s env OVERDRIVE_CONFIG_DIR="$config_dir" \
  "$overdrive_bin" deploy "$example_dir/service.toml" --detach \
  >"$scratch_dir/service-deploy.out" 2>"$scratch_dir/service-deploy.err" || rc=$?
record_observation service-deploy 'overdrive deploy service.toml --detach' "$rc" \
  "$started" "$(utc_now)" stdout "$scratch_dir/service-deploy.out" \
  stderr "$scratch_dir/service-deploy.err"
service_deployed=1
[[ "$rc" -eq 0 ]] || exit "$rc"

deadline=$((SECONDS + 90))
while [[ "$SECONDS" -lt "$deadline" ]]; do
  started="$(utc_now)"
  rc=0
  bounded 10s env OVERDRIVE_CONFIG_DIR="$config_dir" \
    "$overdrive_bin" workload describe "$service_id" \
    >"$scratch_dir/service-describe.out" 2>"$scratch_dir/service-describe.err" || rc=$?
  record_observation service-describe "overdrive workload describe $service_id" "$rc" \
    "$started" "$(utc_now)" stdout "$scratch_dir/service-describe.out" \
    stderr "$scratch_dir/service-describe.err"
  [[ "$rc" -eq 0 && "$(first_service_alloc_state <"$scratch_dir/service-describe.out")" == "Running" ]] \
    && break
  sleep 0.5
done
[[ "$(first_service_alloc_state <"$scratch_dir/service-describe.out")" == "Running" ]] \
  || die "api Service did not reach Running"

# --- Route -----------------------------------------------------------------
started="$(utc_now)"
rc=0
bounded 30s env OVERDRIVE_CONFIG_DIR="$config_dir" \
  "$overdrive_bin" deploy "$route_spec" --detach \
  >"$scratch_dir/route-deploy.out" 2>"$scratch_dir/route-deploy.err" || rc=$?
record_observation route-deploy 'overdrive deploy route.toml --detach' "$rc" \
  "$started" "$(utc_now)" stdout "$scratch_dir/route-deploy.out" \
  stderr "$scratch_dir/route-deploy.err"
[[ "$rc" -eq 0 ]] && route_deployed=1
[[ "$rc" -eq 0 ]] || exit "$rc"

status_exit=1
deadline=$((SECONDS + 60))
while [[ "$SECONDS" -lt "$deadline" ]]; do
  started="$(utc_now)"
  status_exit=0
  operator_curl GET "/v1/gateway/status" \
    >"$scratch_dir/gateway-status.json" 2>"$scratch_dir/gateway-status.err" || status_exit=$?
  record_observation gateway-status 'operator-mTLS GET /v1/gateway/status' "$status_exit" \
    "$started" "$(utc_now)" response.json "$scratch_dir/gateway-status.json" \
    stderr "$scratch_dir/gateway-status.err"
  [[ "$status_exit" -eq 0 ]] && grep -Fq '"listener_bound":true' "$scratch_dir/gateway-status.json" && break
  sleep 0.25
done
if [[ "$status_exit" -ne 0 ]] \
  || ! grep -Fq '"listener_bound":true' "$scratch_dir/gateway-status.json"; then
  die "gateway status did not become active"
fi

# --- public request ---------------------------------------------------------
expected_stream_body=""
for ((index = 0; index < expected_stream_chunks; index++)); do
  expected_stream_body+="$(printf 'PIG-STREAM-CHUNK-%02d' "$index")"$'\n'
done
printf '%s' "$expected_stream_body" >"$scratch_dir/expected-stream-body"

public_curl() {
  local path="$1" prefix="$2"
  bounded 30s curl --silent --show-error --verbose \
    --tlsv1.3 --tls-max 1.3 --http1.1 \
    "${public_trust_args[@]}" \
    --resolve "$hostname:443:$gateway_address" \
    --dump-header "$scratch_dir/$prefix-headers.out" \
    --output "$scratch_dir/$prefix-body.out" \
    --write-out 'http_code=%{http_code} ssl_verify_result=%{ssl_verify_result} time_starttransfer=%{time_starttransfer} time_total=%{time_total}\n' \
    "https://$hostname$path" \
    >"$scratch_dir/$prefix-timing.out" 2>"$scratch_dir/$prefix-verbose.err"
}

deadline=$((SECONDS + 90))
stream_ok=0
while [[ "$SECONDS" -lt "$deadline" ]]; do
  started="$(utc_now)"
  rc=0
  public_curl /stream stream || rc=$?
  record_observation public-stream \
    "curl --resolve $hostname:443:$gateway_address https://$hostname/stream" \
    "$rc/$(<"$scratch_dir/stream-timing.out")" "$started" "$(utc_now)" \
    headers "$scratch_dir/stream-headers.out" body "$scratch_dir/stream-body.out" \
    timing "$scratch_dir/stream-timing.out" verbose "$scratch_dir/stream-verbose.err" \
    serve-log "$serve_log"
  if [[ "$rc" -eq 0 ]] && grep -Fq 'http_code=200 ' "$scratch_dir/stream-timing.out"; then
    stream_ok=1
    break
  fi
  sleep 0.5
done
[[ "$stream_ok" -eq 1 ]] || die "external HTTPS request to /stream did not complete"
grep -Eiq '^X-Workload: api\r?$' "$scratch_dir/stream-headers.out" \
  || die "/stream response lacks X-Workload: api"
cmp -s "$scratch_dir/stream-body.out" "$scratch_dir/expected-stream-body" \
  || die "/stream body differs from the guest's published stream"
grep -Fq 'ssl_verify_result=0' "$scratch_dir/stream-timing.out" \
  || die "/stream certificate verification result was not 0"
grep -Eq 'SSL connection using TLSv1\.3|TLSv1\.3 / ' "$scratch_dir/stream-verbose.err" \
  || die "/stream did not negotiate TLS 1.3"

started="$(utc_now)"
rc=0
public_curl /exact exact || rc=$?
record_observation public-exact \
  "curl --resolve $hostname:443:$gateway_address https://$hostname/exact" \
  "$rc/$(<"$scratch_dir/exact-timing.out")" "$started" "$(utc_now)" \
  headers "$scratch_dir/exact-headers.out" body "$scratch_dir/exact-body.out" \
  timing "$scratch_dir/exact-timing.out"
[[ "$rc" -eq 0 && "$(<"$scratch_dir/exact-body.out")" == "$expected_exact_body" ]] \
  || die "/exact body differs from the guest's Content-Length reply"
grep -Eiq '^Content-Length: ' "$scratch_dir/exact-headers.out" \
  || die "/exact response lacks Content-Length"

started="$(utc_now)"
rc=0
public_curl /count count || rc=$?
record_observation public-request-count \
  "curl --resolve $hostname:443:$gateway_address https://$hostname/count" \
  "$rc/$(<"$scratch_dir/count-timing.out")" "$started" "$(utc_now)" \
  body "$scratch_dir/count-body.out"
[[ "$rc" -eq 0 ]] || die "/count request failed"

echo "public-ingress example PASS: $hostname/stream returned 200, X-Workload: api and the guest's streamed body over TLS 1.3"
cat "$scratch_dir/route-deploy.out"
cat "$scratch_dir/gateway-status.json"
cat "$scratch_dir/stream-headers.out"
cat "$scratch_dir/stream-body.out"
cat "$scratch_dir/stream-timing.out"
cat "$scratch_dir/exact-body.out"
cat "$scratch_dir/count-body.out"
