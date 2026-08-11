#!/usr/bin/env bash
# PROBE increment-a — P1 (kernel boots under CH from ext4 virtio-blk) and
# P2 (vsock beacon + exit status), in TWO placements:
#   run.sh host     -> cloud-hypervisor in the HOST network namespace
#   run.sh netns    -> cloud-hypervisor setns'd into a fresh netns via
#                      `ip netns exec`, host listener staying in the host netns
#
# Run as root inside the overdrive Lima VM.
set -uo pipefail

PLACEMENT="${1:-host}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT=/var/tmp/spike-increment-a
RUN="/run/spike-increment-a/$PLACEMENT"
ARCH="$(uname -m)"
TARGET="${ARCH}-unknown-linux-musl"
BIN_DIR="$HERE/target/$TARGET/release"

# Kernel prepared by build.sh: a bzImage copy on x86_64, an unwrapped raw
# Image on aarch64. Same path either way.
KERNEL="$OUT/kernel"
# SERIAL CONSOLE NAME IS ARCH-SPECIFIC and there is no error if you get it
# wrong — the guest just produces no console output at all, which reads
# exactly like a hang:
#   x86_64  -> ttyS0   (8250/16550 UART)
#   aarch64 -> ttyAMA0 (ARM PL011)
case "$ARCH" in
  x86_64)  CONSOLE_DEV=ttyS0  ;;
  aarch64) CONSOLE_DEV=ttyAMA0 ;;
  *) echo "unsupported arch: $ARCH" >&2; exit 1 ;;
esac
CMDLINE="root=/dev/vda rw console=${CONSOLE_DEV} init=/init panic=1 loglevel=7"
NETNS=spikens
VSOCK_CID=3
VSOCK_PORT=1234

# A stalled VMM from a previous run would hold the vsock UDS. Never inherit one.
# NOTE: `pkill -x cloud-hypervisor` silently matches NOTHING — /proc comm is
# capped at 15 chars (TASK_COMM_LEN) and "cloud-hypervisor" is 17. Match argv.
pkill -9 -f "cloud-hypervisor --cpus boot=" 2>/dev/null; sleep 0.3

rm -rf "$RUN"; mkdir -p "$RUN"
VSOCK_UDS="$RUN/ch.vsock"
LISTEN_UDS="${VSOCK_UDS}_${VSOCK_PORT}"
CONSOLE="$RUN/console.log"
CHLOG="$RUN/ch-stderr.log"
HOSTLOG="$RUN/host-listener.log"

# Per-run rootfs copy (the production model is a copy-per-launch anyway).
cp "$OUT/rootfs.ext4" "$RUN/rootfs.ext4"

echo "##########################################################"
echo "### PROBE increment-a  placement=$PLACEMENT"
echo "### uname -r          : $(uname -r)"
echo "### uname -m          : $(uname -m)"
echo "### cloud-hypervisor  : $(cloud-hypervisor --version 2>&1)"
echo "### kernel image      : $KERNEL"
echo "###   file            : $(file -b $KERNEL)"
echo "##########################################################"
echo

# ---------------------------------------------------------------- listener
echo "--- starting host listener (stays in HOST netns) on $LISTEN_UDS"
"$BIN_DIR/host-listener" "$LISTEN_UDS" >"$HOSTLOG" 2>&1 &
LISTENER_PID=$!
# Wait for the socket to appear before booting the VM.
for _ in $(seq 1 100); do [ -S "$LISTEN_UDS" ] && break; sleep 0.05; done
LISTENER_NETNS=$(readlink /proc/$LISTENER_PID/ns/net 2>/dev/null || echo "<gone>")
echo "--- listener pid=$LISTENER_PID netns=$LISTENER_NETNS"
ls -la "$LISTEN_UDS" || { echo "!!! listener socket never appeared"; cat "$HOSTLOG"; exit 1; }
echo

# ---------------------------------------------------------------- netns prep
CH_PREFIX=()
if [ "$PLACEMENT" = "netns" ] || [ "$PLACEMENT" = "netns-novsock" ]; then
  ip netns del "$NETNS" 2>/dev/null || true
  ip netns add "$NETNS"
  echo "--- created netns '$NETNS'; its link set:"
  ip netns exec "$NETNS" ip -br link show
  echo "--- host netns link set (for contrast):"
  ip -br link show
  CH_PREFIX=(ip netns exec "$NETNS")
  echo
fi

CH_ARGV=(
  cloud-hypervisor
  --cpus "boot=${SPIKE_VCPUS:-1}"
  --memory size=512M
  --kernel "$KERNEL"
  --cmdline "$CMDLINE"
  --disk "path=$RUN/rootfs.ext4"
  --serial "file=$CONSOLE"
  --console off
)
# `novsock` placements drop the vsock device entirely. This is the population
# diff that separates "vsock stalls the guest" from "the nested boot is flaky".
case "$PLACEMENT" in
  *novsock*) echo "--- (vsock device DELIBERATELY omitted for this placement)" ;;
  *) CH_ARGV+=(--vsock "cid=$VSOCK_CID,socket=$VSOCK_UDS") ;;
esac

echo "--- exact CH argv:"
printf '    %q' "${CH_PREFIX[@]}" "${CH_ARGV[@]}"; echo
echo

# ---------------------------------------------------------------- boot
# No `timeout` wrapper here: `ip netns exec` setns()es and then execve()s IN
# PLACE (no fork), so $! IS the cloud-hypervisor pid in BOTH placements. That
# makes /proc/$CH_PID/ns/net the VMM's own namespace rather than a wrapper's.
"${CH_PREFIX[@]}" "${CH_ARGV[@]}" >"$CHLOG" 2>&1 &
CH_PID=$!
sleep 2
CH_NETNS=$(readlink /proc/$CH_PID/ns/net 2>/dev/null || echo "<already-exited>")
CH_NS_NAME=$(ip netns identify "$CH_PID" 2>/dev/null || true)
echo "--- ip netns identify $CH_PID -> '${CH_NS_NAME:-<host netns / unnamed>}'"
echo "--- cloud-hypervisor pid=$CH_PID netns=$CH_NETNS"
echo "--- netns comparison: listener=$LISTENER_NETNS  vmm=$CH_NETNS"
if [ "$PLACEMENT" = "netns" ]; then
  if [ "$LISTENER_NETNS" = "$CH_NETNS" ]; then
    echo "!!! NETNS PLACEMENT DID NOT TAKE EFFECT — same net namespace inode"
  else
    echo "+++ VMM IS IN A DIFFERENT NETWORK NAMESPACE THAN THE LISTENER"
  fi
fi
echo

# ---------------------------------------------------------------- collect
( sleep 90; kill -9 $CH_PID 2>/dev/null ) &
WATCHDOG=$!
wait $CH_PID; CH_RC=$?
kill $WATCHDOG 2>/dev/null; wait $WATCHDOG 2>/dev/null

# Give the listener a moment to drain and exit.
for _ in $(seq 1 60); do kill -0 $LISTENER_PID 2>/dev/null || break; sleep 0.1; done
kill -9 $LISTENER_PID 2>/dev/null
wait $LISTENER_PID 2>/dev/null; LISTENER_RC=$?

echo "=========================== GUEST SERIAL CONSOLE ==========================="
cat "$CONSOLE" 2>/dev/null || echo "<no console output captured>"
echo "=========================== END SERIAL CONSOLE ============================="
echo
echo "=========================== HOST VSOCK LISTENER ==========================="
cat "$HOSTLOG" 2>/dev/null || echo "<no listener output>"
echo "=========================== END HOST LISTENER =============================="
echo
echo "--- cloud-hypervisor stderr:"
cat "$CHLOG" 2>/dev/null
echo
echo "--- cloud-hypervisor exit code : $CH_RC"
echo "--- host listener exit code    : $LISTENER_RC   (0 = beacon+EXIT 7 in order)"

if [ "$PLACEMENT" = "netns" ]; then ip netns del "$NETNS" 2>/dev/null || true; fi

echo "--- artifacts under $RUN"
exit $LISTENER_RC
