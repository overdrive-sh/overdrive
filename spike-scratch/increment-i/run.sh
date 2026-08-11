#!/usr/bin/env bash
# PROBE increment-i — P11: what does `vhost-user-blk` COST?
#
# WHY THIS EXISTS. P10 (increment-h) established that `vhost-user-blk` works on
# CH v53, requires `--memory shared=on`, forfeits rate limiting, and — its one
# real edge — keeps the backend alive across the VMM's death. It explicitly
# recorded ONE thing as not established:
#
#     "No throughput number for `vhost-user-blk`. [...] The per-file overhead
#      comparison is therefore still open for this transport."
#
# This closes that. The instrument is NOT rewritten: it reuses increment-f's
# guest binary (`guest-init-blk`), its kernel, its rootfs, and its 1 GiB volume
# master, so the payload is byte-identical to the one P7 measured — the
# `measure_throughput` / `measure_per_file_latency` / `write_file_bytes`
# functions are literally the same source as increment-e's virtiofs guest
# (verified by diff). Only how the volume is ATTACHED changes.
#
# WHAT THE NUMBER DOES AND DOES NOT MEAN. The backend here is
# `qemu-storage-daemon`, a STAND-IN for a future `overdrive-fs` (#97, a Rust
# chunk store over Garage). Benchmarking it measures QEMU's block layer plus the
# vhost-user transport — NOT what `overdrive-fs` would cost. What it legitimately
# bounds: `plain` and `vublk` write THE SAME IMAGE FILE on THE SAME FILESYSTEM,
# so the delta between those two arms is the vhost-user transport overhead
# (plus qemu's block layer). That is the transferable number.
#
# THE `shared=on` CONFOUND, handled rather than hidden. `vhost-user-blk` is
# REFUSED at config validation without `--memory shared=on`
# (VhostUserRequiresSharedMemory, P10). Plain `--disk` does not need it. So a
# naive plain-vs-vublk comparison crosses a memory-backing change. This script
# therefore runs the plain arm BOTH ways — `plain` (no shared=on) and
# `plain-shared` (shared=on) — so the reader can see whether the backing itself
# moves the number before attributing anything to vhost-user.
#
# Usage: run.sh <mode>
#   plain         ordinary --disk volume, --memory size=512M
#   plain-shared  ordinary --disk volume, --memory size=512M,shared=on
#   vublk         vhost-user-blk volume via qemu-storage-daemon, shared=on
# (the virtiofs arm is increment-e's `run.sh full`, driven by bench.sh — the
#  established instrument for that mechanism, reused verbatim rather than
#  reimplemented here.)
#
# Env overrides: PAYLOAD_MIB, SMALL_FILES, VOLROOT, RL_FSIZE, RL_NOFILE.
#
# Run as root on the bare-metal box.
set -uo pipefail

MODE="${1:-plain}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
F_OUT=/var/tmp/spike-increment-f          # kernel + rootfs + guest binary
OUT=/var/tmp/spike-increment-i
ARCH="$(uname -m)"
BIN_DIR="$(cd "$HERE/../increment-f" && pwd)/target/${ARCH}-unknown-linux-musl/release"
RUN=/run/spike-increment-i
# MUST be on the same filesystem as the volume masters: `cp --reflink=always` is
# an intra-filesystem operation and fails EXDEV across mounts (P7 found this the
# hard way when the images were staged on /run tmpfs). Sockets and logs on
# tmpfs; disk images on the XFS volume.
VOLROOT="${VOLROOT:-/srv/vm/p6f}"

case "$ARCH" in
  x86_64)  CONSOLE_DEV=ttyS0  ;;
  aarch64) CONSOLE_DEV=ttyAMA0 ;;
  *) echo "unsupported arch: $ARCH" >&2; exit 1 ;;
esac

KERNEL="$F_OUT/kernel"
PAYLOAD_MIB="${PAYLOAD_MIB:-256}"
SMALL_FILES="${SMALL_FILES:-1000}"
CMDLINE="root=/dev/vda rw console=${CONSOLE_DEV} init=/init panic=1 loglevel=4 spike.mib=${PAYLOAD_MIB} spike.files=${SMALL_FILES}"
VSOCK_PORT=1234
VMM_USER=spikevmm
VMM_GID=6001
RL_FSIZE=${RL_FSIZE:-$((2 * 1024 * 1024 * 1024))}
RL_NOFILE=${RL_NOFILE:-256}

KVM_ORIG_MODE="$(stat -c %a /dev/kvm)"
cleanup() {
  # `comm` is truncated to 15 chars by the kernel, so `-x cloud-hypervisor` and
  # `-x qemu-storage-daemon` NEVER match. And `pkill -f "cloud-hypervisor ..."`
  # would match this script's own command line. Truncated comm + -x is the only
  # shape that is both effective and safe.
  pkill -9 -x cloud-hyperviso 2>/dev/null
  pkill -9 -x qemu-storage-da 2>/dev/null
  chmod "$KVM_ORIG_MODE" /dev/kvm 2>/dev/null || true
}
trap cleanup EXIT

echo "##########################################################"
echo "### PROBE increment-i (P11: vhost-user-blk cost)  mode=$MODE"
echo "### uname -r          : $(uname -r)   arch=$ARCH"
echo "### virt              : $(systemd-detect-virt || true)"
echo "### cpu               : $(grep -m1 'model name' /proc/cpuinfo | cut -d: -f2- | sed 's/^ //')"
echo "### cloud-hypervisor  : $(cloud-hypervisor --version 2>&1)"
echo "### qemu-storage-daemon: $(qemu-storage-daemon --version 2>&1 | head -1)"
echo "### payload           : ${PAYLOAD_MIB} MiB / ${SMALL_FILES} files"
echo "### volume root       : $VOLROOT"
echo "### volume fs         : $(findmnt -no FSTYPE --target "$VOLROOT" 2>/dev/null || echo '?') on $(findmnt -no SOURCE --target "$VOLROOT" 2>/dev/null || echo '?')"
echo "##########################################################"
echo

[ -f "$KERNEL" ] && [ -f "$F_OUT/rootfs.ext4" ] || {
  echo "!!! need increment-f's kernel+rootfs — run increment-f/build.sh first" >&2; exit 1; }
[ -f "$VOLROOT/volrw.ext4" ] && [ -f "$VOLROOT/volro.ext4" ] || {
  echo "!!! need increment-f's volume masters in $VOLROOT" >&2; exit 1; }

id "$VMM_USER" >/dev/null 2>&1 || {
  groupadd -g "$VMM_GID" "$VMM_USER" 2>/dev/null || true
  useradd -u 6001 -g "$VMM_GID" --system --no-create-home --shell /usr/sbin/nologin "$VMM_USER" 2>/dev/null
}
usermod -aG kvm "$VMM_USER" >/dev/null 2>&1
chown root:kvm /dev/kvm; chmod 0660 /dev/kvm
mkdir -p "$OUT"

##########################################################################
# Per-launch volume provisioning. Every arm starts from a FRESH reflink clone
# of the SAME pristine 1 GiB master, so no arm inherits another's dirty
# journal, allocated extents, or page-cache state in the image.
clone_volumes() {
  local disk_dir="$1"
  local t0 t1 how=reflink
  t0=$(date +%s.%N)
  if ! cp --reflink=always "$VOLROOT/volrw.ext4" "$disk_dir/volrw.ext4" 2>/dev/null; then
    how="FULL COPY (reflink unavailable on $(findmnt -no FSTYPE --target "$disk_dir"))"
    cp "$VOLROOT/volrw.ext4" "$disk_dir/volrw.ext4" || { echo "!!! volume copy FAILED" >&2; return 1; }
  fi
  cp --reflink=auto "$VOLROOT/volro.ext4" "$disk_dir/volro.ext4" || return 1
  t1=$(date +%s.%N)
  echo "--- volumes provisioned in $(echo "$t1 - $t0" | bc)s (1 GiB + 64 MiB) via $how"
}

# The vhost-user-blk backend. `cache.direct=off,cache.no-flush=off` is stated
# EXPLICITLY rather than left to the default so the caching mode is evidence
# and not an assumption: it is buffered writeback through the host page cache,
# which is also what CH's `--disk` does by default (no `direct=on`). Both arms
# therefore hit the same host cache layer, and the delta is transport, not
# O_DIRECT-vs-buffered.
#
# VUB_CACHE selects the backend's caching contract, which exists to settle an
# honesty question the first vublk run raised: its 256 MiB fsync came back in
# 0.007 s against plain --disk's 0.182 s. Either the bytes were already on the
# device, or the FLUSH was being dropped. `noflush` is the control — if it
# measures the SAME as `writeback`, flushes were never honoured and the vublk
# durable number is a lie. `direct` (O_DIRECT, nothing in the host page cache)
# is the other side of the same question.
VUB_CACHE="${VUB_CACHE:-writeback}"
case "$VUB_CACHE" in
  writeback) VUB_CACHE_OPTS="cache.direct=off,cache.no-flush=off" ;;
  direct)    VUB_CACHE_OPTS="cache.direct=on,cache.no-flush=off"  ;;
  noflush)   VUB_CACHE_OPTS="cache.direct=off,cache.no-flush=on"  ;;
  *) echo "unknown VUB_CACHE=$VUB_CACHE" >&2; exit 1 ;;
esac

start_backend() {
  local img="$1" sock="$2" log="$3"
  qemu-storage-daemon \
    --blockdev "driver=file,node-name=f0,filename=$img,$VUB_CACHE_OPTS" \
    --blockdev "driver=raw,node-name=r0,file=f0" \
    --export "type=vhost-user-blk,id=e0,node-name=r0,addr.type=unix,addr.path=$sock,writable=on" \
    >>"$log" 2>&1 &
  BACKEND_PID=$!
  for _ in $(seq 1 100); do [ -S "$sock" ] && { sleep 0.2; return 0; }; sleep 0.1; done
  return 1
}

##########################################################################
boot_once() {
  local vm_run="$RUN/vm"            # tmpfs: sockets, console, logs
  local vub_dir="$RUN/vub"          # tmpfs: the vhost-user-blk socket, own dir
  local disk_dir="$VOLROOT/run-i"   # XFS: disk images, so reflink works
  pkill -9 -x cloud-hyperviso 2>/dev/null
  pkill -9 -x qemu-storage-da 2>/dev/null
  sleep 0.3
  rm -rf "$RUN" "$disk_dir"; mkdir -p "$vm_run" "$vub_dir" "$disk_dir"

  local vsock_uds="$vm_run/ch.vsock"
  local listen_uds="${vsock_uds}_${VSOCK_PORT}"
  local console="$vm_run/console.log"
  local chlog="$vm_run/ch-stderr.log"
  local hostlog="$vm_run/host-collector.log"
  local marker="$vm_run/BEACON"
  local vub_sock="$vub_dir/vublk.sock"
  local vub_log="$vm_run/vublk.log"

  cp "$F_OUT/rootfs.ext4" "$disk_dir/rootfs.ext4"
  clone_volumes "$disk_dir" || return 98

  # The backend must open the image BEFORE CH connects, and it runs as root
  # while CH runs as the unprivileged spikevmm — so the socket needs to be
  # reachable by spikevmm.
  BACKEND_PID=""
  if [ "$MODE" = vublk ]; then
    start_backend "$disk_dir/volrw.ext4" "$vub_sock" "$vub_log" || {
      echo "!!! vhost-user-blk socket never appeared"; sed 's/^/    /' "$vub_log"; return 97; }
    chmod 0666 "$vub_sock"
    echo "--- qemu-storage-daemon pid=$BACKEND_PID socket=$vub_sock"
  fi

  chown -R "$VMM_USER:$VMM_GID" "$vm_run" "$disk_dir"
  chmod 0700 "$vm_run" "$disk_dir"
  chmod 0755 "$vub_dir"

  "$BIN_DIR/host-collector" "$listen_uds" "$marker" >"$hostlog" 2>&1 &
  local collector_pid=$!
  for _ in $(seq 1 100); do [ -S "$listen_uds" ] && break; sleep 0.05; done
  chmod 0666 "$listen_uds" 2>/dev/null

  # vda=rootfs, vdb=THE MEASURED VOLUME, vdc=read-only volume. Only vdb's
  # attachment changes between arms; vdc is a plain readonly --disk in all
  # three so it cannot contribute to the delta.
  #
  # image_type=raw is MANDATORY from CH v53 on every plain --disk. Auto-detection
  # is deprecated and its fallback "disables sector 0 writes" — and these images
  # are bare filesystems with no partition table, so sector 0 IS the filesystem.
  # Omit it and the guest faults, panic=1 reboots, and the failure surfaces two
  # layers away. (A vhost-user disk takes no image_type: the BACKEND owns the
  # format, which is what `driver=raw` above declares.)
  #
  # DISK_DIRECT=1 adds `direct=on` (O_DIRECT) to the plain --disk spec. It
  # exists so the transport delta can be measured under a MATCHED caching
  # contract: qemu-storage-daemon's default `cache.direct=off` was measured to
  # make the guest's fsync a no-op (the noflush control changed nothing), so the
  # only honest vublk number is its O_DIRECT one — and comparing that against a
  # BUFFERED plain --disk would be comparing durability contracts, not
  # transports.
  local VDB_SPEC="path=$disk_dir/volrw.ext4,image_type=raw"
  [ "${DISK_DIRECT:-0}" = 1 ] && VDB_SPEC="$VDB_SPEC,direct=on"
  [ "$MODE" = vublk ] && VDB_SPEC="vhost_user=on,socket=$vub_sock"
  local VDC_SPEC="path=$disk_dir/volro.ext4,image_type=raw,readonly=on"

  local MEM_SPEC="size=512M"
  case "$MODE" in
    plain-shared|vublk) MEM_SPEC="size=512M,shared=on" ;;
  esac

  local CH_ARGV=(
    cloud-hypervisor
    --cpus boot=1
    --memory "$MEM_SPEC"
    --kernel "$KERNEL"
    --cmdline "$CMDLINE"
    --disk "path=$disk_dir/rootfs.ext4,image_type=raw" "$VDB_SPEC" "$VDC_SPEC"
    --serial "file=$console"
    --console off
    --api-socket "path=$vm_run/ch-api.sock"
    --vsock "cid=3,socket=$vsock_uds"
    --seccomp true
    --landlock
  )
  # --landlock-rules is a multi-VALUE option, not a repeatable flag. The
  # vhost-user socket directory needs its OWN grant for the same reason P5's
  # correction 1 found the vsock UDS does: CH does not auto-derive one.
  local LL_RULES=("path=$vm_run,access=rw" "path=$disk_dir,access=rw")
  [ "$MODE" = vublk ] && LL_RULES+=("path=$vub_dir,access=rw")
  CH_ARGV+=(--landlock-rules "${LL_RULES[@]}")

  local PREFIX=(prlimit "--fsize=$RL_FSIZE" "--nofile=$RL_NOFILE" --
                setpriv --reuid="$VMM_USER" --regid="$VMM_GID" --init-groups --no-new-privs --)

  echo "--- exact launch argv:"
  printf '    %q' "${PREFIX[@]}" "${CH_ARGV[@]}"; echo
  echo "--- memory argv : $MEM_SPEC"
  echo "--- vdb (measured volume) : $VDB_SPEC"
  echo

  "${PREFIX[@]}" "${CH_ARGV[@]}" >"$chlog" 2>&1 &
  local ch_pid=$!

  local cap="$OUT/mem-$MODE.txt"
  rm -f "$cap"
  ( for _ in $(seq 1 300); do
      sleep 0.2
      [ -f "$marker" ] || continue
      [ -r "/proc/$ch_pid/status" ] || break
      {
        echo "########## mode=$MODE  pid=$ch_pid  snapshot AT BEACON ##########"
        echo "### memory argv          : $MEM_SPEC"
        echo "### vdb spec             : $VDB_SPEC"
        echo "### --- /proc/$ch_pid/status ---"
        grep -E '^(Name|Uid|Gid|NoNewPrivs|Seccomp|VmPeak|VmRSS|RssAnon|RssFile|RssShmem|Threads):' \
          "/proc/$ch_pid/status" 2>/dev/null
        echo "### --- memfd / shared mappings (ZERO without shared=on) ---"
        grep -cE 'memfd|/dev/shm' "/proc/$ch_pid/maps" 2>/dev/null | sed 's/^/  memfd-ish mapping lines: /'
        # The hypothesis under test for the shared=on write-phase collapse:
        # anonymous guest RAM gets transparent huge pages, a memfd-backed
        # shared mapping does not, so every guest page fault in the write path
        # costs 4K-granularity work instead of 2M. AnonHugePages vs
        # ShmemPmdMapped is the direct evidence either way.
        echo "### --- huge pages backing guest RAM ---"
        grep -E '^(Rss|AnonHugePages|ShmemPmdMapped|FilePmdMapped):' \
          "/proc/$ch_pid/smaps_rollup" 2>/dev/null | sed 's/^/  /'
        echo "### --- host THP policy ---"
        printf '  anon : %s\n' "$(cat /sys/kernel/mm/transparent_hugepage/enabled 2>/dev/null)"
        printf '  shmem: %s\n' "$(cat /sys/kernel/mm/transparent_hugepage/shmem_enabled 2>/dev/null)"
        if [ -n "${BACKEND_PID:-}" ] && [ -r "/proc/$BACKEND_PID/status" ]; then
          echo "### --- backend /proc/$BACKEND_PID/status ---"
          grep -E '^(Name|VmRSS|Threads):' "/proc/$BACKEND_PID/status" 2>/dev/null
        fi
      } >"$cap" 2>&1
      break
    done ) &
  local cap_pid=$!

  ( local waited=0
    while [ "$waited" -lt 600 ]; do
      sleep 3; waited=$((waited + 3))
      kill -0 "$ch_pid" 2>/dev/null || exit 0
      if [ ! -f "$marker" ] && [ "$waited" -ge 90 ]; then
        echo "    [watchdog] no beacon by ${waited}s, killing" >&2; break
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

  # Did the backend outlive the VMM? P10 observed this once; every vublk trial
  # here re-observes it for free.
  if [ "$MODE" = vublk ]; then
    LAST_BACKEND_ALIVE="$(pgrep -x qemu-storage-da | wc -l)"
  else
    LAST_BACKEND_ALIVE=n/a
  fi
  pkill -9 -x qemu-storage-da 2>/dev/null

  LAST_CONSOLE="$console"; LAST_HOSTLOG="$hostlog"; LAST_CHLOG="$chlog"
  LAST_CH_RC="$ch_rc"; LAST_CAP="$cap"; LAST_DISKDIR="$disk_dir"; LAST_VUBLOG="$vub_log"
  cp "$hostlog" "$OUT/transcript-$MODE.txt" 2>/dev/null
  cp "$console" "$OUT/console-$MODE.txt" 2>/dev/null
  return $collector_rc
}

##########################################################################
echo "=== [boot] mode=$MODE"
boot_once; BOOT_RC=$?
echo

echo "=========================== GUEST SERIAL CONSOLE ==========================="
grep -vE '^\[ *[0-9]+\.[0-9]+\] ' "${LAST_CONSOLE:-/dev/null}" 2>/dev/null | head -70
echo "=========================== END SERIAL CONSOLE ============================="
echo
echo "=========================== HOST VSOCK COLLECTOR ==========================="
cat "${LAST_HOSTLOG:-/dev/null}" 2>/dev/null
echo "=========================== END HOST COLLECTOR ============================="
echo
echo "--- cloud-hypervisor stderr:"; head -20 "${LAST_CHLOG:-/dev/null}" 2>/dev/null
echo "--- cloud-hypervisor exit code : ${LAST_CH_RC:-?}"
if [ "$MODE" = vublk ]; then
  echo "--- qemu-storage-daemon log:"; head -10 "${LAST_VUBLOG:-/dev/null}" 2>/dev/null | sed 's/^/    /'
  echo "--- backend still alive after the VMM exited: ${LAST_BACKEND_ALIVE:-?}"
fi
echo
echo "=========================== /proc AT BEACON ================================"
cat "${LAST_CAP:-/dev/null}" 2>/dev/null || echo "<none captured>"
echo "=========================== END /proc AT BEACON ============================"
echo

# A host-side assertion after a guest that never ran is VACUOUSLY true and reads
# as evidence. Same gate increment-e and increment-f use.
if [ "$BOOT_RC" != 0 ]; then
  echo "HOST-SIDE VERIFICATION SKIPPED — the guest never completed; every"
  echo "assertion below would be vacuously true."
  echo "--- increment-i mode=$MODE VERDICT: DID NOT COMPLETE"
  exit "$BOOT_RC"
fi

echo "=========================== HOST-SIDE VERIFICATION ========================="
MP="$OUT/verify-mnt"; mkdir -p "$MP"
umount "$MP" 2>/dev/null
if mount -o ro,noload,loop "${LAST_DISKDIR}/volrw.ext4" "$MP" 2>"$OUT/mount-err.txt"; then
  echo "--- measured volume, loop-mounted read-only post-shutdown:"
  if [ -f "$MP/from-guest.txt" ]; then
    echo "    from-guest.txt content : $(cat "$MP/from-guest.txt")"
    printf 'GUEST-WROTE-THIS-0123456789-abcdefghij\n' >"$OUT/expected-from-guest.txt"
    cmp -s "$OUT/expected-from-guest.txt" "$MP/from-guest.txt" \
      && echo "    +++ BYTE-IDENTICAL to what the guest reported writing" \
      || echo "    !!! DIFFERS from expected"
  else
    echo "    !!! from-guest.txt MISSING — guest->host did not round-trip"
  fi
  echo "    payload.bin  : $(ls -la "$MP/payload.bin" 2>/dev/null | awk '{print $5}' || echo missing) bytes"
  echo "    small files  : $(ls "$MP/manyfiles" 2>/dev/null | wc -l)"
  umount "$MP"
else
  echo "    !!! could not loop-mount the volume: $(cat "$OUT/mount-err.txt")"
fi
echo
# The matched host baseline. NOT a shell loop with one trailing `sync` — an
# earlier version of this harness did exactly that and made the host look ~6x
# faster than it is. `dd conv=fsync` and a python open+write+fsync+close loop
# are the SAME syscall sequence the guest issues.
echo "--- HOST-SIDE DIRECT-WRITE BASELINE (same syscalls, same filesystem)"
HOSTBASE="$VOLROOT/hostbase-i"; rm -rf "$HOSTBASE"; mkdir -p "$HOSTBASE/manyfiles"
S=$(date +%s.%N); dd if=/dev/zero of="$HOSTBASE/payload.bin" bs=1M count="$PAYLOAD_MIB" conv=fsync 2>&1 | tail -1; E=$(date +%s.%N)
echo "    host ${PAYLOAD_MIB} MiB direct write+fsync: $(echo "$E - $S" | bc)s"
python3 - "$HOSTBASE/manyfiles" "$SMALL_FILES" <<'PY'
import os, sys, time
d, n = sys.argv[1], int(sys.argv[2])
body = b"per-file-latency-probe\n"
t0 = time.monotonic()
for i in range(n):
    fd = os.open(os.path.join(d, f"f{i:04}.txt"), os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o644)
    os.write(fd, body); os.fsync(fd); os.close(fd)
dt = time.monotonic() - t0
print(f"    host {n} small files (open+write+fsync+close, SAME sequence as the guest): {dt:.3f}s -> mean {dt*1000/n:.2f} ms/file")
PY
rm -rf "$HOSTBASE"
echo "=========================== END HOST VERIFICATION =========================="
echo
echo "--- increment-i mode=$MODE VERDICT: COMPLETED"
exit 0
