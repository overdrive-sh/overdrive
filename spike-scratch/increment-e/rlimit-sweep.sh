#!/usr/bin/env bash
# PROBE increment-e — isolate the RLIMIT_FSIZE x memfd interaction on x86_64.
#
# env A (aarch64) found: `--memory shared=on` backs guest RAM with a memfd, and
# a memfd is a FILE for RLIMIT_FSIZE, so CH dies with SIGXFSZ (exit 153) the
# moment guest RAM exceeds the ceiling. The threshold tracked `--memory size`,
# NOT the rootfs image. Sizing the limit off the rootfs — the obvious thing to
# do, and what increment-c did — makes every volume-carrying VM die with an
# opaque signal.
#
# That is a mechanism claim, so it is re-measured here rather than carried over
# from a different architecture.
#
# Each trial boots far enough to prove the point and is then killed, so the
# outcome under test is the EXIT SIGNAL, not a completed run:
#   rc=153  -> 128+SIGXFSZ, the rlimit fired
#   rc=137  -> 128+SIGKILL, our watchdog; the rlimit did NOT fire
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT=/var/tmp/spike-increment-e
ARCH="$(uname -m)"
BIN_DIR="$HERE/target/${ARCH}-unknown-linux-musl/release"
RUN=/run/spike-increment-e-rl
KERNEL="$OUT/kernel"
VMM_USER=spikevmm
VMM_GID=6001
case "$ARCH" in
  x86_64)  CONSOLE_DEV=ttyS0  ;;
  aarch64) CONSOLE_DEV=ttyAMA0 ;;
esac

trial() {   # <label> <mem-argv> <fsize-MiB>
  local label="$1" mem="$2" fsize_mib="$3"
  rm -rf "$RUN"; mkdir -p "$RUN"
  cp "$OUT/rootfs.ext4" "$RUN/rootfs.ext4"
  chown -R "$VMM_USER:$VMM_GID" "$RUN"; chmod 0700 "$RUN"

  prlimit "--fsize=$((fsize_mib * 1024 * 1024))" --nofile=256 -- \
    setpriv --reuid="$VMM_USER" --regid="$VMM_GID" --init-groups --no-new-privs -- \
    cloud-hypervisor \
      --cpus boot=1 --memory "$mem" \
      --kernel "$KERNEL" \
      --cmdline "root=/dev/vda rw console=${CONSOLE_DEV} init=/init panic=1" \
      --disk "path=$RUN/rootfs.ext4,image_type=raw" \
      --serial "file=$RUN/console.log" --console off \
      --api-socket "path=$RUN/ch-api.sock" \
      --seccomp true --landlock --landlock-rules "path=$RUN,access=rw" \
      >"$RUN/ch.log" 2>&1 &
  local pid=$!
  # Suppress bash's own async "File size limit exceeded (core dumped)" job
  # report: it is emitted on the NEXT prompt, so it lands under the following
  # trial's line and reads as though that trial failed.
  set +m 2>/dev/null
  ( sleep 25; kill -9 "$pid" 2>/dev/null ) &
  local wd=$!
  wait "$pid"; local rc=$?
  kill "$wd" 2>/dev/null; wait "$wd" 2>/dev/null

  local verdict
  case "$rc" in
    153) verdict="*** SIGXFSZ — the rlimit FIRED ***" ;;
    137) verdict="no rlimit failure (still running at 25s, watchdog killed it)" ;;
    0)   verdict="no rlimit failure (guest ran to completion and powered off)" ;;
    *)   verdict="no rlimit failure (unexpected exit $rc — inspect)" ;;
  esac
  printf '  %-24s %-24s FSIZE=%4s MiB  rc=%-4s %s\n' \
    "$label" "$mem" "$fsize_mib" "$rc" "$verdict"
  # The rootfs image is 96 MiB in every trial, so any threshold that moves with
  # `--memory size` cannot be explained by the disk.
  grep -qi 'file size' "$RUN/ch.log" 2>/dev/null && \
    sed 's/^/      ch: /' "$RUN/ch.log" | head -2
}

echo "  rootfs image is 96 MiB in EVERY trial below — so a threshold that tracks"
echo "  '--memory size' cannot be explained by the disk."
echo
trial noshare-fsize256      "size=512M"           256
trial sharedon-fsize256     "size=512M,shared=on" 256
trial sharedon-fsize768     "size=512M,shared=on" 768
trial sharedon-256M-fsize192 "size=256M,shared=on" 192
trial sharedon-256M-fsize384 "size=256M,shared=on" 384
rm -rf "$RUN"
