# shellcheck shell=bash
# O08 — refusal text and describe read-back for human-readable `[resources]`.
#
# EXECUTION SUBSTRATE: native-metal. This runner executes ON the metal box,
# started by
#   cargo xtask metal run --exclusive -- env ... \
#     verification/harness/run-expectation.sh O08
# It does not invoke `cargo xtask metal run` itself.
#
# BLACK-BOX ONLY. The surface is the BUILT `overdrive` binary (`serve`,
# `deploy`, `workload describe`, `job stop`) and what the kernel exposes
# (/proc, cgroupfs, /run, ip). No `overdrive-*` crate is imported or linked.
# The binary is built with DEFAULT features (the production composition).
#
# NOT A TEST, NOT A MATRIX. Exactly these contrasting cases, run once:
#   accepted : cpu = "500m"  memory = "128MiB"      (dev-box)
#   accepted : cpu = "0.5"   memory = "1024MiB"     (dev-box-gib)
#   refused  : memory = 134217728                   (bare number)
#   refused  : memory = "128MB"
#   refused  : cpu    = "500M"
#   refused  : memory = "1000B"
#   refused  : the removed `memory_bytes` key
# The runner records verbatim stdout, stderr and exit status of every command
# and does NOT judge them: whether a refusal "names the remedy" is a
# reviewer's call. Each command writes its own files (<label>.stdout /
# .stderr / .meta) and every step is appended to trace.log; nothing is
# overwritten or truncated.
set -uo pipefail

: "${EVIDENCE_DIR:?harness must export EVIDENCE_DIR}"
: "${REPO_ROOT:?harness must export REPO_ROOT}"
cd "$REPO_ROOT" || exit 1
EVID="$EVIDENCE_DIR"
mkdir -p "$EVID"

STAGING="/srv/vm/overdrive-testing"
KERNEL="$STAGING/kernel"
ROOTFS_MASTER="$STAGING/rootfs.ext4"
BIND="127.0.0.1:7648"
VM_RUN_ROOT="/run/overdrive/vm"
WORKLOADS_SLICE="/sys/fs/cgroup/overdrive.slice/workloads.slice"

CFG_DIR=""; DATA_DIR=""; CREDS_DIR=""; WORK=""; LOOP=""; MNT=""; SERVE_PID=""

trace() { echo "$(date -u +%Y-%m-%dT%H:%M:%SZ) $*" >> "$EVID/trace.log"; echo "  $*"; }

# ---- probes (only /proc, cgroupfs, /run, ip) -------------------------------
probe_ch_pids() {
  for p in /proc/[0-9]*; do
    [ -r "$p/cmdline" ] || continue
    argv0="$(tr '\0' '\n' < "$p/cmdline" 2>/dev/null | head -1)"
    case "$(basename "$argv0" 2>/dev/null)" in cloud-hypervisor) basename "$p" ;; esac
  done | sort -n
}
probe_scopes()   { ls "$WORKLOADS_SLICE" 2>/dev/null | grep '^alloc-' | sort || true; }
probe_run_dirs() { ls "$VM_RUN_ROOT" 2>/dev/null | sort || true; }
probe_links()    { ip -br link show 2>/dev/null | awk '{print $1}' | sort; }
new_only() { comm -13 <(printf '%s\n' "$1" | sed '/^$/d') <(printf '%s\n' "$2" | sed '/^$/d'); }

BEFORE_CH="$(probe_ch_pids)"; BEFORE_SCOPES="$(probe_scopes)"
BEFORE_RUN_DIRS="$(probe_run_dirs)"; BEFORE_LINKS="$(probe_links)"

# Delta-scoped teardown: reap only what appeared during this run.
sweep() {
  [ -n "$SERVE_PID" ] && kill "$SERVE_PID" 2>/dev/null
  for pid in $(new_only "$BEFORE_CH" "$(probe_ch_pids)"); do
    echo "sweep: killing hypervisor pid $pid started by this run"; kill -KILL "$pid" 2>/dev/null
  done
  for d in $(new_only "$BEFORE_SCOPES" "$(probe_scopes)"); do
    echo "sweep: reaping cgroup scope $d created by this run"
    echo 1 > "$WORKLOADS_SLICE/$d/cgroup.kill" 2>/dev/null; rmdir "$WORKLOADS_SLICE/$d" 2>/dev/null
  done
  for d in $(new_only "$BEFORE_RUN_DIRS" "$(probe_run_dirs)"); do
    echo "sweep: removing VM run dir $VM_RUN_ROOT/$d created by this run"; rm -rf "${VM_RUN_ROOT:?}/$d" 2>/dev/null
  done
  for i in $(new_only "$BEFORE_LINKS" "$(probe_links)"); do
    case "$i" in
      ovd-*) echo "sweep: deleting link $i created by this run"; ip link del "$i" 2>/dev/null ;;
      *)     echo "sweep: LEAVING unexpected new link $i" ;;
    esac
  done
  [ -n "$MNT" ] && mountpoint -q "$MNT" && umount "$MNT" 2>/dev/null
  [ -n "$LOOP" ] && losetup -d "$LOOP" 2>/dev/null
  [ -n "$WORK" ] && rm -rf "$WORK" 2>/dev/null
  [ -n "$CFG_DIR" ] && rm -rf "$CFG_DIR" 2>/dev/null
  [ -n "$DATA_DIR" ] && rm -rf "$DATA_DIR" 2>/dev/null
  [ -n "$CREDS_DIR" ] && rm -rf "$CREDS_DIR" 2>/dev/null
  command -v keyctl >/dev/null 2>&1 && keyctl purge user "overdrive:ca:kek:overdrive-ca-root" 2>/dev/null
  return 0
}
on_exit() {
  ts="$(date -u +%Y%m%dT%H%M%SZ)"
  sweep > "$EVID/teardown_sweep.$ts.txt" 2>&1
  {
    echo "leaked_hypervisors=$(new_only "$BEFORE_CH" "$(probe_ch_pids)" | grep -c .)"
    echo "leaked_scopes=$(new_only "$BEFORE_SCOPES" "$(probe_scopes)" | grep -c .)"
    echo "leaked_run_dirs=$(new_only "$BEFORE_RUN_DIRS" "$(probe_run_dirs)" | grep -c .)"
  } > "$EVID/leak_verdict.$ts.txt" 2>&1
  return 0
}
trap on_exit EXIT

# run_case <label> <cmd...>: verbatim stdout, stderr and exit status, one file set per command.
run_case() {
  local label="$1"; shift
  local rc=0
  {
    echo "command: $*"
    echo "started: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  } > "$EVID/$label.meta"
  "$@" > "$EVID/$label.stdout" 2> "$EVID/$label.stderr" || rc=$?
  {
    echo "finished: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "exit: $rc"
  } >> "$EVID/$label.meta"
  trace "case $label exit=$rc"
  return 0
}

# ---- substrate record ------------------------------------------------------
{
  echo "arch:      $(uname -m)"
  echo "kernel:    $(uname -sr)"
  echo "virt:      $(systemd-detect-virt 2>&1)"
  if [ -c /dev/kvm ]; then echo "kvm:       /dev/kvm present"; else echo "kvm:       ABSENT"; fi
  echo "cloud-hypervisor: $(command -v cloud-hypervisor || echo ABSENT)"
  echo "staged kernel: $([ -f "$KERNEL" ] && echo present || echo ABSENT)"
  echo "staged rootfs: $([ -f "$ROOTFS_MASTER" ] && echo present || echo ABSENT)"
  echo "head sha:  ${EXPECTATION_SOURCE_SHA:-unknown} (source dirty=${EXPECTATION_SOURCE_DIRTY:-unknown})"
} > "$EVID/substrate.txt" 2>&1
trace "substrate recorded"

if [ ! -c /dev/kvm ] || ! command -v cloud-hypervisor >/dev/null 2>&1 \
   || [ ! -f "$KERNEL" ] || [ ! -f "$ROOTFS_MASTER" ]; then
  trace "CAPABILITY_MISSING — see substrate.txt"; exit 1
fi

# ---- build the PRODUCTION binary (default features) -----------------------
trace "building overdrive (default features)"
if ! cargo build -p overdrive-cli --bin overdrive > "$EVID/build.log" 2>&1; then
  trace "BUILD_FAILED — see build.log"; exit 1
fi
BIN="${CARGO_TARGET_DIR:-$PWD/target}/debug/overdrive"
[ -x "$BIN" ] || { trace "BIN_MISSING $BIN"; exit 1; }
{ echo "path: $BIN"; ls -l "$BIN"; "$BIN" --version 2>&1; } > "$EVID/binary_under_test.txt" 2>&1

# ---- private rootfs clone with a long-lived guest command (as E06) --------
WORK="$(mktemp -d "$STAGING/o08-XXXXXX")"; MNT="$WORK/mnt"; mkdir -p "$MNT"
cp --reflink=auto "$ROOTFS_MASTER" "$WORK/rootfs.ext4" 2>> "$EVID/rootfs_staging.log" || { trace "ROOTFS_CLONE_FAILED"; exit 1; }
LOOP="$(losetup --find --show "$WORK/rootfs.ext4" 2>> "$EVID/rootfs_staging.log")"
[ -n "$LOOP" ] || { trace "LOSETUP_FAILED"; exit 1; }
mount "$LOOP" "$MNT" 2>> "$EVID/rootfs_staging.log" || { trace "MOUNT_FAILED"; exit 1; }
cp /usr/bin/busybox "$MNT/sbin/busybox" 2>> "$EVID/rootfs_staging.log"
chmod 0755 "$MNT/sbin/busybox"
umount "$MNT" 2>> "$EVID/rootfs_staging.log"; losetup -d "$LOOP" 2>> "$EVID/rootfs_staging.log"
LOOP=""; MNT=""

# ---- spec writer: write_spec <id> <resources-lines> -------------------------
write_spec() {
  local id="$1" res="$2"
  cat > "$WORK/$id.toml" <<SPEC_EOF
[job]
id = "$id"

[vm]
command = "/sbin/busybox"
args = ["sleep", "3600"]
kernel = "$KERNEL"
rootfs = "$WORK/rootfs.ext4"

[resources]
$res
SPEC_EOF
  cp "$WORK/$id.toml" "$EVID/spec_$id.toml"
}
write_spec dev-box                $'cpu = "500m"\nmemory = "128MiB"'
write_spec dev-box-gib            $'cpu = "0.5"\nmemory = "1024MiB"'
write_spec refused-bare-number    $'cpu = "500m"\nmemory = 134217728'
write_spec refused-128mb          $'cpu = "500m"\nmemory = "128MB"'
write_spec refused-cpu-capital-m  $'cpu = "500M"\nmemory = "128MiB"'
write_spec refused-1000b          $'cpu = "500m"\nmemory = "1000B"'
write_spec refused-memory-bytes   $'cpu = "500m"\nmemory_bytes = 134217728'

# ---- serve (production KEK delivery, as E06 phase B) -----------------------
CFG_DIR="$(mktemp -d /tmp/od-o08-cfg.XXXXXX)"
DATA_DIR="$(mktemp -d "$STAGING/od-o08-data.XXXXXX")"; chmod 0711 "$DATA_DIR"
CREDS_DIR="$(mktemp -d /tmp/od-o08-creds.XXXXXX)"; chmod 0700 "$CREDS_DIR"
head -c 32 /dev/urandom > "$CREDS_DIR/overdrive-ca-root"; chmod 0400 "$CREDS_DIR/overdrive-ca-root"
trace "starting serve bind=$BIND"
OVERDRIVE_CONFIG_DIR="$CFG_DIR" CREDENTIALS_DIRECTORY="$CREDS_DIR" \
  "$BIN" serve --bind "$BIND" --data-dir "$DATA_DIR" > "$EVID/serve.log" 2>&1 &
SERVE_PID=$!
ready=0
for _ in $(seq 1 60); do
  [ -f "$CFG_DIR/.overdrive/config" ] && { ready=1; break; }
  kill -0 "$SERVE_PID" 2>/dev/null || break
  sleep 0.5
done
if [ "$ready" -ne 1 ]; then trace "SERVE_NOT_READY — see serve.log"; exit 1; fi
trace "serve ready pid=$SERVE_PID"

od() { OVERDRIVE_CONFIG_DIR="$CFG_DIR" "$BIN" "$@"; }

# ---- deploy: accepted first, then the refused contrasts ---------------------
for id in dev-box dev-box-gib refused-bare-number refused-128mb refused-cpu-capital-m refused-1000b refused-memory-bytes; do
  run_case "deploy_$id" od deploy "$WORK/$id.toml"
done

# ---- describe the accepted workloads ----------------------------------------
# Poll until the Memory: line renders (it needs a first allocation row); every
# observation is kept as its own file, none overwritten.
for id in dev-box dev-box-gib; do
  n=0
  for _ in $(seq 1 45); do
    n=$((n+1)); p="$EVID/describe_${id}_poll$(printf %02d "$n")"
    rc=0; od workload describe "$id" > "$p.stdout" 2> "$p.stderr" || rc=$?
    echo "exit: $rc" > "$p.meta"
    grep -q '^Memory:' "$p.stdout" && break
    sleep 2
  done
  trace "describe $id polls=$n"
  run_case "describe_${id}_final" od workload describe "$id"
done
# A refused id was never admitted.
run_case "describe_refused-128mb" od workload describe refused-128mb

# ---- hygiene: stop through the production verb ------------------------------
for id in dev-box dev-box-gib; do run_case "stop_$id" od job stop "$id"; done
sleep 5
trace "done"
exit 0
