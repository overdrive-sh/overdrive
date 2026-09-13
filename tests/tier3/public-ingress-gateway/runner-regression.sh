#!/usr/bin/env bash
# Host-safe regression for the Tier-3 cleartext guard.
# CONTRACT_SHAPE: unbounded-preservation.
set -euo pipefail

test_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly test_dir
scratch="$(mktemp -d "${TMPDIR:-/tmp}/pig-runner-regression.XXXXXX")"
readonly scratch
cleanup() { rm -rf -- "$scratch"; }
trap cleanup EXIT HUP INT TERM

mkdir -p "$scratch/bin"
printf 'capture-placeholder\n' >"$scratch/contains-marker.pcap"
# shellcheck disable=SC2016 # these literals are the generated fake's source
printf '%s\n' '#!/usr/bin/env bash' \
  'case "${FAKE_TCPDUMP_MODE:-marker}" in' \
  '  marker) for _ in $(seq 1 20000); do printf "0123456789abcdef"; done; printf "public-ingress-gateway-stream-ok\n" ;;' \
  '  failure) printf "partial decode\n"; exit 42 ;;' \
  '  clean) printf "TLS ciphertext only\n" ;;' \
  'esac' \
  >"$scratch/bin/tcpdump"
chmod 0755 "$scratch/bin/tcpdump"
PATH="$scratch/bin:$PATH"

# shellcheck source=/dev/null
source "$test_dir/cleartext-guard.sh"
marker_status=0
FAKE_TCPDUMP_MODE=marker assert_capture_excludes_marker \
  "$scratch/contains-marker.pcap" public-ingress-gateway-stream-ok regression \
  || marker_status=$?
if [[ "$marker_status" -ne 1 ]]; then
  echo "runner regression: marker-bearing capture was incorrectly accepted" >&2
  exit 1
fi

failure_status=0
FAKE_TCPDUMP_MODE=failure assert_capture_excludes_marker \
  "$scratch/contains-marker.pcap" public-ingress-gateway-stream-ok regression \
  || failure_status=$?
if [[ "$failure_status" -ne 2 ]]; then
  echo "runner regression: tcpdump failure was not distinguished" >&2
  exit 1
fi

FAKE_TCPDUMP_MODE=clean assert_capture_excludes_marker \
  "$scratch/contains-marker.pcap" public-ingress-gateway-stream-ok regression

# A listener-readiness probe may append a validated receipt occurrence inside
# any request suffix. Cardinality is over operation=request only: in particular,
# the 404 suffix below contains one probe and zero request occurrences.
mkdir -p "$scratch/connector-trace" "$scratch/observations"
printf 'getsockopt(SOL_SOCKET, SO_COOKIE) = 7\n' \
  >"$scratch/connector-trace/serve.1"
printf 'spiffe_id: spiffe://overdrive.local/workload/api/alloc/api-0\n' \
  >"$scratch/service-describe.out"
for status in 200 404 502 503; do
  record="$scratch/observations/0001-public-request-$status"
  mkdir -p "$record"
  printf 'status=0/%s\n' "$status" >"$record/metadata"
  printf '%s\n' \
    'event="gateway.upstream.receipt.validated" operation="probe" receipt_outcome="no_backend" socket_cookie=41 service_key_vip="127.0.0.1" service_key_port=8080 service_key_protocol="tcp" validated gateway upstream BPF selection receipt' \
    >"$record/serve-log-suffix"
  case "$status" in
    200|502)
      printf '%s\n' \
        'event="gateway.upstream.receipt.validated" operation="request" receipt_outcome="selected" socket_cookie=42 service_key_vip="127.0.0.1" service_key_port=8080 service_key_protocol="tcp" backend_id=9 expected_peer_spiffe_id="spiffe://overdrive.local/workload/api/alloc/api-0" validated gateway upstream BPF selection receipt' \
        >>"$record/serve-log-suffix"
      ;;
    503)
      printf '%s\n' \
        'event="gateway.upstream.receipt.validated" operation="request" receipt_outcome="no_backend" socket_cookie=43 service_key_vip="127.0.0.1" service_key_port=8080 service_key_protocol="tcp" validated gateway upstream BPF selection receipt' \
        >>"$record/serve-log-suffix"
      ;;
  esac
done
python3 "$test_dir/receipt-oracle.py" "$scratch/connector-trace" \
  "$scratch/observations" "$scratch/service-describe.out"

echo "runner regression PASS: cleartext guard and request-only receipt cardinality are exact"
