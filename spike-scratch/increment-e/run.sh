#!/usr/bin/env bash
# PROBE increment-e — P6 on BARE METAL: does virtiofsd + `--memory shared=on`
# compose with the boot AND with the P5 confinement, and what does it cost?
#
# This is increment-d's script ported to env B (bare-metal x86_64). On env A
# (nested aarch64) `shared=on` never booted, so everything past the mount
# attempts was unreachable. Differences that matter are flagged inline.
#
# Usage: run.sh <mode> [max_attempts]
#   full              virtiofsd x2 + shared=on + fs devices + ALL P5 confinement
#   full-no-fsd-rule  as `full` but WITHOUT a --landlock-rules entry for the
#                     vhost-user socket directory. Tests whether CH v46
#                     auto-derives a rule for --fs socket= the way it does for
#                     --disk / --kernel / --api-socket (matrix2.sh showed it
#                     does NOT for --vsock).
#   sharedonly        shared=on, NO fs devices — isolates the cost of the
#                     memory backing from the cost of the fs device.
#   noshare           neither — the [D8b] baseline a volume-free VM would pay.
#
# Env overrides:
#   CACHE=never|auto|always   virtiofsd --cache mode. [D8c] picked `never` for
#                             the volume role WITHOUT measuring it; this is how
#                             that decision gets a number.
#   PAYLOAD_MIB, SMALL_FILES  handed to the guest on the kernel cmdline so a
#                             cache comparison holds the payload fixed.
#   RL_FSIZE, RL_NOFILE       prlimit ceilings (see the RLIMIT_FSIZE note below).
#
# The guest binary is identical in every mode and touches a fixed 128 MiB before
# the beacon, so the /proc snapshots are taken at the same guest lifecycle point
# with the same page-touching. Anything else would be comparing workloads, not
# backings.
#
# Run as root on the bare-metal box.
set -uo pipefail

MODE="${1:-full}"
# increment-d defaulted to 10 attempts to retry PAST the nested-virt stall.
# Env B has no stall (increment-a: 12/12), so a failure here is SIGNAL, not
# noise — default to a single attempt and let it surface.
MAX_ATTEMPTS="${2:-1}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
A_OUT=/var/tmp/spike-increment-a
OUT=/var/tmp/spike-increment-e
ARCH="$(uname -m)"
BIN_DIR="$HERE/target/${ARCH}-unknown-linux-musl/release"
RUN=/run/spike-increment-e
VIRTIOFSD=/usr/libexec/virtiofsd   # NOT on $PATH on this image

# Serial console device name is ARCH-SPECIFIC and getting it wrong produces NO
# output — indistinguishable from a hang. ttyS0 is the x86_64 8250/16550;
# ttyAMA0 is the ARM PL011.
case "$ARCH" in
  x86_64)  CONSOLE_DEV=ttyS0  ;;
  aarch64) CONSOLE_DEV=ttyAMA0 ;;
  *) echo "unsupported arch: $ARCH" >&2; exit 1 ;;
esac

# increment-e's OWN 0644 copy, staged by build.sh. increment-a's kernel is
# 0600 root:root and the uid-dropped CH cannot open it (KernelFile/EACCES).
KERNEL="$OUT/kernel"
CACHE="${CACHE:-never}"
PAYLOAD_MIB="${PAYLOAD_MIB:-256}"
SMALL_FILES="${SMALL_FILES:-1000}"
CMDLINE="root=/dev/vda rw console=${CONSOLE_DEV} init=/init panic=1 loglevel=7 spike.mib=${PAYLOAD_MIB} spike.files=${SMALL_FILES}"
VSOCK_PORT=1234
VMM_USER=spikevmm
VMM_GID=6001
# RLIMIT_FSIZE. increment-c sized this off the rootfs alone (128 MiB for a
# 64 MiB rootfs) and that was enough. It is NOT enough here: `--memory
# shared=on` backs guest RAM with a memfd, and a memfd is a FILE for
# RLIMIT_FSIZE purposes — so CH dies with SIGXFSZ (exit 153, "File size limit
# exceeded") the moment guest RAM exceeds the ceiling. The limit must therefore
# cover max(rootfs image, guest RAM) once shared=on is in play. See
# rlimit-sweep.sh for the isolating experiment, re-run here on x86_64 because
# the original was measured on aarch64.
RL_FSIZE=${RL_FSIZE:-$((1024 * 1024 * 1024))}
RL_NOFILE=${RL_NOFILE:-256}

# Volume SOURCE directories live on a native path, deliberately far from every
# path CH is given. [D8e] claims only virtiofsd reaches the data; if CH needed
# the source dir in its ruleset, volumes would widen [D7]'s confinement and
# US-VM-8's non-widening AC would have to be restated.
# VOLROOT defaults to $OUT (/var/tmp, ext4 on the root disk). The real volume
# home is /srv/vm — XFS with reflink=1, the filesystem P4's measurements ran on
# — so the throughput number means little unless it is taken there too.
VOLROOT="${VOLROOT:-$OUT}"
VOLRW="$VOLROOT/volsrc-rw"
VOLRO="$VOLROOT/volsrc-ro"

KVM_ORIG_MODE="$(stat -c %a /dev/kvm)"
cleanup() {
  pkill -9 -f "virtiofsd --socket-path=$RUN" 2>/dev/null
  chmod "$KVM_ORIG_MODE" /dev/kvm 2>/dev/null || true
}
trap cleanup EXIT

echo "##########################################################"
echo "### PROBE increment-e (P6, BARE METAL)  mode=$MODE cache=$CACHE"
echo "### uname -r          : $(uname -r)"
echo "### uname -m          : $ARCH"
echo "### virt              : $(systemd-detect-virt || true)   <- 'none' == not nested"
echo "### cpu               : $(grep -m1 'model name' /proc/cpuinfo | cut -d: -f2- | sed 's/^ //')"
echo "### cloud-hypervisor  : $(cloud-hypervisor --version 2>&1)"
echo "### virtiofsd         : $($VIRTIOFSD --version 2>&1)  (at $VIRTIOFSD)"
echo "### virtiofsd on PATH : $(command -v virtiofsd || echo '<NOT ON PATH>')"
echo "### payload           : ${PAYLOAD_MIB} MiB / ${SMALL_FILES} files"
echo "### volume src root   : $VOLROOT"
echo "### volume src fs     : $(findmnt -no FSTYPE --target "$VOLROOT" 2>/dev/null || echo '?') on $(findmnt -no SOURCE --target "$VOLROOT" 2>/dev/null || echo '?')"
echo "##########################################################"
echo

echo "=== [setup] --sandbox=namespace availability (the [D8d] question)"
$VIRTIOFSD --help 2>&1 | grep -A4 -- '--sandbox' | sed 's/^/    /'
echo

id "$VMM_USER" >/dev/null 2>&1 || {
  groupadd -g "$VMM_GID" "$VMM_USER" 2>/dev/null || true
  useradd -u 6001 -g "$VMM_GID" --system --no-create-home --shell /usr/sbin/nologin "$VMM_USER" 2>/dev/null
}
usermod -aG kvm "$VMM_USER" >/dev/null 2>&1
chown root:kvm /dev/kvm; chmod 0660 /dev/kvm
echo "=== [setup] VMM identity: $(id "$VMM_USER")"
echo "=== [setup] /dev/kvm: $(stat -c '%A %U:%G' /dev/kvm)"
echo

##########################################################################
seed_volumes() {
  rm -rf "$VOLRW" "$VOLRO"; mkdir -p "$VOLRW" "$VOLRO"
  printf 'HOST-WROTE-THIS-9876543210-zyxwvutsrq\n' >"$VOLRW/from-host.txt"
  printf 'PREEXISTING-HOST-CONTENT-DO-NOT-CHANGE\n' >"$VOLRO/preexisting-host-file.txt"
  chmod -R 0777 "$VOLRW" "$VOLRO"
  echo "--- volume sources seeded:"
  echo "    $VOLRW/from-host.txt : $(cat "$VOLRW/from-host.txt")"
  echo "    $VOLRO/preexisting-host-file.txt : $(cat "$VOLRO/preexisting-host-file.txt")"
}

start_virtiofsd() {
  local fsd_dir="$1"
  mkdir -p "$fsd_dir"
  # --cache per [D8c] (default never, overridable); --sandbox=namespace per
  # [D8d]; --seccomp kill is virtiofsd's own default and is left there.
  $VIRTIOFSD --socket-path="$fsd_dir/volrw.sock" --shared-dir="$VOLRW" \
    --tag=volrw --cache="$CACHE" --sandbox=namespace --seccomp kill \
    --socket-group="$VMM_USER" --log-level=info >"$fsd_dir/fsd-rw.log" 2>&1 &
  FSD_RW_PID=$!
  $VIRTIOFSD --socket-path="$fsd_dir/volro.sock" --shared-dir="$VOLRO" \
    --tag=volro --cache="$CACHE" --sandbox=namespace --seccomp kill --readonly \
    --socket-group="$VMM_USER" --log-level=info >"$fsd_dir/fsd-ro.log" 2>&1 &
  FSD_RO_PID=$!

  local ok=1
  for _ in $(seq 1 100); do
    [ -S "$fsd_dir/volrw.sock" ] && [ -S "$fsd_dir/volro.sock" ] && { ok=0; break; }
    sleep 0.1
  done
  echo "--- virtiofsd rw pid=$FSD_RW_PID  ro pid=$FSD_RO_PID  sockets_ready=$([ $ok = 0 ] && echo yes || echo NO)"
  ls -la "$fsd_dir" | sed 's/^/    /'
  echo "--- virtiofsd rw log:"; sed 's/^/    /' "$fsd_dir/fsd-rw.log"
  echo "--- virtiofsd ro log:"; sed 's/^/    /' "$fsd_dir/fsd-ro.log"
  # [D8d]: is --sandbox=namespace actually in effect, or silently downgraded?
  # Compare namespace inodes against this shell's. mnt+net differing is the
  # honest claim; pid+user do NOT differ.
  if [ "$ok" = 0 ]; then
    echo "--- [D8d] virtiofsd namespace inodes vs this shell (pid $FSD_RW_PID):"
    for ns in mnt net pid user; do
      printf '      %-5s fsd=%-24s shell=%s\n' "$ns" \
        "$(readlink "/proc/$FSD_RW_PID/ns/$ns" 2>/dev/null || echo '?')" \
        "$(readlink "/proc/$$/ns/$ns" 2>/dev/null || echo '?')"
    done
  fi
  return $ok
}

##########################################################################
boot_once() {
  local attempt=$1
  local vm_run="$RUN/vm"
  local fsd_dir="$RUN/fsd"
  pkill -9 -f "cloud-hypervisor --cpus boot=" 2>/dev/null
  pkill -9 -f "virtiofsd --socket-path=$RUN" 2>/dev/null
  sleep 0.3
  rm -rf "$RUN"; mkdir -p "$vm_run" "$fsd_dir"
  seed_volumes

  local vsock_uds="$vm_run/ch.vsock"
  local listen_uds="${vsock_uds}_${VSOCK_PORT}"
  local console="$vm_run/console.log"
  local chlog="$vm_run/ch-stderr.log"
  local hostlog="$vm_run/host-collector.log"
  local marker="$vm_run/BEACON"
  cp "$OUT/rootfs.ext4" "$vm_run/rootfs.ext4"
  chown -R "$VMM_USER:$VMM_GID" "$vm_run"
  chmod 0700 "$vm_run"

  "$BIN_DIR/host-collector" "$listen_uds" "$marker" >"$hostlog" 2>&1 &
  local collector_pid=$!
  for _ in $(seq 1 100); do [ -S "$listen_uds" ] && break; sleep 0.05; done
  chmod 0666 "$listen_uds" 2>/dev/null

  local want_fs=0
  case "$MODE" in full|full-no-fsd-rule) want_fs=1 ;; esac
  if [ "$want_fs" = 1 ]; then
    start_virtiofsd "$fsd_dir" || { echo "!!! virtiofsd sockets never appeared"; return 99; }
  fi

  local mem="size=512M"
  case "$MODE" in
    full|full-no-fsd-rule|sharedonly) mem="size=512M,shared=on" ;;
    noshare) mem="size=512M" ;;
  esac

  local CH_ARGV=(
    cloud-hypervisor
    --cpus boot=1
    --memory "$mem"
    --kernel "$KERNEL"
    --cmdline "$CMDLINE"
  # image_type=raw is MANDATORY from CH v53. Auto-detection is deprecated, and
  # its fallback is not benign: v53 logs "Autodetected raw image type. Disabling
  # sector 0 writes", then the guest's writes to sector 0 are refused. Our
  # images are bare filesystems with no partition table, so sector 0 IS the
  # filesystem. The guest faults, `panic=1` reboots it, and on reboot CH cannot
  # reconnect to the already-exited virtiofsd -> fatal CreateVirtioFs/
  # ConnectionRefused. Only the --fs modes died, which made it look like a
  # virtiofs regression; it is a disk-parameter migration.
    --disk "path=$vm_run/rootfs.ext4,image_type=raw"
    --serial "file=$console"
    --console off
    --api-socket "path=$vm_run/ch-api.sock"
    --vsock "cid=3,socket=$vsock_uds"
    --seccomp true
    --landlock
  )
  # NOTE: --landlock-rules (like --disk and --fs) is a multi-VALUE option, not a
  # repeatable flag: `--landlock-rules a b`, never `--landlock-rules a
  # --landlock-rules b`. The repeated form fails with
  #   error: the argument '--landlock-rules <landlock-rules>...' cannot be used multiple times
  local LL_RULES=("path=$vm_run,access=rw")
  if [ "$want_fs" = 1 ]; then
    CH_ARGV+=(--fs "tag=volrw,socket=$fsd_dir/volrw.sock" "tag=volro,socket=$fsd_dir/volro.sock")
    # The [D8e] test: the vhost-user socket dir MAY be needed; the volume
    # SOURCE dirs ($VOLRW/$VOLRO) are deliberately NEVER granted.
    [ "$MODE" = full ] && LL_RULES+=("path=$fsd_dir,access=rw")
  fi
  CH_ARGV+=(--landlock-rules "${LL_RULES[@]}")

  local PREFIX=(prlimit "--fsize=$RL_FSIZE" "--nofile=$RL_NOFILE" --
                setpriv --reuid="$VMM_USER" --regid="$VMM_GID" --init-groups --no-new-privs --)

  if [ "$attempt" = 1 ]; then
    echo "--- exact launch argv:"
    printf '    %q' "${PREFIX[@]}" "${CH_ARGV[@]}"; echo
    echo "--- landlock rules granted to CH: $vm_run$([ "$MODE" = full ] && echo " , $fsd_dir")"
    echo "--- landlock rules NOT granted  : $VOLRW , $VOLRO  (the [D8e] claim under test)"
    echo
  fi

  "${PREFIX[@]}" "${CH_ARGV[@]}" >"$chlog" 2>&1 &
  local ch_pid=$!

  # ---- /proc snapshot AT THE BEACON: identical guest lifecycle point in
  # ---- every mode, 128 MiB already resident, before any fs I/O.
  local cap="$OUT/mem-$MODE.txt"
  rm -f "$cap"
  ( for _ in $(seq 1 200); do
      sleep 0.2
      [ -f "$marker" ] || continue
      [ -r "/proc/$ch_pid/status" ] || break
      {
        echo "########## mode=$MODE cache=$CACHE  pid=$ch_pid  snapshot AT BEACON ##########"
        echo "### memory argv          : --memory $mem"
        echo "### fs devices present   : $([ "$want_fs" = 1 ] && echo yes || echo no)"
        echo "### wall clock           : $(date -Is)"
        echo "### --- /proc/$ch_pid/status (memory + confinement) ---"
        grep -E '^(Name|Uid|Gid|Groups|NoNewPrivs|Seccomp|VmPeak|VmSize|VmHWM|VmRSS|RssAnon|RssFile|RssShmem|Threads):' \
          "/proc/$ch_pid/status" 2>/dev/null
        echo "### --- /proc/$ch_pid/smaps_rollup ---"
        cat "/proc/$ch_pid/smaps_rollup" 2>/dev/null
        echo "### --- /proc/$ch_pid/limits (reduced ceilings) ---"
        grep -E 'Max file size|Max open files' "/proc/$ch_pid/limits" 2>/dev/null
        echo "### --- per-thread seccomp (the thread-group leader is NOT where CH filters) ---"
        for t in /proc/$ch_pid/task/*; do
          printf '  %-18s %s\n' "$(cat "$t/comm" 2>/dev/null)" \
            "$(grep -E '^(Seccomp|NoNewPrivs):' "$t/status" 2>/dev/null | tr '\n\t' ' ; ')"
        done
        echo "### --- memfd / shared mappings in the VMM address space ---"
        grep -cE 'memfd|/dev/shm' "/proc/$ch_pid/maps" 2>/dev/null | sed 's/^/  memfd-ish mapping lines: /'
        awk '/memfd|dev\/shm/ {print "  " $0}' "/proc/$ch_pid/maps" 2>/dev/null | head -5
        echo "### --- host-side free(1) while the VM is live ---"
        free -m 2>/dev/null | sed 's/^/  /'
      } >"$cap" 2>&1
      break
    done ) &
  local cap_pid=$!

  # Watchdog. On env A this existed to cut short nested-virt stalls; on env B a
  # missing beacon is a real failure, so the short arm just fails fast rather
  # than burning the full budget. A run that DID reach the beacon is doing real
  # filesystem work and gets the full window.
  ( local waited=0
    while [ "$waited" -lt 600 ]; do
      sleep 3; waited=$((waited + 3))
      kill -0 "$ch_pid" 2>/dev/null || exit 0
      if [ ! -f "$marker" ] && [ "$waited" -ge 90 ]; then
        echo "    [watchdog] no beacon by ${waited}s -> guest never reached /init, killing" >&2
        break
      fi
    done
    kill -9 "$ch_pid" 2>/dev/null ) &
  local watchdog=$!
  wait "$ch_pid"; local ch_rc=$?
  kill "$watchdog" 2>/dev/null; wait "$watchdog" 2>/dev/null
  wait "$cap_pid" 2>/dev/null

  for _ in $(seq 1 60); do kill -0 "$collector_pid" 2>/dev/null || break; sleep 0.1; done
  kill -9 "$collector_pid" 2>/dev/null
  wait "$collector_pid" 2>/dev/null; local collector_rc=$?

  LAST_CONSOLE="$console"; LAST_HOSTLOG="$hostlog"; LAST_CHLOG="$chlog"
  LAST_CH_RC="$ch_rc"; LAST_CAP="$cap"; LAST_FSD="$fsd_dir"
  # $RUN is wiped at the top of every boot_once, so persist the transcript and
  # console where a later cross-mode comparison can still read them.
  # Keyed by mode AND cache mode: `full` runs at both cache settings, and
  # keying on mode alone let the second run silently overwrite the first.
  cp "$hostlog" "$OUT/transcript-$MODE-$CACHE.txt" 2>/dev/null
  cp "$console" "$OUT/console-$MODE-$CACHE.txt" 2>/dev/null
  return $collector_rc
}

##########################################################################
echo "=== [boot] mode=$MODE"
BOOT_RC=1
for attempt in $(seq 1 "$MAX_ATTEMPTS"); do
  echo "--- attempt $attempt/$MAX_ATTEMPTS"
  boot_once "$attempt"; BOOT_RC=$?
  if [ "$BOOT_RC" = 0 ]; then echo "--- attempt $attempt SUCCEEDED"; break; fi
  echo "--- attempt $attempt did not complete (rc=$BOOT_RC, ch_rc=${LAST_CH_RC:-?})"
  head -3 "${LAST_CHLOG:-/dev/null}" 2>/dev/null | grep -i error
  tail -2 "${LAST_CONSOLE:-/dev/null}" 2>/dev/null
  # A CLI/config rejection is deterministic — retrying it only wastes wall clock
  # and buries the real message.
  if grep -qE "^error:|^Error parsing config" "${LAST_CHLOG:-/dev/null}" 2>/dev/null; then
    echo "--- deterministic CH rejection, not a transient — stopping retries"
    break
  fi
done
echo

echo "=========================== GUEST SERIAL CONSOLE ==========================="
grep -vE '^\[ *[0-9]+\.[0-9]+\] ' "${LAST_CONSOLE:-/dev/null}" 2>/dev/null | head -80
echo "--- (kernel ring lines elided above; full log at ${LAST_CONSOLE:-?})"
echo "=========================== END SERIAL CONSOLE ============================="
echo
echo "=========================== HOST VSOCK COLLECTOR ==========================="
cat "${LAST_HOSTLOG:-/dev/null}" 2>/dev/null
echo "=========================== END HOST COLLECTOR ============================="
echo
echo "--- cloud-hypervisor stderr:"
head -20 "${LAST_CHLOG:-/dev/null}" 2>/dev/null
echo "--- cloud-hypervisor exit code : ${LAST_CH_RC:-?}"
echo
echo "=========================== /proc AT BEACON ================================"
cat "${LAST_CAP:-/dev/null}" 2>/dev/null || echo "<none captured>"
echo "=========================== END /proc AT BEACON ============================"
echo

# Gated on a COMPLETED boot on purpose. If the guest never ran, "the read-only
# file is unchanged" is vacuously true and would read as evidence of host-side
# enforcement when nothing was ever attempted. This gate is why env A's P6
# reported UNPROVEN rather than a false pass.
if [ "$BOOT_RC" != 0 ]; then
  echo "=========================== HOST-SIDE VERIFICATION ========================="
  echo "SKIPPED — the guest never completed, so every host-side assertion below"
  echo "would be VACUOUSLY true (nothing was attempted against the shares)."
  echo "=========================== END HOST VERIFICATION =========================="
  echo
  echo "--- P6 mode=$MODE cache=$CACHE VERDICT: DID NOT COMPLETE"
  exit $BOOT_RC
fi

case "$MODE" in full|full-no-fsd-rule)
echo "=========================== HOST-SIDE VERIFICATION ========================="
echo "--- guest->host round trip: $VOLRW/from-guest.txt"
if [ -f "$VOLRW/from-guest.txt" ]; then
  echo "    exists; bytes=$(stat -c %s "$VOLRW/from-guest.txt") owner=$(stat -c %U:%G "$VOLRW/from-guest.txt")"
  echo "    content : $(cat "$VOLRW/from-guest.txt")"
  printf 'GUEST-WROTE-THIS-0123456789-abcdefghij\n' >"$OUT/expected-from-guest.txt"
  if cmp -s "$OUT/expected-from-guest.txt" "$VOLRW/from-guest.txt"; then
    echo "    +++ BYTE-IDENTICAL to what the guest reported writing"
  else
    echo "    !!! DIFFERS from expected"; cmp "$OUT/expected-from-guest.txt" "$VOLRW/from-guest.txt"
  fi
else
  echo "    !!! MISSING — guest->host did not round-trip"
fi
echo "--- throughput payload: $(ls -la "$VOLRW/payload.bin" 2>/dev/null || echo '<missing>')"
echo "--- small files       : $(ls "$VOLRW/manyfiles" 2>/dev/null | wc -l) files"
echo
echo "--- HOST-SIDE read-only enforcement ([D8g]); source dir $VOLRO"
ls -la "$VOLRO" | sed 's/^/    /'
if [ -f "$VOLRO/guest-should-not-create.txt" ]; then
  echo "    !!! guest-should-not-create.txt EXISTS -> host-side read_only NOT enforced"
else
  echo "    +++ guest-should-not-create.txt absent -> the guest's create was refused HOST-SIDE"
fi
echo "    preexisting file content now: $(cat "$VOLRO/preexisting-host-file.txt")"
if grep -q 'PREEXISTING-HOST-CONTENT-DO-NOT-CHANGE' "$VOLRO/preexisting-host-file.txt"; then
  echo "    +++ unchanged -> the guest's overwrite was refused HOST-SIDE"
else
  echo "    !!! MODIFIED -> host-side read_only NOT enforced"
fi
echo
echo "--- [D8e] check: did CH ever need the volume SOURCE dirs in its ruleset?"
echo "    CH landlock rules were: $RUN/vm$([ "$MODE" = full ] && echo " and $RUN/fsd")"
echo "    volume sources ($VOLRW, $VOLRO) were NEVER granted."
echo "    boot+round-trip outcome above answers it."
echo
echo "--- virtiofsd logs (post-run)"
sed 's/^/    [rw] /' "${LAST_FSD:-$RUN/fsd}/fsd-rw.log" 2>/dev/null
sed 's/^/    [ro] /' "${LAST_FSD:-$RUN/fsd}/fsd-ro.log" 2>/dev/null
echo
echo "--- HOST-SIDE DIRECT-WRITE BASELINE (same payload, no virtiofs in the path)"
echo "    Same filesystem the shares live on, so the guest number above divides"
echo "    into 'virtiofs overhead' and 'the disk'."
HOSTBASE="$VOLROOT/hostbase"; rm -rf "$HOSTBASE"; mkdir -p "$HOSTBASE/manyfiles"
S=$(date +%s.%N); dd if=/dev/zero of="$HOSTBASE/payload.bin" bs=1M count="$PAYLOAD_MIB" conv=fsync 2>&1 | tail -1; E=$(date +%s.%N)
echo "    host ${PAYLOAD_MIB} MiB direct write+fsync: $(echo "$E - $S" | bc)s"
# The guest does open+write+fsync+close PER FILE. increment-d's baseline used a
# shell loop with a single trailing `sync`, which is a different operation
# entirely — it made the host look ~6x faster than it is and the ratio
# meaningless. Match the guest's syscall sequence exactly instead.
python3 - "$HOSTBASE/manyfiles" "$SMALL_FILES" <<'PY'
import os, sys, time
d, n = sys.argv[1], int(sys.argv[2])
body = b"per-file-latency-probe\n"
t0 = time.monotonic()
for i in range(n):
    fd = os.open(os.path.join(d, f"f{i:04}.txt"), os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o644)
    os.write(fd, body)
    os.fsync(fd)
    os.close(fd)
dt = time.monotonic() - t0
print(f"    host {n} small files (open+write+fsync+close, SAME sequence as the guest): "
      f"{dt:.3f}s -> mean {dt * 1000 / n:.2f} ms/file")
PY
echo "=========================== END HOST VERIFICATION =========================="
;;
esac

echo
echo "--- P6 mode=$MODE cache=$CACHE VERDICT: COMPLETED"
exit 0
