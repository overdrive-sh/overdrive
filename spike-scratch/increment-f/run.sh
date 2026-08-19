#!/usr/bin/env bash
# PROBE increment-f — volumes over virtio-blk, the counterfactual to
# increment-e's virtiofs volumes.
#
# Held identical to increment-e wherever it is not the thing under test: same
# kernel, same box, same 128 MiB pre-beacon touch, same payload, same P5
# confinement, same beacon-synchronised /proc capture, same matched host
# baseline on the same filesystem. The deliberate differences:
#
#   * volumes are `--disk` devices, not `--fs` + virtiofsd
#   * NO `--memory shared=on`  <- the whole point
#   * no virtiofsd process at all
#
# Usage: run.sh <mode>
#   blk            two block volumes (rw + host-side readonly=on)
#   blk-ratelimit  same, plus `bw_size`/`bw_refill_time` on the rw volume.
#                  `--disk` supports rate limiting; `--fs` does not. That claim
#                  gets demonstrated rather than asserted.
# For the volume-free baseline, use increment-e's `noshare` mode — it is the
# same VM shape and the numbers are directly comparable.
#
# Env overrides: PAYLOAD_MIB, SMALL_FILES, VOLROOT, RL_FSIZE, RL_NOFILE.
#
# Run as root on the bare-metal box.
set -uo pipefail

MODE="${1:-blk}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT=/var/tmp/spike-increment-f
ARCH="$(uname -m)"
BIN_DIR="$HERE/target/${ARCH}-unknown-linux-musl/release"
RUN=/run/spike-increment-f
VOLROOT="${VOLROOT:-/srv/vm/p6f}"

case "$ARCH" in
  x86_64)  CONSOLE_DEV=ttyS0  ;;
  aarch64) CONSOLE_DEV=ttyAMA0 ;;
  *) echo "unsupported arch: $ARCH" >&2; exit 1 ;;
esac

KERNEL="$OUT/kernel"
PAYLOAD_MIB="${PAYLOAD_MIB:-256}"
SMALL_FILES="${SMALL_FILES:-1000}"
CMDLINE="root=/dev/vda rw console=${CONSOLE_DEV} init=/init panic=1 loglevel=7 spike.mib=${PAYLOAD_MIB} spike.files=${SMALL_FILES}"
VSOCK_PORT=1234
VMM_USER=spikevmm
VMM_GID=6001
# RLIMIT_FSIZE, and the contrast with increment-e is the interesting part:
#   virtiofs + shared=on -> the ceiling must cover GUEST RAM (memfd is a file)
#   block volumes        -> the ceiling must cover the LARGEST VOLUME IMAGE,
#                           because CH itself writes those files
# Neither is "size it off the rootfs". The rw volume image is 1 GiB here.
RL_FSIZE=${RL_FSIZE:-$((2 * 1024 * 1024 * 1024))}
RL_NOFILE=${RL_NOFILE:-256}

# Rate limit for blk-ratelimit: 32 MiB per 1000 ms.
RL_BW_SIZE=${RL_BW_SIZE:-$((32 * 1024 * 1024))}
RL_BW_REFILL=${RL_BW_REFILL:-1000}

KVM_ORIG_MODE="$(stat -c %a /dev/kvm)"
cleanup() { chmod "$KVM_ORIG_MODE" /dev/kvm 2>/dev/null || true; }
trap cleanup EXIT

echo "##########################################################"
echo "### PROBE increment-f (virtio-blk volumes)  mode=$MODE"
echo "### uname -r          : $(uname -r)   arch=$ARCH"
echo "### virt              : $(systemd-detect-virt || true)"
echo "### cpu               : $(grep -m1 'model name' /proc/cpuinfo | cut -d: -f2- | sed 's/^ //')"
echo "### cloud-hypervisor  : $(cloud-hypervisor --version 2>&1)"
echo "### virtiofsd         : NOT USED — no daemon in this path"
echo "### payload           : ${PAYLOAD_MIB} MiB / ${SMALL_FILES} files"
echo "### volume root       : $VOLROOT"
echo "### volume fs         : $(findmnt -no FSTYPE --target "$VOLROOT" 2>/dev/null || echo '?') on $(findmnt -no SOURCE --target "$VOLROOT" 2>/dev/null || echo '?')"
echo "##########################################################"
echo

id "$VMM_USER" >/dev/null 2>&1 || {
  groupadd -g "$VMM_GID" "$VMM_USER" 2>/dev/null || true
  useradd -u 6001 -g "$VMM_GID" --system --no-create-home --shell /usr/sbin/nologin "$VMM_USER" 2>/dev/null
}
usermod -aG kvm "$VMM_USER" >/dev/null 2>&1
chown root:kvm /dev/kvm; chmod 0660 /dev/kvm
echo "=== [setup] VMM identity: $(id "$VMM_USER")"
echo

##########################################################################
# Per-launch volume provisioning, the way a driver would actually do it:
# reflink-clone the pristine master. P4 measured this at ~260x cheaper than a
# copy and free in space on XFS(reflink=1).
# NOTE, and it is a real constraint on the driver, not a quirk of this probe:
# reflink is an intra-filesystem operation. The first version of this script put
# the per-VM disk images under $RUN (/run, tmpfs) alongside the sockets, and
# `cp --reflink=always` failed with EXDEV:
#     cp: failed to clone '/run/.../x.ext4' from '/srv/vm/...': Invalid cross-device link
# `--reflink=always` fails LOUDLY. `--reflink=auto` — which is coreutils >=9's
# default for plain `cp` — would instead have silently done a FULL COPY, and
# P4's ~260x advantage would have evaporated with no error anywhere. So the
# per-VM disk images live on the SAME filesystem as their masters; only the
# sockets and logs live on tmpfs.
clone_volumes() {
  local disk_dir="$1"
  local t0 t1
  t0=$(date +%s.%N)
  local how=reflink
  if ! cp --reflink=always "$VOLROOT/volrw.ext4" "$disk_dir/volrw.ext4" 2>/dev/null; then
    # Not every filesystem supports FICLONE (ext4 does not). Falling back is
    # fine when the question is "does it boot" rather than "what does the clone
    # cost" — but it is announced, never silent, because a silent full copy is
    # exactly the failure mode `--reflink=auto` would have produced.
    how="FULL COPY (reflink unavailable on $(findmnt -no FSTYPE --target "$disk_dir"))"
    cp "$VOLROOT/volrw.ext4" "$disk_dir/volrw.ext4" || { echo "!!! volume copy FAILED" >&2; return 1; }
  fi
  cp --reflink=auto "$VOLROOT/volro.ext4" "$disk_dir/volro.ext4" || return 1
  t1=$(date +%s.%N)
  echo "--- volumes provisioned in $(echo "$t1 - $t0" | bc)s (1 GiB + 64 MiB) via $how"
  echo "    clone size: $(du -sh --apparent-size "$disk_dir/volrw.ext4" | cut -f1) apparent / $(du -sh "$disk_dir/volrw.ext4" | cut -f1) actual on disk"
}

##########################################################################
boot_once() {
  local vm_run="$RUN/vm"          # tmpfs: sockets, console, logs
  local disk_dir="$VOLROOT/run"   # XFS: disk images, so reflink works
  pkill -9 -f "cloud-hypervisor --cpus boot=" 2>/dev/null
  sleep 0.3
  rm -rf "$RUN" "$disk_dir"; mkdir -p "$vm_run" "$disk_dir"

  local vsock_uds="$vm_run/ch.vsock"
  local listen_uds="${vsock_uds}_${VSOCK_PORT}"
  local console="$vm_run/console.log"
  local chlog="$vm_run/ch-stderr.log"
  local hostlog="$vm_run/host-collector.log"
  local marker="$vm_run/BEACON"
  cp "$OUT/rootfs.ext4" "$disk_dir/rootfs.ext4"
  clone_volumes "$disk_dir" || return 98
  chown -R "$VMM_USER:$VMM_GID" "$vm_run" "$disk_dir"
  chmod 0700 "$vm_run" "$disk_dir"

  "$BIN_DIR/host-collector" "$listen_uds" "$marker" >"$hostlog" 2>&1 &
  local collector_pid=$!
  for _ in $(seq 1 100); do [ -S "$listen_uds" ] && break; sleep 0.05; done
  chmod 0666 "$listen_uds" 2>/dev/null

  # Volume disk specs. Order matters: vda=rootfs, vdb=rw volume, vdc=ro volume.
  local RW_SPEC="path=$disk_dir/volrw.ext4,image_type=raw"
  [ "$MODE" = blk-ratelimit ] && \
    RW_SPEC="path=$disk_dir/volrw.ext4,image_type=raw,bw_size=$RL_BW_SIZE,bw_refill_time=$RL_BW_REFILL"
  # readonly=on is the [D8g] analogue: host-side enforcement, tested against a
  # guest that mounts it read-WRITE and tries to write.
  local RO_SPEC="path=$disk_dir/volro.ext4,image_type=raw,readonly=on"

  local CH_ARGV=(
    cloud-hypervisor
    --cpus boot=1
    --memory "size=512M"          # <-- NO shared=on. The point of increment-f.
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
    --disk "path=$disk_dir/rootfs.ext4,image_type=raw" "$RW_SPEC" "$RO_SPEC"
    --serial "file=$console"
    --console off
    --api-socket "path=$vm_run/ch-api.sock"
    --vsock "cid=3,socket=$vsock_uds"
    --seccomp true
    --landlock
    --landlock-rules "path=$vm_run,access=rw" "path=$disk_dir,access=rw"
  )
  local PREFIX=(prlimit "--fsize=$RL_FSIZE" "--nofile=$RL_NOFILE" --
                setpriv --reuid="$VMM_USER" --regid="$VMM_GID" --init-groups --no-new-privs --)

  echo "--- exact launch argv:"
  printf '    %q' "${PREFIX[@]}" "${CH_ARGV[@]}"; echo
  echo "--- memory argv: size=512M   (NO shared=on — no memfd, no nested-virt blocker)"
  [ "$MODE" = blk-ratelimit ] && echo "--- rate limit on vdb: bw_size=$RL_BW_SIZE bytes / bw_refill_time=$RL_BW_REFILL ms"
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
        echo "### memory argv          : size=512M (no shared=on)"
        echo "### --- /proc/$ch_pid/status ---"
        grep -E '^(Name|Uid|Gid|Groups|NoNewPrivs|Seccomp|VmPeak|VmSize|VmHWM|VmRSS|RssAnon|RssFile|RssShmem|Threads):' \
          "/proc/$ch_pid/status" 2>/dev/null
        echo "### --- /proc/$ch_pid/limits ---"
        grep -E 'Max file size|Max open files' "/proc/$ch_pid/limits" 2>/dev/null
        echo "### --- per-thread seccomp ---"
        for t in /proc/$ch_pid/task/*; do
          printf '  %-18s %s\n' "$(cat "$t/comm" 2>/dev/null)" \
            "$(grep -E '^(Seccomp|NoNewPrivs):' "$t/status" 2>/dev/null | tr '\n\t' ' ; ')"
        done
        echo "### --- memfd / shared mappings (expect ZERO without shared=on) ---"
        grep -cE 'memfd|/dev/shm' "/proc/$ch_pid/maps" 2>/dev/null | sed 's/^/  memfd-ish mapping lines: /'
        echo "### --- host free(1) while live ---"
        free -m 2>/dev/null | sed 's/^/  /'
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

  LAST_CONSOLE="$console"; LAST_HOSTLOG="$hostlog"; LAST_CHLOG="$chlog"
  LAST_CH_RC="$ch_rc"; LAST_CAP="$cap"; LAST_VMRUN="$vm_run"; LAST_DISKDIR="$disk_dir"
  cp "$hostlog" "$OUT/transcript-$MODE.txt" 2>/dev/null
  cp "$console" "$OUT/console-$MODE.txt" 2>/dev/null
  return $collector_rc
}

##########################################################################
echo "=== [boot] mode=$MODE"
boot_once; BOOT_RC=$?
echo

echo "=========================== GUEST SERIAL CONSOLE ==========================="
grep -vE '^\[ *[0-9]+\.[0-9]+\] ' "${LAST_CONSOLE:-/dev/null}" 2>/dev/null | head -80
echo "=========================== END SERIAL CONSOLE ============================="
echo
echo "=========================== HOST VSOCK COLLECTOR ==========================="
cat "${LAST_HOSTLOG:-/dev/null}" 2>/dev/null
echo "=========================== END HOST COLLECTOR ============================="
echo
echo "--- cloud-hypervisor stderr:"; head -20 "${LAST_CHLOG:-/dev/null}" 2>/dev/null
echo "--- cloud-hypervisor exit code : ${LAST_CH_RC:-?}"
echo
echo "=========================== /proc AT BEACON ================================"
cat "${LAST_CAP:-/dev/null}" 2>/dev/null || echo "<none captured>"
echo "=========================== END /proc AT BEACON ============================"
echo

# Same gate as increment-e: a host-side assertion after a guest that never ran
# is VACUOUSLY true and reads as evidence.
if [ "$BOOT_RC" != 0 ]; then
  echo "HOST-SIDE VERIFICATION SKIPPED — the guest never completed; every"
  echo "assertion below would be vacuously true."
  echo "--- increment-f mode=$MODE VERDICT: DID NOT COMPLETE"
  exit "$BOOT_RC"
fi

echo "=========================== HOST-SIDE VERIFICATION ========================="
echo "--- THE SEMANTIC DIFFERENCE, stated plainly:"
echo "    increment-e could read the guest's write from the host DURING the run."
echo "    A block volume is single-writer, so the host must loop-mount the image"
echo "    AFTER shutdown. That is not a performance difference; it is a"
echo "    capability difference, and it is the real decision axis."
echo
MP="$OUT/verify-mnt"; mkdir -p "$MP"
umount "$MP" 2>/dev/null
# `-o ro` alone fails on a dirty ext4 (journal replay needs write access).
# `noload` skips replay so a cleanly-unmounted image mounts, and a DIRTY one
# still mounts for inspection instead of silently reporting "missing file".
if mount -o ro,noload,loop "${LAST_DISKDIR}/volrw.ext4" "$MP" 2>"$OUT/mount-err.txt"; then
  echo "--- rw volume, loop-mounted read-only post-shutdown:"
  ls -la "$MP" | sed 's/^/    /'
  if [ -f "$MP/from-guest.txt" ]; then
    echo "    from-guest.txt content : $(cat "$MP/from-guest.txt")"
    printf 'GUEST-WROTE-THIS-0123456789-abcdefghij\n' >"$OUT/expected-from-guest.txt"
    if cmp -s "$OUT/expected-from-guest.txt" "$MP/from-guest.txt"; then
      echo "    +++ BYTE-IDENTICAL to what the guest reported writing"
    else
      echo "    !!! DIFFERS from expected"
    fi
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
echo "--- HOST-SIDE readonly=on enforcement (the [D8g] analogue for --disk)"
umount "$MP" 2>/dev/null
if mount -o ro,noload,loop "${LAST_DISKDIR}/volro.ext4" "$MP" 2>/dev/null; then
  ls -la "$MP" | sed 's/^/    /'
  if [ -f "$MP/guest-should-not-create.txt" ]; then
    echo "    !!! guest-should-not-create.txt EXISTS -> readonly=on NOT enforced"
  else
    echo "    +++ guest-should-not-create.txt absent -> the create was refused"
  fi
  if grep -q 'PREEXISTING-HOST-CONTENT-DO-NOT-CHANGE' "$MP/preexisting-host-file.txt" 2>/dev/null; then
    echo "    +++ preexisting file unchanged -> the overwrite was refused"
  else
    echo "    !!! MODIFIED -> readonly=on NOT enforced"
  fi
  umount "$MP"
fi
echo
echo "--- HOST-SIDE DIRECT-WRITE BASELINE (same syscalls, same filesystem)"
HOSTBASE="$VOLROOT/hostbase"; rm -rf "$HOSTBASE"; mkdir -p "$HOSTBASE/manyfiles"
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
print(f"    host {n} small files (open+write+fsync+close): {dt:.3f}s -> mean {dt*1000/n:.2f} ms/file")
PY
rm -rf "$HOSTBASE"
echo "=========================== END HOST VERIFICATION =========================="
echo
echo "--- increment-f mode=$MODE VERDICT: COMPLETED"
exit 0
