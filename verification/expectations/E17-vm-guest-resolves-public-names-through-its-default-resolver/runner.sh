#!/usr/bin/env bash
# E17 capture: one `overdrive serve`, one `overdrive deploy` of herdr-box.toml,
# the guest console and the operator's view recorded verbatim.
#
# Black-box: drives the built `overdrive` binary only. Installs no resolver,
# route or rule in the guest or on the host; reads the host's resolver file and
# never writes it. Every observation is appended to $EVIDENCE_DIR/capture.log
# (append-only, timestamped) or written to a uniquely named file.
#
# Prerequisites on the metal box (separate commands):
#   cargo xtask metal run -- cargo xtask guest-kit-prepare
# Run:
#   cargo xtask metal run --exclusive -- \
#     verification/harness/run-expectation.sh \
#     E17-vm-guest-resolves-public-names-through-its-default-resolver
set -uo pipefail

: "${EVIDENCE_DIR:?run through verification/harness/run-expectation.sh}"
: "${EXPECTATION_DIR:?}"
: "${REPO_ROOT:?}"
readonly OUTPUT_ROOT="/srv/vm/overdrive-testing/svm-e08"
readonly CONFIG_DIR="$OUTPUT_ROOT/config"
readonly DATA_DIR="$OUTPUT_ROOT/data"
readonly CREDS_DIR="$OUTPUT_ROOT/credentials"
readonly BIN="$REPO_ROOT/target/debug/overdrive"
readonly BIND="127.0.0.1:7644"
readonly PREPARE="$EXPECTATION_DIR/prepare.sh"
readonly SPEC="$EXPECTATION_DIR/herdr-box.toml"
readonly JOB_ID="herdr-box"
readonly VM_RUN_ROOT="/run/overdrive/vm"
RUN_TS="$(date -u +%Y%m%dT%H%M%SZ)"
readonly RUN_TS
readonly LOG="$EVIDENCE_DIR/capture.log"
readonly SNAP_DIR="$EVIDENCE_DIR/console-snapshots-$RUN_TS"
readonly GUEST_PEER_HOST="$REPO_ROOT/target/x86_64-unknown-linux-musl/release/overdrive-guest-peer"

SERVE_PID=""
PREPARED=0

log() { printf '%s %s\n' "$(date -u +%Y-%m-%dT%H:%M:%S.%3NZ)" "$*" >>"$LOG"; printf '%s\n' "$*"; }
die() { log "FATAL: $*"; exit 1; }
bounded() { local d="$1"; shift; timeout --foreground --signal=TERM --kill-after=5s "$d" "$@"; }
cli() { bounded 30s env OVERDRIVE_CONFIG_DIR="$CONFIG_DIR" "$BIN" "$@"; }

# Run a command, appending its command line, stdout+stderr and exit status.
record() {
  local label="$1"; shift
  log "[$label] \$ $*"
  local out rc
  out="$("$@" 2>&1)"; rc=$?
  printf '%s\n' "$out" | sed 's/^/    /' >>"$LOG"
  printf '%s\n' "$out"
  log "[$label] exit=$rc"
  return "$rc"
}

alloc_state() {
  awk '/^Alloc[[:space:]]+State[[:space:]]+/ { t = 1; next } t && $1 ~ /^alloc-/ { print $2; exit }'
}

start_serve() {
  setsid keyctl session - env OVERDRIVE_CONFIG_DIR="$CONFIG_DIR" \
    CREDENTIALS_DIRECTORY="$CREDS_DIR" "$BIN" serve --bind "$BIND" --data-dir "$DATA_DIR" \
    >"$EVIDENCE_DIR/serve-$RUN_TS.log" 2>&1 &
  SERVE_PID=$!
  log "serve started pid=$SERVE_PID bind=$BIND"
  local attempts=60
  while [[ "$attempts" -gt 0 ]]; do
    [[ -f "$CONFIG_DIR/.overdrive/config" ]] && kill -0 "$SERVE_PID" 2>/dev/null && return 0
    kill -0 "$SERVE_PID" 2>/dev/null || break
    sleep 0.5; attempts=$((attempts - 1))
  done
  die "serve did not become ready within 30 seconds (see serve-$RUN_TS.log)"
}

stop_serve() {
  [[ -n "$SERVE_PID" ]] || return 0
  kill -- "-$SERVE_PID" 2>/dev/null || kill "$SERVE_PID" 2>/dev/null || true
  local attempts=50
  while kill -0 "$SERVE_PID" 2>/dev/null && [[ "$attempts" -gt 0 ]]; do sleep 0.1; attempts=$((attempts - 1)); done
  kill -KILL -- "-$SERVE_PID" 2>/dev/null || true
  wait "$SERVE_PID" 2>/dev/null || true
  log "serve stopped"
  SERVE_PID=""
}

cleanup() {
  local rc=$?
  trap - EXIT HUP INT TERM
  local failed=0
  if [[ -n "$SERVE_PID" ]]; then cli job stop "$JOB_ID" >/dev/null 2>&1 || true; fi
  stop_serve || failed=1
  if [[ "$PREPARED" -eq 1 ]]; then bounded 45s "$PREPARE" cleanup || failed=1; log "cleanup done failed=$failed"; fi
  [[ "$rc" -ne 0 || "$failed" -eq 0 ]] || exit 1
  exit "$rc"
}

# Copy every VM console that has grown to a new uniquely named file (never
# overwriting an earlier snapshot).
snapshot_consoles() {
  local f alloc size last
  for f in "$VM_RUN_ROOT"/*/console.log; do
    [[ -f "$f" ]] || continue
    alloc="$(basename "$(dirname "$f")")"
    size="$(stat -c %s "$f")"
    last="$(cat "$SNAP_DIR/.last-$alloc" 2>/dev/null || echo 0)"
    if [[ "$size" -gt "$last" ]]; then
      cp "$f" "$SNAP_DIR/$alloc.$(date -u +%H%M%S.%3N).console"
      echo "$size" >"$SNAP_DIR/.last-$alloc"
      log "console snapshot alloc=$alloc bytes=$size"
    fi
  done
}

# --- preconditions and pins ---------------------------------------------------
mkdir -p "$SNAP_DIR"
log "=== E17 capture $RUN_TS ==="
log "source sha=${EXPECTATION_SOURCE_SHA:-unknown} dirty=${EXPECTATION_SOURCE_DIRTY:-unknown}"
[[ "$(uname -s)" == "Linux" && "$(uname -m)" == "x86_64" ]] || die "requires native x86_64 Linux"
[[ "$(id -u)" -eq 0 ]] || die "requires root (run through cargo xtask metal run --exclusive --)"
[[ -c /dev/kvm ]] || die "/dev/kvm is not a character device"
record host-uname uname -a
record host-kernel-release uname -r
record host-virt systemd-detect-virt
record host-resolv-conf cat /etc/resolv.conf
if command -v resolvectl >/dev/null 2>&1; then record host-resolvectl resolvectl status; fi
[[ ! -e "$OUTPUT_ROOT" ]] || die "refusing to overwrite pre-existing materialization: $OUTPUT_ROOT"
[[ -x "$GUEST_PEER_HOST" ]] \
  || die "guest kit not prepared (cargo xtask metal run -- cargo xtask guest-kit-prepare)"
record guest-peer-sha256 sha256sum "$GUEST_PEER_HOST"
record spec cat "$SPEC"
record host-nft-before nft list ruleset
record host-ip-rule-before ip rule show
record host-ip-route-before ip route show table all

trap cleanup EXIT
trap 'exit 130' HUP INT TERM
record product-build bounded 900s cargo build -p overdrive-cli --bin overdrive || die "product build failed"
record binary-sha256 sha256sum "$BIN"
PREPARED=1
record prepare bounded 300s "$PREPARE" prepare || die "prepare failed"
GUEST_CAT="$(cat "$OUTPUT_ROOT/guest-cat-path" 2>/dev/null || true)"
log "guest image cat=${GUEST_CAT:-none}"
start_serve

# --- deploy and observe -------------------------------------------------------
record deploy cli deploy --detach "$SPEC" || die "deploy failed"
deadline=$((SECONDS + 150))
state=""
out=""
while [[ "$SECONDS" -lt "$deadline" ]]; do
  snapshot_consoles
  out="$(bounded 10s env OVERDRIVE_CONFIG_DIR="$CONFIG_DIR" "$BIN" workload describe "$JOB_ID" 2>&1 || true)"
  state="$(alloc_state <<<"$out")"
  log "poll describe state=${state:-none}"
  case "$state" in Terminated|Failed) break ;; esac
  sleep 1
done
snapshot_consoles
printf '%s\n' "$out" >"$EVIDENCE_DIR/describe-$RUN_TS.txt"
log "final alloc state=${state:-none}"

# --- the guest's resolver file (only if its image carries a cat) --------------
if [[ -n "$GUEST_CAT" ]]; then
  second="$SNAP_DIR/herdr-resolv.toml"
  cat >"$second" <<TOML
[job]
id = "herdr-resolv"

[vm]
command = "/opt/overdrive/examples/svm/overdrive-guest-peer"
args = ["run-with-modules", "--", "$GUEST_CAT", "/etc/resolv.conf"]
kernel = "$OUTPUT_ROOT/kernel"
rootfs = "$OUTPUT_ROOT/rootfs.ext4"

[resources]
cpu = "500m"
memory = "128MiB"
TOML
  record deploy-resolv cli deploy --detach "$second" || log "second job deploy failed"
  deadline=$((SECONDS + 90))
  out2=""
  while [[ "$SECONDS" -lt "$deadline" ]]; do
    snapshot_consoles
    out2="$(bounded 10s env OVERDRIVE_CONFIG_DIR="$CONFIG_DIR" "$BIN" workload describe herdr-resolv 2>&1 || true)"
    case "$(alloc_state <<<"$out2")" in Terminated|Failed) break ;; esac
    sleep 1
  done
  snapshot_consoles
  printf '%s\n' "$out2" >"$EVIDENCE_DIR/describe-resolv-$RUN_TS.txt"
fi

record host-nft-after nft list ruleset
record host-ip-rule-after ip rule show
record host-ip-route-after ip route show table all
record host-resolv-conf-after cat /etc/resolv.conf
log "=== capture ends; consoles in $SNAP_DIR ==="
[[ "$state" == "Terminated" ]]
