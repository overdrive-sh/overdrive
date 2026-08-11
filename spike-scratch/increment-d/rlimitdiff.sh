#!/usr/bin/env bash
# PROBE increment-d — the P5 x P6 interaction the first `full` run surfaced.
#
# Every attempt died with bash's "File size limit exceeded" and CH exit 153
# (= 128 + SIGXFSZ). 153 is TAXONOMY, not mechanism: it says a write would have
# exceeded RLIMIT_FSIZE, not WHICH file. The rootfs image is 96 MiB and the
# ceiling was 256 MiB, so the rootfs is not the culprit.
#
# Hypothesis:        `--memory shared=on` backs guest RAM with a memfd, and a
#                    memfd counts as a FILE for RLIMIT_FSIZE. So the ceiling
#                    must cover GUEST RAM, not just the disk image.
# Predicted outcome: with RLIMIT_FSIZE=256 MiB and 512 MiB of guest RAM,
#                    shared=on dies with SIGXFSZ and the identical run WITHOUT
#                    shared=on does not; raising the ceiling above guest RAM
#                    fixes shared=on; and the failure threshold tracks the
#                    --memory size, not the rootfs size.
# Falsification:     the no-shared run also dies (=> the rootfs or something
#                    else is the file), or raising the ceiling does not help
#                    (=> not an RLIMIT_FSIZE effect at all).
#
# No guest work is needed to answer this — the failure happens at VM creation —
# so each trial is short and the nested-virt stall is irrelevant.
set -uo pipefail

A_OUT=/var/tmp/spike-increment-a
OUT=/var/tmp/spike-increment-d
KERNEL="$A_OUT/Image"
CMDLINE="root=/dev/vda rw console=ttyAMA0 init=/init panic=1 loglevel=7"
VMM_USER=spikevmm
VMM_GID=6001

KVM_ORIG_MODE="$(stat -c %a /dev/kvm)"
trap 'chmod "$KVM_ORIG_MODE" /dev/kvm 2>/dev/null || true' EXIT
chown root:kvm /dev/kvm; chmod 0660 /dev/kvm

echo "### uname -r: $(uname -r)   CH: $(cloud-hypervisor --version)"
echo "### rootfs image size: $(stat -c %s "$OUT/rootfs.ext4") bytes ($(( $(stat -c %s "$OUT/rootfs.ext4") / 1024 / 1024 )) MiB)"
echo

trial() {
  local name="$1" mem="$2" fsize_mib="$3"
  local run="/run/spike-d-rl/$name"
  pkill -9 -f "cloud-hypervisor --cpus boot=" 2>/dev/null; sleep 0.2
  rm -rf "$run"; mkdir -p "$run"
  cp "$OUT/rootfs.ext4" "$run/rootfs.ext4"
  chown -R 6001:6001 "$run"; chmod 0700 "$run"

  timeout 12 \
    prlimit "--fsize=$((fsize_mib * 1024 * 1024))" --nofile=256 -- \
    setpriv --reuid="$VMM_USER" --regid="$VMM_GID" --init-groups --no-new-privs -- \
    cloud-hypervisor --cpus boot=1 --memory "$mem" \
      --kernel "$KERNEL" --cmdline "$CMDLINE" \
      --disk "path=$run/rootfs.ext4" \
      --serial "file=$run/console.log" --console off \
      --api-socket "path=$run/ch-api.sock" \
      --vsock "cid=3,socket=$run/ch.vsock" \
      --seccomp true --landlock --landlock-rules "path=$run,access=rw" \
      >"$run/ch.log" 2>&1
  local rc=$?
  local note
  case "$rc" in
    153) note="*** SIGXFSZ (128+25) — RLIMIT_FSIZE exceeded ***" ;;
    124) note="launched, killed at 12s (no rlimit failure)" ;;
    *)   note="$(grep -m1 -E '^(Error|error)' "$run/ch.log" 2>/dev/null | head -c 140)" ;;
  esac
  printf '  %-30s memory=%-22s RLIMIT_FSIZE=%-5s MiB  rc=%-4s %s\n' \
    "$name" "$mem" "$fsize_mib" "$rc" "$note"
}

echo "=== does shared=on, and only shared=on, trip RLIMIT_FSIZE?"
trial noshare-fsize256   "size=512M"          256
trial sharedon-fsize256  "size=512M,shared=on" 256
echo
echo "=== does raising the ceiling above GUEST RAM fix shared=on?"
trial sharedon-fsize768  "size=512M,shared=on" 768
trial sharedon-fsize1024 "size=512M,shared=on" 1024
echo
echo "=== does the threshold track --memory size rather than the rootfs size?"
trial sharedon-256M-fsize192 "size=256M,shared=on" 192
trial sharedon-256M-fsize384 "size=256M,shared=on" 384
echo
echo "=== is it really a memfd? (mappings of a shared=on VMM that survives)"
run=/run/spike-d-rl/inspect
pkill -9 -f "cloud-hypervisor --cpus boot=" 2>/dev/null; sleep 0.2
rm -rf "$run"; mkdir -p "$run"; cp "$OUT/rootfs.ext4" "$run/rootfs.ext4"
chown -R 6001:6001 "$run"; chmod 0700 "$run"
prlimit --fsize=$((1024*1024*1024)) --nofile=256 -- \
  setpriv --reuid="$VMM_USER" --regid="$VMM_GID" --init-groups --no-new-privs -- \
  cloud-hypervisor --cpus boot=1 --memory "size=512M,shared=on" \
    --kernel "$KERNEL" --cmdline "$CMDLINE" --disk "path=$run/rootfs.ext4" \
    --serial "file=$run/console.log" --console off \
    --api-socket "path=$run/ch-api.sock" \
    --seccomp true --landlock --landlock-rules "path=$run,access=rw" \
    >"$run/ch.log" 2>&1 &
p=$!
sleep 4
echo "--- /proc/$p/maps lines mentioning memfd or /dev/shm:"
grep -E 'memfd|/dev/shm' "/proc/$p/maps" 2>/dev/null | head -5 | sed 's/^/    /'
echo "--- open fds that are memfds:"
ls -l /proc/$p/fd 2>/dev/null | grep -i memfd | sed 's/^/    /'
echo "--- RssShmem (shared-backed resident pages):"
grep -E '^(VmRSS|RssAnon|RssShmem|RssFile):' "/proc/$p/status" 2>/dev/null | sed 's/^/    /'
kill -9 $p 2>/dev/null; wait $p 2>/dev/null
echo
echo "--- contrast: the same VMM WITHOUT shared=on"
run2=/run/spike-d-rl/inspect-noshare
rm -rf "$run2"; mkdir -p "$run2"; cp "$OUT/rootfs.ext4" "$run2/rootfs.ext4"
chown -R 6001:6001 "$run2"; chmod 0700 "$run2"
prlimit --fsize=$((1024*1024*1024)) --nofile=256 -- \
  setpriv --reuid="$VMM_USER" --regid="$VMM_GID" --init-groups --no-new-privs -- \
  cloud-hypervisor --cpus boot=1 --memory "size=512M" \
    --kernel "$KERNEL" --cmdline "$CMDLINE" --disk "path=$run2/rootfs.ext4" \
    --serial "file=$run2/console.log" --console off \
    --api-socket "path=$run2/ch-api.sock" \
    --seccomp true --landlock --landlock-rules "path=$run2,access=rw" \
    >"$run2/ch.log" 2>&1 &
p2=$!
sleep 4
echo "--- /proc/$p2/maps lines mentioning memfd or /dev/shm:"
grep -cE 'memfd|/dev/shm' "/proc/$p2/maps" 2>/dev/null | sed 's/^/    matches: /'
echo "--- open fds that are memfds:"
ls -l /proc/$p2/fd 2>/dev/null | grep -ci memfd | sed 's/^/    matches: /'
grep -E '^(VmRSS|RssAnon|RssShmem|RssFile):' "/proc/$p2/status" 2>/dev/null | sed 's/^/    /'
kill -9 $p2 2>/dev/null; wait $p2 2>/dev/null

echo
echo "--- (probe complete; the trailing kill -9 above is bookkeeping, not a failure)"
exit 0
