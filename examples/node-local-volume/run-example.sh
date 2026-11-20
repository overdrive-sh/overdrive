#!/usr/bin/env bash
# Run the node-local volume example journeys on qualified native metal.
#
#   run-example.sh check-source          validate the checked-in sources (host-safe)
#   run-example.sh run restart-survives  deploy, restart, stop and resume; read the notes back
#   run-example.sh run oversized         deploy the volume larger than the pool
#   run-example.sh prepare | serve | cleanup
#                                        the individual steps, for a journey that spans a
#                                        node reboot (README: "Across a node reboot")
set -euo pipefail

EXAMPLE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly EXAMPLE_DIR
readonly PREPARE="$EXAMPLE_DIR/prepare.sh"
readonly OUTPUT_ROOT="/srv/vm/overdrive-testing/svm-e08"
readonly CONFIG_DIR="$OUTPUT_ROOT/config"
readonly DATA_DIR="$OUTPUT_ROOT/data"
readonly CREDS_DIR="$OUTPUT_ROOT/credentials"
BIN="$(cd "$EXAMPLE_DIR/../.." && pwd)/target/debug/overdrive"
readonly BIN
readonly BIND="127.0.0.1:7644"
readonly SERVICE_ID="volume-notes"
readonly OVERSIZED_ID="volume-oversized"
readonly POOL_VG="overdrive"

SERVE_PID=""
PREPARED=0
DEPLOYED_IDS=()

die() {
  echo "node-local-volume run: $*" >&2
  exit 1
}

bounded() {
  local duration="$1"
  shift
  timeout --foreground --signal=TERM --kill-after=5s "$duration" "$@"
}

cli() {
  bounded 30s env OVERDRIVE_CONFIG_DIR="$CONFIG_DIR" "$BIN" "$@"
}

require_native_metal() {
  [[ "$(uname -s)" == "Linux" && "$(uname -m)" == "x86_64" ]] \
    || die "runtime is restricted to native x86_64 Linux metal"
  [[ "$(id -u)" -eq 0 ]] || die "run through cargo xtask metal run -- (root context)"
  [[ -c /dev/kvm ]] || die "/dev/kvm is not an accessible character device"
  local command
  for command in awk cloud-hypervisor dd grep keyctl lvs setsid sort timeout tr; do
    command -v "$command" >/dev/null 2>&1 || die "required command is unavailable: $command"
  done
  lvs --noheadings "$POOL_VG" >/dev/null 2>&1 \
    || die "this node has no volume pool (volume group $POOL_VG); see the README prerequisites"
}

pool_volumes() {
  lvs --noheadings -o lv_path "$POOL_VG" | sed 's/^ *//' | sort
}

alloc_id() {
  awk '/^Alloc[[:space:]]+State[[:space:]]+/ { t = 1; next }
       t && $1 ~ /^alloc-/ { print $1; exit }'
}

alloc_state() {
  awk '/^Alloc[[:space:]]+State[[:space:]]+/ { t = 1; next }
       t && $1 ~ /^alloc-/ { print $2; exit }'
}

describe() {
  bounded 10s env OVERDRIVE_CONFIG_DIR="$CONFIG_DIR" "$BIN" workload describe "$1" 2>&1 || true
}

# Wait until the workload's current allocation is Running, its readiness probe
# passes (the guest read every earlier note back and flushed a new one), and
# its allocation id is not $2 (the predecessor, or empty for a first launch).
wait_ready() {
  local id="$1" previous="${2:-}" out current
  local deadline=$((SECONDS + 150))
  while [[ "$SECONDS" -lt "$deadline" ]]; do
    out="$(describe "$id")"
    current="$(alloc_id <<<"$out")"
    if [[ "$(alloc_state <<<"$out")" == "Running" && -n "$current" && "$current" != "$previous" ]] \
      && grep -Eqi 'readiness.*http.*18080.*last=pass' <<<"$out"; then
      printf '%s\n' "$out"
      return 0
    fi
    sleep 1
  done
  printf '%s\n' "$out" >&2
  return 1
}

wait_terminated() {
  local id="$1" out
  local deadline=$((SECONDS + 60))
  while [[ "$SECONDS" -lt "$deadline" ]]; do
    out="$(describe "$id")"
    [[ "$(alloc_state <<<"$out")" == "Terminated" ]] && return 0
    sleep 1
  done
  printf '%s\n' "$out" >&2
  return 1
}

# Read the notes straight from the volume's device node, with direct I/O, while
# no VMM holds it. Prints one note per line.
read_notes_from_host() {
  local device="$1"
  bounded 15s dd if="$device" iflag=direct bs=512 count=16 status=none | tr -d '\000'
}

expect_notes() {
  local notes="$1" count="$2" index
  [[ "$(grep -c '^OVD-VOLUME-NOTE ' <<<"$notes")" -eq "$count" ]] \
    || die "expected $count note(s) on the volume, read: $notes"
  for ((index = 1; index <= count; index++)); do
    grep -Fq "seq=$index prior_verified=$((index - 1)) " <<<"$notes" \
      || die "note $index is missing or did not verify its $((index - 1)) predecessor(s): $notes"
  done
}

start_serve() {
  [[ -x "$BIN" ]] || die "default-feature product binary was not built: $BIN"
  setsid keyctl session - env OVERDRIVE_CONFIG_DIR="$CONFIG_DIR" \
    CREDENTIALS_DIRECTORY="$CREDS_DIR" "$BIN" serve --bind "$BIND" --data-dir "$DATA_DIR" \
    >"$OUTPUT_ROOT/serve.log" 2>&1 &
  SERVE_PID=$!
  local attempts=60
  while [[ "$attempts" -gt 0 ]]; do
    [[ -f "$CONFIG_DIR/.overdrive/config" ]] && kill -0 "$SERVE_PID" 2>/dev/null && return 0
    kill -0 "$SERVE_PID" 2>/dev/null || break
    sleep 0.5
    attempts=$((attempts - 1))
  done
  cat "$OUTPUT_ROOT/serve.log" >&2
  die "serve did not become ready within 30 seconds"
}

stop_serve() {
  [[ -n "$SERVE_PID" ]] || return 0
  kill -- "-$SERVE_PID" 2>/dev/null || kill "$SERVE_PID" 2>/dev/null || true
  local attempts=50
  while kill -0 "$SERVE_PID" 2>/dev/null && [[ "$attempts" -gt 0 ]]; do
    sleep 0.1
    attempts=$((attempts - 1))
  done
  kill -KILL -- "-$SERVE_PID" 2>/dev/null || true
  wait "$SERVE_PID" 2>/dev/null || true
  SERVE_PID=""
}

# The volume itself stays in the pool: nothing in this slice deletes it. Only
# the processes and the private materialization are removed.
cleanup() {
  local incoming_rc=$?
  trap - EXIT HUP INT TERM
  local failed=0 id
  if [[ -n "$SERVE_PID" ]]; then
    for id in "${DEPLOYED_IDS[@]}"; do
      cli job stop "$id" >/dev/null 2>&1 || true
    done
  fi
  stop_serve || failed=1
  [[ "$PREPARED" -eq 0 ]] || bounded 45s "$PREPARE" cleanup || failed=1
  [[ "$incoming_rc" -ne 0 || "$failed" -eq 0 ]] || exit 1
  exit "$incoming_rc"
}

begin() {
  require_native_metal
  "$PREPARE" check-source
  [[ ! -e "$OUTPUT_ROOT" ]] || die "refusing to overwrite pre-existing materialization: $OUTPUT_ROOT"
  trap cleanup EXIT
  trap 'exit 130' HUP INT TERM
  bounded 600s cargo build -p overdrive-cli --bin overdrive
  PREPARED=1
  bounded 300s "$PREPARE" prepare
  start_serve
}

run_restart_survives() {
  begin
  local before after device out first second third
  before="$(pool_volumes)"

  echo '--- deploy: a 1GiB volume, first start ---'
  DEPLOYED_IDS+=("$SERVICE_ID")
  cli deploy --detach "$EXAMPLE_DIR/volume-service.toml"
  out="$(wait_ready "$SERVICE_ID")" || die "the volume Service never became ready"
  printf '%s\n' "$out"
  first="$(alloc_id <<<"$out")"
  grep -Eq 'Volume +1GiB \(guest disk: virtio serial ovd-volume\)' <<<"$out" \
    || die "describe did not show the declared volume"
  after="$(pool_volumes)"
  device="$(comm -13 <(printf '%s\n' "$before") <(printf '%s\n' "$after") | sed '/^$/d')"
  [[ "$(wc -l <<<"$device")" -eq 1 && -n "$device" ]] \
    || die "expected exactly one new logical volume in $POOL_VG, saw: $device"
  echo "volume device: $device"

  echo '--- workload restart: a replacement attaches the same volume ---'
  cli workload restart "$SERVICE_ID"
  out="$(wait_ready "$SERVICE_ID" "$first")" || die "the replacement never became ready"
  printf '%s\n' "$out"
  second="$(alloc_id <<<"$out")"

  echo '--- stop, then read the notes from the host (no VMM holds the volume) ---'
  cli job stop "$SERVICE_ID"
  wait_terminated "$SERVICE_ID" || die "the Service did not reach Terminated"
  out="$(read_notes_from_host "$device")"
  printf '%s\n' "$out"
  expect_notes "$out" 2

  echo '--- workload restart of the stopped Service: resume reattaches the volume ---'
  cli workload restart "$SERVICE_ID"
  out="$(wait_ready "$SERVICE_ID" "$second")" || die "the resumed Service never became ready"
  printf '%s\n' "$out"
  third="$(alloc_id <<<"$out")"
  cli job stop "$SERVICE_ID"
  wait_terminated "$SERVICE_ID" || die "the Service did not reach Terminated"
  out="$(read_notes_from_host "$device")"
  printf '%s\n' "$out"
  expect_notes "$out" 3

  [[ "$(pool_volumes | wc -l)" -eq "$(($(printf '%s\n' "$before" | sed '/^$/d' | wc -l) + 1))" ]] \
    || die "the pool holds a different number of volumes than the starting set plus one"
  [[ "$first" != "$second" && "$second" != "$third" ]] || die "an allocation was reused"
  echo 'node-local-volume PASS: three starts, three notes, each start read every earlier note back'
}

run_oversized() {
  begin
  local before out=""
  before="$(pool_volumes)"
  DEPLOYED_IDS+=("$OVERSIZED_ID")
  cli deploy --detach "$EXAMPLE_DIR/volume-oversized.toml"
  local deadline=$((SECONDS + 90))
  while [[ "$SECONDS" -lt "$deadline" ]]; do
    out="$(describe "$OVERSIZED_ID")"
    grep -Eqi 'insufficient' <<<"$out" && break
    sleep 1
  done
  printf '%s\n' "$out"
  grep -Eqi 'insufficient' <<<"$out" || die "describe did not show a typed insufficient-space reason"
  [[ "$(pool_volumes)" == "$before" ]] || die "the oversized deploy changed the pool"
  echo 'node-local-volume PASS: the oversized volume is refused with a typed reason and creates nothing'
}

case "${1:-}" in
  check-source) "$PREPARE" check-source ;;
  prepare)
    require_native_metal
    "$PREPARE" prepare
    ;;
  serve)
    require_native_metal
    exec setsid keyctl session - env OVERDRIVE_CONFIG_DIR="$CONFIG_DIR" \
      CREDENTIALS_DIRECTORY="$CREDS_DIR" "$BIN" serve --bind "$BIND" --data-dir "$DATA_DIR"
    ;;
  cleanup) "$PREPARE" cleanup ;;
  run)
    case "${2:-}" in
      restart-survives) run_restart_survives ;;
      oversized) run_oversized ;;
      *) die 'usage: run-example.sh run restart-survives|oversized' ;;
    esac
    ;;
  *) die 'usage: run-example.sh check-source|prepare|serve|cleanup|run restart-survives|oversized' ;;
esac
