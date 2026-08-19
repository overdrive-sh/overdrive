#!/usr/bin/env bash
# PROBE increment-c — population diff for the P5 launch failure.
#
# The first confined run died with
#   Error booting VM: VmBoot(DeviceManager(CreateVsockBackend(UnixBind(EACCES))))
# EACCES is TAXONOMY, not mechanism (.claude/rules/debugging.md § 2): it names
# the layer that gave up, not which of the four confinement mechanisms caused
# it. So vary ONE mechanism at a time and diff the populations.
#
# Hypothesis:        the vsock UDS bind is refused by exactly one mechanism.
# Predicted outcome: the mechanism-isolating runs separate cleanly — the
#                    offending one reproduces EACCES, the others do not.
# Falsification:     every single-mechanism run succeeds (⇒ an interaction), or
#                    every run fails (⇒ something outside the four mechanisms).
#
# This diagnoses the LAUNCH, not the boot: each run is killed after a few
# seconds. Getting past device creation is the only signal wanted, so the
# nested-virt stall is irrelevant here.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
A_OUT=/var/tmp/spike-increment-a
KERNEL="$A_OUT/Image"
CMDLINE="root=/dev/vda rw console=ttyAMA0 init=/init panic=1 loglevel=7"
VMM_USER=spikevmm
VMM_GID=6001
RL_FSIZE=$((128 * 1024 * 1024))
RL_NOFILE=256

KVM_ORIG_MODE="$(stat -c %a /dev/kvm)"
trap 'chmod "$KVM_ORIG_MODE" /dev/kvm 2>/dev/null || true' EXIT
chown root:kvm /dev/kvm; chmod 0660 /dev/kvm

id "$VMM_USER" >/dev/null 2>&1 || {
  groupadd -g "$VMM_GID" "$VMM_USER" 2>/dev/null || true
  useradd -u 6001 -g "$VMM_GID" --system --no-create-home --shell /usr/sbin/nologin "$VMM_USER"
}
usermod -aG kvm "$VMM_USER" >/dev/null 2>&1

echo "### uname -r: $(uname -r)   CH: $(cloud-hypervisor --version)"
echo

trial() {
  local name="$1" use_uid="$2" use_rl="$3" use_ll="$4" extra_rules="${5:-}"
  local run="/run/spike-c-matrix/$name"
  pkill -9 -f "cloud-hypervisor --cpus boot=" 2>/dev/null; sleep 0.2
  rm -rf "$run"; mkdir -p "$run"
  cp "$A_OUT/rootfs.ext4" "$run/rootfs.ext4"
  [ "$use_uid" = 1 ] && { chown -R 6001:6001 "$run"; chmod 0700 "$run"; }

  local argv=(cloud-hypervisor --cpus boot=1 --memory size=512M
    --kernel "$KERNEL" --cmdline "$CMDLINE"
    --disk "path=$run/rootfs.ext4"
    --serial "file=$run/console.log" --console off
    --api-socket "path=$run/ch-api.sock"
    --vsock "cid=3,socket=$run/ch.vsock")
  [ "$use_ll" = 1 ] && argv+=(--seccomp true --landlock)
  if [ -n "$extra_rules" ]; then
    # shellcheck disable=SC2206
    local r=($extra_rules)
    for x in "${r[@]}"; do argv+=(--landlock-rules "$x"); done
  fi

  local prefix=()
  [ "$use_rl" = 1 ] && prefix+=(prlimit "--fsize=$RL_FSIZE" "--nofile=$RL_NOFILE" --)
  [ "$use_uid" = 1 ] && prefix+=(setpriv --reuid="$VMM_USER" --regid="$VMM_GID" --init-groups --no-new-privs --)

  timeout 8 "${prefix[@]}" "${argv[@]}" >"$run/ch.log" 2>&1
  local rc=$?
  local err
  err="$(grep -m1 -E 'Error|error' "$run/ch.log" 2>/dev/null | head -c 200)"
  if [ -z "$err" ]; then
    if grep -qE 'Run /init as init|HELLO from overdrive' "$run/console.log" 2>/dev/null; then
      err="<launched; guest reached /init>"
    elif [ -s "$run/console.log" ]; then
      err="<launched; kernel output, killed at 8s>"
    else
      err="<launched; no console output yet (nested-virt stall)>"
    fi
  fi
  printf '%-26s uid=%s rl=%s ll=%s  rc=%-3s  %s\n' "$name" "$use_uid" "$use_rl" "$use_ll" "$rc" "$err"
}

echo "=== population diff: one mechanism at a time"
trial baseline-root-none      0 0 0
trial uid-only                1 0 0
trial rlimit-only             0 1 0
trial landlock-only-root      0 0 1
trial uid+landlock            1 0 1
trial all-four                1 1 1
echo
echo "=== does an EXPLICIT landlock rule for the vsock/api directory fix it?"
mkdir -p /run/spike-c-matrix/expl
trial all-four+explicit-dir   1 1 1 "path=/run/spike-c-matrix/all-four+explicit-dir,access=rw"
echo
echo "=== full ch.log of the failing all-four run"
cat /run/spike-c-matrix/all-four/ch.log 2>/dev/null
echo
echo "=== full ch.log of all-four+explicit-dir"
cat "/run/spike-c-matrix/all-four+explicit-dir/ch.log" 2>/dev/null
