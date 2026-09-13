#!/usr/bin/env bash
set -euo pipefail

usage() {
  echo "usage: $0 <absolute-overdrive-binary> <absolute-scratch-dir> <gateway-ipv4> [--public-hostname <hostname> --certificate-chain <absolute-path> --private-key <absolute-path>]" >&2
  exit 2
}

[[ "$#" -ge 3 ]] || usage
readonly overdrive_bin="$1"
readonly scratch_dir="$2"
readonly gateway_address="$3"
shift 3
[[ "$overdrive_bin" = /* && "$scratch_dir" = /* ]] || usage
[[ -x "$overdrive_bin" ]] || {
  echo "public-ingress example: binary is not executable: $overdrive_bin" >&2
  exit 2
}

example_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly example_dir
repo_root="$(cd "$example_dir/../.." && pwd)"
readonly repo_root
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
readonly hostname production_trust
readonly config_dir="$scratch_dir/config"
readonly data_dir="$scratch_dir/data"
readonly credentials_dir="$scratch_dir/credentials"
readonly ca_dir="$scratch_dir/public-pki"
readonly operator_dir="$scratch_dir/operator-mtls"
readonly serve_log="$scratch_dir/serve.log"
readonly config_file="$config_dir/.overdrive/config"
readonly observations_dir="$scratch_dir/observations"
readonly specs_dir="$scratch_dir/specs"
readonly route_spec="$specs_dir/route.toml"
readonly narrow_route_spec="$specs_dir/route-narrow.toml"

serve_pid=""
service_deployed=0
route_deployed=0
observation_seq=0

bounded() {
  local duration="$1"
  shift
  timeout --foreground --signal=TERM --kill-after=5s "$duration" "$@"
}

operator_curl() {
  local method="$1"
  local path="$2"
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

record_observation() {
  local kind="$1"
  local command_text="$2"
  local command_status="$3"
  local started_at="$4"
  local ended_at="$5"
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
    local name="$1"
    local source="$2"
    shift 2
    [[ -e "$source" ]] && cp -- "$source" "$record/$name"
  done
}

capture_serve_suffix() {
  local offset="$1"
  local output="$2"
  tail -c "+$((offset + 1))" "$serve_log" >"$output"
}

cleanup() {
  local status=$?
  set +e
  if [[ "$route_deployed" -eq 1 && -s "$operator_dir/endpoint" ]]; then
    local route_delete_started route_delete_ended route_delete_status
    route_delete_started="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    route_delete_status=0
    operator_curl DELETE "/v1/routes/public-api" \
      >"$scratch_dir/route-delete.out" 2>"$scratch_dir/route-delete.err" || route_delete_status=$?
    route_delete_ended="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    record_observation cleanup-route-delete \
      'operator-mTLS DELETE /v1/routes/public-api' "$route_delete_status" \
      "$route_delete_started" "$route_delete_ended" \
      stdout "$scratch_dir/route-delete.out" stderr "$scratch_dir/route-delete.err"
  fi
  if [[ "$service_deployed" -eq 1 ]]; then
    local service_stop_started service_stop_ended service_stop_status
    service_stop_started="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    service_stop_status=0
    env OVERDRIVE_CONFIG_DIR="$config_dir" \
      "$overdrive_bin" job stop api \
      >"$scratch_dir/service-stop.out" 2>"$scratch_dir/service-stop.err" || service_stop_status=$?
    service_stop_ended="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    record_observation cleanup-service-stop 'overdrive job stop api' \
      "$service_stop_status" "$service_stop_started" "$service_stop_ended" \
      stdout "$scratch_dir/service-stop.out" stderr "$scratch_dir/service-stop.err"
  fi
  if [[ -n "$serve_pid" ]] && kill -0 "$serve_pid" 2>/dev/null; then
    kill -INT -- "-$serve_pid" 2>/dev/null
    local deadline=$((SECONDS + 15))
    while kill -0 "$serve_pid" 2>/dev/null && [[ "$SECONDS" -lt "$deadline" ]]; do
      sleep 0.1
    done
    kill -KILL -- "-$serve_pid" 2>/dev/null
    wait "$serve_pid" 2>/dev/null
  fi
  printf 'example_exit=%s\n' "$status" >"$scratch_dir/example-exit"
  return "$status"
}

[[ ! -e "$scratch_dir" ]] || {
  echo "public-ingress example: scratch path already exists: $scratch_dir" >&2
  exit 2
}
install -d -m 0700 "$scratch_dir" "$config_dir" "$data_dir" \
  "$credentials_dir" "$ca_dir" "$operator_dir" "$observations_dir" "$specs_dir"
trap cleanup EXIT HUP INT TERM

printf '0123456789abcdef0123456789abcdef' \
  >"$credentials_dir/overdrive-ca-root"
chmod 0400 "$credentials_dir/overdrive-ca-root"

public_trust_args=()
if [[ "$production_trust" -eq 0 ]]; then
  "$example_dir/prepare-test-ca.sh" "$ca_dir" "$hostname" \
    >"$scratch_dir/test-ca-paths"
  certificate_chain_path="$ca_dir/api-origin-chain.pem"
  private_key_path="$ca_dir/api-origin-key.pem"
  public_ca_path="$ca_dir/test-ca.pem"
  public_trust_args=(--cacert "$public_ca_path")
else
  for named_path in \
    "certificate-chain:$certificate_chain_path" \
    "private-key:$private_key_path"; do
    label="${named_path%%:*}"
    supplied="${named_path#*:}"
    [[ "$supplied" = /* && -r "$supplied" ]] || {
      echo "public-ingress example: supplied $label path must be absolute and readable" >&2
      exit 2
    }
  done
fi

readonly certificate_chain_path private_key_path
[[ "$(stat -c '%a' "$private_key_path")" == "600" ]] || {
  echo "public-ingress example: supplied private-key mode must be exactly 0600" >&2
  exit 2
}

for template_and_output in \
  "$example_dir/route.toml.template:$route_spec" \
  "$example_dir/route-narrow.toml.template:$narrow_route_spec"; do
  template="${template_and_output%%:*}"
  output="${template_and_output#*:}"
  [[ "$(grep -Fo '__PUBLIC_HOSTNAME__' "$template" | wc -l | tr -d ' ')" -eq 1 ]] || {
    echo "public-ingress example: Route template must contain one hostname token" >&2
    exit 2
  }
  sed "s/__PUBLIC_HOSTNAME__/$hostname/" "$template" >"$output"
  [[ "$(grep -Fo '__PUBLIC_HOSTNAME__' "$output" | wc -l | tr -d ' ')" -eq 0 ]] || {
    echo "public-ingress example: materialized Route retained a hostname token" >&2
    exit 2
  }
done

cd "$repo_root"
serve_command=("$overdrive_bin" serve \
  --bind 127.0.0.1:0 \
  --data-dir "$data_dir" \
  --gateway-address "$gateway_address" \
  --gateway-certified-key-id api-origin \
  --gateway-certificate-chain "$certificate_chain_path" \
  --gateway-private-key "$private_key_path")
if [[ -n "${PIG_SERVE_TRACE_DIR:-}" ]]; then
  install -d -m 0700 "$PIG_SERVE_TRACE_DIR"
  serve_command=(strace -ff -ttt -yy -xx -e 'trace=bpf,connect,getsockopt' \
    -o "$PIG_SERVE_TRACE_DIR/serve" "${serve_command[@]}")
fi
env OVERDRIVE_CONFIG_DIR="$config_dir" \
  CREDENTIALS_DIRECTORY="$credentials_dir" \
  setsid "${serve_command[@]}" \
  >>"$serve_log" 2>&1 &
serve_pid=$!
printf '%s\n' "$serve_pid" >"$scratch_dir/serve.pid"

deadline=$((SECONDS + 30))
while [[ ! -s "$config_file" && "$SECONDS" -lt "$deadline" ]]; do
  kill -0 "$serve_pid" 2>/dev/null || {
    echo "public-ingress example: serve exited before operator config appeared" >&2
    exit 1
  }
  sleep 0.1
done
[[ -s "$config_file" ]] || {
  echo "public-ingress example: operator config did not appear" >&2
  exit 1
}

python3 - "$config_file" "$operator_dir" <<'PY'
import base64
import pathlib
import sys
import tomllib

config_path = pathlib.Path(sys.argv[1])
out = pathlib.Path(sys.argv[2])
data = tomllib.loads(config_path.read_text())
current = data["current-context"]
context = next(item for item in data["contexts"] if item["name"] == current)
(out / "endpoint").write_text(context["endpoint"])
for field, name in (("ca", "ca.pem"), ("crt", "client.pem"), ("key", "client-key.pem")):
    (out / name).write_bytes(base64.b64decode(context[field], validate=True))
PY
chmod 0600 "$operator_dir/client-key.pem"

service_deploy_started="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
service_deploy_status=0
bounded 20s env OVERDRIVE_CONFIG_DIR="$config_dir" \
  "$overdrive_bin" deploy "$example_dir/service.toml" --detach \
  >"$scratch_dir/service-deploy.out" 2>"$scratch_dir/service-deploy.err" || service_deploy_status=$?
service_deploy_ended="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
record_observation service-deploy 'overdrive deploy service.toml --detach' \
  "$service_deploy_status" "$service_deploy_started" "$service_deploy_ended" \
  stdout "$scratch_dir/service-deploy.out" stderr "$scratch_dir/service-deploy.err"
[[ "$service_deploy_status" -eq 0 ]] || exit "$service_deploy_status"
service_deployed=1

deadline=$((SECONDS + 60))
while [[ "$SECONDS" -lt "$deadline" ]]; do
  describe_started="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  describe_status=0
  bounded 10s env OVERDRIVE_CONFIG_DIR="$config_dir" \
    "$overdrive_bin" workload describe api \
    >"$scratch_dir/service-describe.out" 2>"$scratch_dir/service-describe.err" \
    || describe_status=$?
  describe_ended="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  record_observation service-describe 'overdrive workload describe api' \
    "$describe_status" "$describe_started" "$describe_ended" \
    stdout "$scratch_dir/service-describe.out" stderr "$scratch_dir/service-describe.err"
  if [[ "$describe_status" -eq 0 ]] \
    && grep -Fq "Running" "$scratch_dir/service-describe.out"; then
    break
  fi
  sleep 0.25
done
grep -Fq "Running" "$scratch_dir/service-describe.out" || {
  echo "public-ingress example: api Service did not reach Running" >&2
  exit 1
}

route_deploy_started="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
route_deploy_status=0
bounded 20s env OVERDRIVE_CONFIG_DIR="$config_dir" \
  "$overdrive_bin" deploy "$route_spec" --detach \
  >"$scratch_dir/route-deploy.out" 2>"$scratch_dir/route-deploy.err" || route_deploy_status=$?
route_deploy_ended="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
record_observation route-deploy 'overdrive deploy route.toml --detach' \
  "$route_deploy_status" "$route_deploy_started" "$route_deploy_ended" \
  stdout "$scratch_dir/route-deploy.out" stderr "$scratch_dir/route-deploy.err"
[[ "$route_deploy_status" -eq 0 ]] || exit "$route_deploy_status"
route_deployed=1

deadline=$((SECONDS + 60))
while [[ "$SECONDS" -lt "$deadline" ]]; do
  status_started="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  status_exit=0
  operator_curl GET "/v1/gateway/status" \
    >"$scratch_dir/gateway-status.json" 2>"$scratch_dir/gateway-status.err" || status_exit=$?
  status_ended="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  record_observation gateway-status 'operator-mTLS GET /v1/gateway/status' \
    "$status_exit" "$status_started" "$status_ended" \
    response.json "$scratch_dir/gateway-status.json" stderr "$scratch_dir/gateway-status.err"
  if grep -Fq '"listener_bound":true' "$scratch_dir/gateway-status.json"; then
    break
  fi
  sleep 0.25
done
if [[ "$status_exit" -ne 0 ]] \
  || ! grep -Fq '"listener_bound":true' "$scratch_dir/gateway-status.json"; then
  echo "public-ingress example: gateway status did not become active" >&2
  exit 1
fi

if [[ -n "${PIG_PRE_REQUEST_GATE:-}" ]]; then
  : >"${PIG_PRE_REQUEST_GATE}.ready"
  gate_deadline=$((SECONDS + 30))
  while [[ -e "$PIG_PRE_REQUEST_GATE" && "$SECONDS" -lt "$gate_deadline" ]]; do
    sleep 0.1
  done
  [[ ! -e "$PIG_PRE_REQUEST_GATE" ]] || {
    echo "public-ingress example: pre-request observation gate timed out" >&2
    exit 1
  }
fi

deadline=$((SECONDS + 60))
public_ok=0
while [[ "$SECONDS" -lt "$deadline" ]]; do
  public_receipt_offset="$(wc -c <"$serve_log")"
  public_started="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  public_exit=0
  public_status="$(bounded 15s curl --silent --show-error \
      --tlsv1.3 --tls-max 1.3 --http1.1 \
      "${public_trust_args[@]}" \
      --resolve "$hostname:443:$gateway_address" \
      --dump-header "$scratch_dir/public-headers.out" \
      --output "$scratch_dir/public-body.out" \
      --write-out '%{http_code}' \
      "https://$hostname/stream")" || public_exit=$?
  printf '%s\n' "$public_status" >"$scratch_dir/public-status"
  capture_serve_suffix "$public_receipt_offset" "$scratch_dir/public-receipt-suffix.log"
  public_ended="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  record_observation public-request-200 \
    "curl --resolve $hostname:443:$gateway_address https://$hostname/stream" \
    "$public_exit/$public_status" "$public_started" "$public_ended" \
    headers "$scratch_dir/public-headers.out" body "$scratch_dir/public-body.out" \
    serve-log-suffix "$scratch_dir/public-receipt-suffix.log"
  if [[ "$public_exit" -eq 0 && "$public_status" == "200" ]]; then
    public_ok=1
    break
  fi
  sleep 0.25
done
[[ "$public_ok" -eq 1 ]] || {
  echo "public-ingress example: external HTTPS request did not complete" >&2
  exit 1
}

if [[ "${PIG_CAPTURE_FAILURE_MATRIX:-0}" -eq 1 ]]; then
  narrow_started="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  bounded 20s env OVERDRIVE_CONFIG_DIR="$config_dir" \
    "$overdrive_bin" deploy "$narrow_route_spec" --detach \
    >"$scratch_dir/route-narrow-deploy.out" 2>"$scratch_dir/route-narrow-deploy.err"
  narrow_ended="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  record_observation route-narrow-deploy 'overdrive deploy route-narrow.toml --detach' \
    0 "$narrow_started" "$narrow_ended" stdout "$scratch_dir/route-narrow-deploy.out" \
    stderr "$scratch_dir/route-narrow-deploy.err"
  deadline=$((SECONDS + 60))
  while [[ "$SECONDS" -lt "$deadline" ]]; do
    route_miss_receipt_offset="$(wc -c <"$serve_log")"
    miss_started="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    route_miss_exit=0
    route_miss_status="$(bounded 15s curl --silent --show-error \
      --tlsv1.3 --tls-max 1.3 --http1.1 \
      "${public_trust_args[@]}" \
      --resolve "$hostname:443:$gateway_address" \
      --output "$scratch_dir/route-miss-body.out" \
      --write-out '%{http_code}' \
      "https://$hostname/unmatched")" || route_miss_exit=$?
    printf '%s\n' "$route_miss_status" >"$scratch_dir/route-miss-status"
    capture_serve_suffix "$route_miss_receipt_offset" \
      "$scratch_dir/route-miss-receipt-suffix.log"
    miss_ended="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    record_observation public-request-404 \
      "curl --resolve $hostname:443:$gateway_address https://$hostname/unmatched" \
      "$route_miss_exit/$route_miss_status" "$miss_started" "$miss_ended" \
      status "$scratch_dir/route-miss-status" body "$scratch_dir/route-miss-body.out" \
      serve-log-suffix "$scratch_dir/route-miss-receipt-suffix.log"
    [[ "$route_miss_exit" -eq 0 && "$route_miss_status" == "404" ]] && break
    sleep 0.25
  done
  [[ "$route_miss_exit" -eq 0 && "$route_miss_status" == "404" ]] || {
    echo "public-ingress example: unmatched Route did not return 404" >&2
    exit 1
  }

  restore_started="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  bounded 20s env OVERDRIVE_CONFIG_DIR="$config_dir" \
    "$overdrive_bin" deploy "$route_spec" --detach \
    >"$scratch_dir/route-restore.out" 2>"$scratch_dir/route-restore.err"
  restore_ended="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  record_observation route-restore 'overdrive deploy route.toml --detach' \
    0 "$restore_started" "$restore_ended" stdout "$scratch_dir/route-restore.out" \
    stderr "$scratch_dir/route-restore.err"
  deadline=$((SECONDS + 60))
  while [[ "$SECONDS" -lt "$deadline" ]]; do
    upstream_receipt_offset="$(wc -c <"$serve_log")"
    upstream_failure_started="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    upstream_failure_exit=0
    upstream_failure_status="$(bounded 15s curl --silent --show-error \
      --tlsv1.3 --tls-max 1.3 --http1.1 \
      "${public_trust_args[@]}" \
      --resolve "$hostname:443:$gateway_address" \
      --output "$scratch_dir/upstream-failure-body.out" \
      --write-out '%{http_code}' \
      "https://$hostname/abort")" || upstream_failure_exit=$?
    printf '%s\n' "$upstream_failure_status" >"$scratch_dir/upstream-failure-status"
    capture_serve_suffix "$upstream_receipt_offset" \
      "$scratch_dir/upstream-failure-receipt-suffix.log"
    upstream_failure_ended="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    record_observation public-request-502 \
      "curl --resolve $hostname:443:$gateway_address https://$hostname/abort" \
      "$upstream_failure_exit/$upstream_failure_status" \
      "$upstream_failure_started" "$upstream_failure_ended" \
      status "$scratch_dir/upstream-failure-status" body "$scratch_dir/upstream-failure-body.out" \
      serve-log-suffix "$scratch_dir/upstream-failure-receipt-suffix.log"
    [[ "$upstream_failure_exit" -eq 0 && "$upstream_failure_status" == "502" ]] && break
    sleep 0.25
  done
  [[ "$upstream_failure_exit" -eq 0 && "$upstream_failure_status" == "502" ]] || {
    echo "public-ingress example: selected upstream failure did not return 502" >&2
    exit 1
  }
fi

if [[ "${PIG_CAPTURE_NO_BACKEND:-0}" -eq 1 ]]; then
  no_backend_stop_started="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  no_backend_stop_status=0
  bounded 15s env OVERDRIVE_CONFIG_DIR="$config_dir" \
    "$overdrive_bin" job stop api \
    >"$scratch_dir/no-backend-stop.out" 2>"$scratch_dir/no-backend-stop.err" \
    || no_backend_stop_status=$?
  no_backend_stop_ended="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  record_observation no-backend-service-stop 'overdrive job stop api' \
    "$no_backend_stop_status" "$no_backend_stop_started" "$no_backend_stop_ended" \
    stdout "$scratch_dir/no-backend-stop.out" stderr "$scratch_dir/no-backend-stop.err"
  [[ "$no_backend_stop_status" -eq 0 ]] || exit "$no_backend_stop_status"
  service_deployed=0
  deadline=$((SECONDS + 60))
  while [[ "$SECONDS" -lt "$deadline" ]]; do
    no_backend_receipt_offset="$(wc -c <"$serve_log")"
    no_backend_started="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    no_backend_exit=0
    no_backend_status="$(bounded 15s curl --silent --show-error \
      --tlsv1.3 --tls-max 1.3 --http1.1 \
      "${public_trust_args[@]}" \
      --resolve "$hostname:443:$gateway_address" \
      --output "$scratch_dir/no-backend-body.out" \
      --write-out '%{http_code}' \
      "https://$hostname/")" || no_backend_exit=$?
    printf '%s\n' "$no_backend_status" >"$scratch_dir/no-backend-status"
    capture_serve_suffix "$no_backend_receipt_offset" \
      "$scratch_dir/no-backend-receipt-suffix.log"
    no_backend_ended="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    record_observation public-request-503 \
      "curl --resolve $hostname:443:$gateway_address https://$hostname/" \
      "$no_backend_exit/$no_backend_status" "$no_backend_started" "$no_backend_ended" \
      status "$scratch_dir/no-backend-status" body "$scratch_dir/no-backend-body.out" \
      serve-log-suffix "$scratch_dir/no-backend-receipt-suffix.log"
    [[ "$no_backend_exit" -eq 0 && "$no_backend_status" == "503" ]] && break
    sleep 0.25
  done
  [[ "$no_backend_exit" -eq 0 && "$no_backend_status" == "503" ]] || {
    echo "public-ingress example: empty backend set did not return 503" >&2
    exit 1
  }
fi

if [[ -n "${PIG_OBSERVATION_GATE:-}" ]]; then
  : >"${PIG_OBSERVATION_GATE}.ready"
  gate_deadline=$((SECONDS + 30))
  while [[ -e "$PIG_OBSERVATION_GATE" && "$SECONDS" -lt "$gate_deadline" ]]; do
    sleep 0.1
  done
  [[ ! -e "$PIG_OBSERVATION_GATE" ]] || {
    echo "public-ingress example: observation gate timed out" >&2
    exit 1
  }
fi

cat "$scratch_dir/route-deploy.out"
cat "$scratch_dir/gateway-status.json"
cat "$scratch_dir/public-headers.out"
cat "$scratch_dir/public-body.out"
