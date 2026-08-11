#!/usr/bin/env bash
# PROBE increment-c — P5: do the [D7] confinement flags compose with a real boot?
#
#   (a) --landlock with a ruleset covering only this VM's kernel, rootfs copy
#       and API socket
#   (b) a NON-ROOT uid/gid
#   (c) reduced RLIMIT_FSIZE and RLIMIT_NOFILE
#   (d) seccomp at its default (never false/log)
#
# ...all four at once, on the SAME VM that booted in increment-a (P1/P2).
#
# Usage: run.sh [confined|baseline] [max_attempts]
#   confined  — all four confinement mechanisms on (the P5 hypothesis)
#   baseline  — none of them (the control: same VM, root, unconfined)
#
# The nested-virt stall (increment-a findings § "The nested-virt stall") means
# roughly 2 in 3 boots freeze before /init. That is an environment property, not
# a CH property, and it never produces a WRONG answer — only a missing one. So
# this script RETRIES until it gets a boot rather than treating a stall as a
# verdict.
#
# Run as root inside the overdrive Lima VM.
set -uo pipefail

MODE="${1:-confined}"
MAX_ATTEMPTS="${2:-8}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
A_OUT=/var/tmp/spike-increment-a
A_BIN="$(cd "$HERE/../increment-a" && pwd)/target/aarch64-unknown-linux-musl/release"
OUT=/var/tmp/spike-increment-c
RUN=/run/spike-increment-c

KERNEL="$A_OUT/Image"
CMDLINE="root=/dev/vda rw console=ttyAMA0 init=/init panic=1 loglevel=7"
VSOCK_CID=3
VSOCK_PORT=1234

# --- the confinement parameters under test -------------------------------
VMM_USER=spikevmm
VMM_UID=6001
VMM_GID=6001
# RLIMIT_FSIZE must be >= the largest file the VMM writes. The rootfs copy is
# 64 MiB, so 128 MiB is a real reduction from `unlimited` that still leaves
# headroom. This is a *per-VM computed* value, not a constant — see findings.
RL_FSIZE=$((128 * 1024 * 1024))
# Root's default here is 1024. 256 is a real reduction that still covers
# 1 vCPU + 1 disk + 1 vsock + 1 serial.
RL_NOFILE=256

mkdir -p "$OUT"

##########################################################################
# /dev/kvm permission model — this is the OPEN DESIGN QUESTION, so test the
# PRODUCTION-REALISTIC shape (0660 root:kvm + group membership), not the one
# the Lima udev rule (MODE="0666") makes easy.
##########################################################################
KVM_ORIG_MODE="$(stat -c %a /dev/kvm)"
KVM_ORIG_OWNER="$(stat -c %U:%G /dev/kvm)"
restore_kvm() {
  chmod "$KVM_ORIG_MODE" /dev/kvm 2>/dev/null || true
  echo "--- restored /dev/kvm to mode $KVM_ORIG_MODE ($(stat -c '%A %U:%G' /dev/kvm))"
}
trap restore_kvm EXIT

echo "##########################################################"
echo "### PROBE increment-c (P5)  mode=$MODE"
echo "### uname -r          : $(uname -r)"
echo "### uname -m          : $(uname -m)"
echo "### cloud-hypervisor  : $(cloud-hypervisor --version 2>&1)"
echo "### active LSMs       : $(cat /sys/kernel/security/lsm)"
echo "##########################################################"
echo

echo "=== [setup] unprivileged VMM identity"
if ! id "$VMM_USER" >/dev/null 2>&1; then
  groupadd -g "$VMM_GID" "$VMM_USER" 2>/dev/null || true
  useradd -u "$VMM_UID" -g "$VMM_GID" --system --no-create-home \
          --shell /usr/sbin/nologin "$VMM_USER"
fi
usermod -aG kvm "$VMM_USER"
echo "--- id $VMM_USER : $(id "$VMM_USER")"

echo
echo "=== [setup] /dev/kvm — testing the PRODUCTION-REALISTIC 0660 root:kvm shape"
echo "--- before : $(stat -c '%A %U:%G' /dev/kvm)   (Lima udev rule installs MODE=0666)"
chown root:kvm /dev/kvm
chmod 0660 /dev/kvm
echo "--- now    : $(stat -c '%A %U:%G' /dev/kvm)"
echo "--- can the unprivileged, kvm-group VMM identity open it?"
setpriv --reuid="$VMM_USER" --regid="$VMM_GID" --init-groups --no-new-privs \
  -- /bin/sh -c 'id; exec 3<>/dev/kvm && echo "+++ open(/dev/kvm, O_RDWR) OK as $(id -un)" || echo "!!! open(/dev/kvm) FAILED as $(id -un)"'
echo

echo "=== [setup] CH --landlock-rules is genuinely parsed (not silently ignored)"
echo "--- control: hand CH a rule for a path that does not exist"
timeout 20 cloud-hypervisor --landlock \
  --landlock-rules "path=$OUT/definitely-not-here,access=rw" \
  --cpus boot=1 --memory size=128M --kernel "$KERNEL" --cmdline "$CMDLINE" \
  --disk "path=$A_OUT/rootfs.ext4" --console off --serial null 2>&1 | head -5
echo "--- (a non-zero/complaining result above means the flag reaches the ruleset builder)"
echo

##########################################################################
boot_once() {
  local attempt=$1
  local vm_run="$RUN/vm-a"
  rm -rf "$RUN"; mkdir -p "$vm_run" "$RUN/vm-b"

  local vsock_uds="$vm_run/ch.vsock"
  local listen_uds="${vsock_uds}_${VSOCK_PORT}"
  local console="$vm_run/console.log"
  local chlog="$vm_run/ch-stderr.log"
  local hostlog="$vm_run/host-listener.log"
  local api_sock="$vm_run/ch-api.sock"

  cp "$A_OUT/rootfs.ext4" "$vm_run/rootfs.ext4"
  # A SIBLING VM's rootfs. This is the path that matters: the isolation claim
  # is "VM A's confined hypervisor cannot reach VM B's disk", and a deny target
  # that does not exist would return ENOENT and prove nothing.
  cp "$A_OUT/rootfs.ext4" "$RUN/vm-b/rootfs.ext4"

  if [ "$MODE" = confined ]; then
    chown -R "$VMM_UID:$VMM_GID" "$vm_run"
    chmod 0700 "$vm_run"
  fi

  # The listener stays root, in the host netns (P2 already established the
  # netns half; this probe varies confinement, not placement).
  "$A_BIN/host-listener" "$listen_uds" >"$hostlog" 2>&1 &
  local listener_pid=$!
  for _ in $(seq 1 100); do [ -S "$listen_uds" ] && break; sleep 0.05; done
  [ -S "$listen_uds" ] || { echo "!!! listener socket never appeared"; cat "$hostlog"; return 99; }
  # The uid-dropped VMM must be able to connect() to it.
  chmod 0666 "$listen_uds"

  local CH_ARGV=(
    cloud-hypervisor
    --cpus boot=1
    --memory size=512M
    --kernel "$KERNEL"
    --cmdline "$CMDLINE"
    --disk "path=$vm_run/rootfs.ext4"
    --serial "file=$console"
    --console off
    --api-socket "path=$api_sock"
    --vsock "cid=$VSOCK_CID,socket=$vsock_uds"
  )
  local PREFIX=()
  if [ "$MODE" = confined ]; then
    # matrix2.sh finding: CH v46 auto-derives Landlock rules for --kernel,
    # --disk, --serial file= and --api-socket, but NOT for the vsock UDS it
    # binds itself. Without this explicit rw rule the boot dies with an opaque
    # CreateVsockBackend(UnixBind(EACCES)) that never mentions Landlock. The
    # rule must name the CONTAINING DIRECTORY — CH validates rule paths for
    # existence at config time and the socket does not exist yet.
    CH_ARGV+=(--seccomp true --landlock --landlock-rules "path=$vm_run,access=rw")
    # (c) rlimits, then (b) uid/gid drop. Both execve in place, so $! below is
    # the cloud-hypervisor pid itself, not a wrapper's.
    PREFIX=(prlimit "--fsize=$RL_FSIZE" "--nofile=$RL_NOFILE" --
            setpriv --reuid="$VMM_USER" --regid="$VMM_GID" --init-groups --no-new-privs --)
  fi

  if [ "$attempt" = 1 ]; then
    echo "--- exact launch argv:"
    printf '    %q' "${PREFIX[@]}" "${CH_ARGV[@]}"; echo
    echo
  fi

  "${PREFIX[@]}" "${CH_ARGV[@]}" >"$chlog" 2>&1 &
  local ch_pid=$!

  # ---- capture /proc WHILE THE VM IS LIVE (beacon lands ~t+8.7s) ----------
  # Poll rather than sleeping a fixed interval: a run that dies early would
  # otherwise leave an empty capture that reads as "no evidence" instead of
  # "the VMM was already gone".
  local capture="$OUT/proc-capture-$MODE.txt"
  ( for _ in $(seq 1 14); do
      sleep 0.5
      [ -r "/proc/$ch_pid/status" ] || continue
      grep -q '^VmRSS' "/proc/$ch_pid/status" 2>/dev/null || continue
    {
      echo "############ /proc/$ch_pid captured WHILE VM LIVE (mode=$MODE) ############"
      echo "### capture wall clock: $(date -Is)"
      echo "### comm    : $(cat /proc/$ch_pid/comm 2>/dev/null)"
      echo "### cmdline : $(tr '\0' ' ' </proc/$ch_pid/cmdline 2>/dev/null)"
      echo "### ---------------- /proc/$ch_pid/status (selected) ----------------"
      grep -E '^(Name|Umask|State|Pid|PPid|Uid|Gid|Groups|Seccomp|Seccomp_filters|NoNewPrivs|CapEff|CapBnd|CapPrm|CapInh):' \
        /proc/$ch_pid/status 2>/dev/null
      echo "### ---------------- /proc/$ch_pid/limits ----------------"
      cat /proc/$ch_pid/limits 2>/dev/null
      echo "### ---------------- open fd count ----------------"
      echo "fds open: $(ls /proc/$ch_pid/fd 2>/dev/null | wc -l)"
      echo "### ---------------- VmRSS / VmPeak ----------------"
      grep -E '^(VmPeak|VmSize|VmRSS|RssAnon|RssShmem|RssFile):' /proc/$ch_pid/status 2>/dev/null
    } >"$capture.tmp" 2>&1
    done
  ) &
  local cap_pid=$!

  ( sleep 90; kill -9 "$ch_pid" 2>/dev/null ) &
  local watchdog=$!
  wait "$ch_pid"; local ch_rc=$?
  kill "$watchdog" 2>/dev/null; wait "$watchdog" 2>/dev/null
  wait "$cap_pid" 2>/dev/null

  for _ in $(seq 1 60); do kill -0 "$listener_pid" 2>/dev/null || break; sleep 0.1; done
  kill -9 "$listener_pid" 2>/dev/null
  wait "$listener_pid" 2>/dev/null; local listener_rc=$?

  LAST_CONSOLE="$console"; LAST_HOSTLOG="$hostlog"; LAST_CHLOG="$chlog"
  LAST_CH_RC="$ch_rc"; LAST_CAPTURE="$capture"; LAST_VM_RUN="$vm_run"
  [ -f "$capture.tmp" ] && mv "$capture.tmp" "$capture"
  return $listener_rc
}

##########################################################################
echo "=== [boot] attempting the confined boot (retrying past nested-virt stalls)"
BOOT_RC=1
for attempt in $(seq 1 "$MAX_ATTEMPTS"); do
  echo "--- attempt $attempt/$MAX_ATTEMPTS"
  boot_once "$attempt"; BOOT_RC=$?
  if [ "$BOOT_RC" = 0 ]; then echo "--- attempt $attempt SUCCEEDED"; break; fi
  echo "--- attempt $attempt did not reach the beacon (rc=$BOOT_RC); ch_rc=${LAST_CH_RC:-?}"
  tail -3 "${LAST_CONSOLE:-/dev/null}" 2>/dev/null
done
echo

echo "=========================== /proc CAPTURE (VM LIVE) ========================="
cat "${LAST_CAPTURE:-/dev/null}" 2>/dev/null || echo "<none captured>"
echo "=========================== END /proc CAPTURE ==============================="
echo
echo "=========================== GUEST SERIAL CONSOLE ==========================="
cat "${LAST_CONSOLE:-/dev/null}" 2>/dev/null || echo "<no console output captured>"
echo "=========================== END SERIAL CONSOLE ============================="
echo
echo "=========================== HOST VSOCK LISTENER ==========================="
cat "${LAST_HOSTLOG:-/dev/null}" 2>/dev/null || echo "<no listener output>"
echo "=========================== END HOST LISTENER =============================="
echo
echo "--- cloud-hypervisor stderr:"
cat "${LAST_CHLOG:-/dev/null}" 2>/dev/null
echo "--- cloud-hypervisor exit code : ${LAST_CH_RC:-?}"
echo "--- P5 BOOT VERDICT            : $([ "$BOOT_RC" = 0 ] && echo 'BOOTED + BEACON + EXIT 7' || echo 'NO BEACON')"
echo

##########################################################################
# The denial. US-VM-7 AC 1(b) evidence — cannot be reconstructed later.
##########################################################################
if [ "$MODE" = confined ]; then
  echo "=========================== LANDLOCK DENIAL PROBE =========================="
  echo "--- running as root ON PURPOSE: root bypasses DAC, so any EACCES below is"
  echo "--- necessarily Landlock and not file permissions."
  echo "--- id: $(id)"
  echo
  "$HERE/target/release/landlock-denial" \
    "ro:$KERNEL" \
    "rw:${LAST_VM_RUN:-$RUN/vm-a}" \
    "rw:/dev/kvm" \
    -- \
    "allow:$KERNEL" \
    "allow:${LAST_VM_RUN:-$RUN/vm-a}/rootfs.ext4" \
    "allow:/dev/kvm" \
    "deny:$OUT/SENTINEL-OUTSIDE-RULESET" \
    "deny:$RUN/vm-b/rootfs.ext4" \
    "deny:$A_OUT/rootfs.ext4" \
    "deny:/etc/shadow"
  DENIAL_RC=$?
  echo "--- denial probe exit code: $DENIAL_RC  (0 = every deny: refused)"
  echo "=========================== END DENIAL PROBE ==============================="
fi

echo
echo "--- artifacts under $RUN and $OUT"
exit $BOOT_RC
