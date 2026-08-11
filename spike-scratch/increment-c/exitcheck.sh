#!/usr/bin/env bash
# PROBE increment-c — two follow-ups the first confined run raised.
#
# (1) SECCOMP. `/proc/<pid>/status` showed `Seccomp: 0` on a CH launched with
#     `--seccomp true`. The thread-group leader is not where CH installs its
#     filters, so the naive check is the wrong probe altitude
#     (.claude/rules/debugging.md § 7). Capture PER-THREAD seccomp instead.
#
# (2) SHUTDOWN. The confined run's guest printed `init: powering off
#     (RB_POWER_OFF)` but the kernel never printed `reboot: Power down`, and CH
#     had to be SIGKILLed at the 90 s watchdog (exit 137). increment-a's
#     unconfined run reached both. Is that a confinement effect or the known
#     nested-virt stall landing at the power-off boundary?
#
#     Hypothesis:        it is the nested-virt stall, not confinement.
#     Predicted outcome: repeated runs of BOTH modes sometimes reach
#                        `reboot: Power down` + CH exit 0 and sometimes do not.
#     Falsification:     confined runs NEVER reach power-down while baseline
#                        runs ALWAYS do -> confinement breaks VM shutdown, which
#                        would be a [D3] exit-reporting problem and a P5
#                        DOESN'T-WORK on that clause.
#
# Usage: exitcheck.sh [successful_boots_per_mode]
set -uo pipefail

WANT="${1:-3}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
A_OUT=/var/tmp/spike-increment-a
A_BIN="$(cd "$HERE/../increment-a" && pwd)/target/aarch64-unknown-linux-musl/release"
OUT=/var/tmp/spike-increment-c
KERNEL="$A_OUT/Image"
CMDLINE="root=/dev/vda rw console=ttyAMA0 init=/init panic=1 loglevel=7"
VMM_USER=spikevmm
VMM_GID=6001
RL_FSIZE=$((128 * 1024 * 1024))
RL_NOFILE=256

KVM_ORIG_MODE="$(stat -c %a /dev/kvm)"
trap 'chmod "$KVM_ORIG_MODE" /dev/kvm 2>/dev/null || true' EXIT
chown root:kvm /dev/kvm; chmod 0660 /dev/kvm

echo "### uname -r: $(uname -r)   CH: $(cloud-hypervisor --version)"
echo

one_boot() {
  local mode="$1" tag="$2"
  local run="/run/spike-c-exit/$tag"
  pkill -9 -f "cloud-hypervisor --cpus boot=" 2>/dev/null; sleep 0.2
  rm -rf "$run"; mkdir -p "$run"
  cp "$A_OUT/rootfs.ext4" "$run/rootfs.ext4"
  [ "$mode" = confined ] && { chown -R 6001:6001 "$run"; chmod 0700 "$run"; }

  "$A_BIN/host-listener" "$run/ch.vsock_1234" >"$run/host.log" 2>&1 &
  local lp=$!
  for _ in $(seq 1 100); do [ -S "$run/ch.vsock_1234" ] && break; sleep 0.05; done
  chmod 0666 "$run/ch.vsock_1234" 2>/dev/null

  local argv=(cloud-hypervisor --cpus boot=1 --memory size=512M
    --kernel "$KERNEL" --cmdline "$CMDLINE"
    --disk "path=$run/rootfs.ext4"
    --serial "file=$run/console.log" --console off
    --api-socket "path=$run/ch-api.sock"
    --vsock "cid=3,socket=$run/ch.vsock")
  local prefix=()
  if [ "$mode" = confined ]; then
    argv+=(--seccomp true --landlock --landlock-rules "path=$run,access=rw")
    prefix=(prlimit "--fsize=$RL_FSIZE" "--nofile=$RL_NOFILE" --
            setpriv --reuid="$VMM_USER" --regid="$VMM_GID" --init-groups --no-new-privs --)
  fi

  "${prefix[@]}" "${argv[@]}" >"$run/ch.log" 2>&1 &
  local pid=$!

  # ---- per-thread seccomp, captured while live -------------------------
  ( for _ in $(seq 1 14); do
      sleep 0.5
      [ -d "/proc/$pid/task" ] || continue
      grep -q '^VmRSS' "/proc/$pid/status" 2>/dev/null || continue
      {
        echo "### mode=$mode  pid=$pid  thread count: $(ls /proc/$pid/task 2>/dev/null | wc -l)"
        echo "### thread-group leader /proc/$pid/status Seccomp line:"
        grep -E '^Seccomp' "/proc/$pid/status" 2>/dev/null
        echo "### PER-THREAD (comm : Seccomp : Seccomp_filters : NoNewPrivs):"
        for t in /proc/$pid/task/*; do
          printf '  %-18s %s\n' "$(cat "$t/comm" 2>/dev/null)" \
            "$(grep -E '^(Seccomp|Seccomp_filters|NoNewPrivs):' "$t/status" 2>/dev/null | tr '\n\t' ' ; ' )"
        done
        echo "### distinct Seccomp modes across all threads:"
        cat /proc/$pid/task/*/status 2>/dev/null | grep -E '^Seccomp:' | sort | uniq -c
      } >"$OUT/seccomp-$mode.txt" 2>&1
    done ) &
  local cp=$!

  ( sleep 60; kill -9 "$pid" 2>/dev/null ) & local wd=$!
  wait "$pid"; local rc=$?
  kill "$wd" 2>/dev/null; wait "$wd" 2>/dev/null; wait "$cp" 2>/dev/null
  for _ in $(seq 1 40); do kill -0 "$lp" 2>/dev/null || break; sleep 0.1; done
  kill -9 "$lp" 2>/dev/null; wait "$lp" 2>/dev/null; local lrc=$?

  local beacon=no powerdown=no poweroff_req=no
  [ "$lrc" = 0 ] && beacon=yes
  grep -q 'reboot: Power down' "$run/console.log" 2>/dev/null && powerdown=yes
  grep -q 'powering off (RB_POWER_OFF)' "$run/console.log" 2>/dev/null && poweroff_req=yes
  printf '  %-9s beacon=%-3s init_requested_poweroff=%-3s kernel_powered_down=%-3s ch_exit=%s\n' \
    "$mode" "$beacon" "$poweroff_req" "$powerdown" "$rc"
  [ "$beacon" = yes ] && return 0 || return 1
}

for mode in baseline confined; do
  echo "=== mode=$mode — collecting $WANT successful boots (retrying past stalls)"
  got=0; att=0
  while [ "$got" -lt "$WANT" ] && [ "$att" -lt 25 ]; do
    att=$((att+1))
    if one_boot "$mode" "$mode-$att"; then got=$((got+1)); fi
  done
  echo "    -> $got successful boots out of $att attempts"
  echo
done

echo "=========================== PER-THREAD SECCOMP =============================="
for m in baseline confined; do
  echo "----- $m -----"
  cat "$OUT/seccomp-$m.txt" 2>/dev/null || echo "<not captured>"
  echo
done
echo "=========================== END PER-THREAD SECCOMP =========================="
