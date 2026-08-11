#!/usr/bin/env bash
# PROBE increment-c — second population diff: WHICH paths does CH v46's
# implicit Landlock ruleset cover, and which does it miss?
#
# matrix.sh established that `--landlock` alone (not uid-drop, not rlimits)
# refuses the vsock UDS bind, and that one explicit --landlock-rules for the
# containing directory fixes it. This narrows that to a per-device answer, so
# [D7] / US-VM-7 can name the rule list instead of guessing it.
#
# Hypothesis:        CH auto-derives rules for --kernel / --disk / --serial but
#                    NOT for the sockets it creates itself (--vsock, --api-socket).
# Predicted outcome: the no-socket run boots with bare --landlock; adding either
#                    socket without an explicit rule reproduces EACCES.
# Falsification:     bare --landlock fails even with no sockets (⇒ CH derives
#                    nothing), or the socket runs succeed (⇒ some other cause).
set -uo pipefail

A_OUT=/var/tmp/spike-increment-a
KERNEL="$A_OUT/Image"
CMDLINE="root=/dev/vda rw console=ttyAMA0 init=/init panic=1 loglevel=7"

echo "### uname -r: $(uname -r)   CH: $(cloud-hypervisor --version)"
echo

trial() {
  local name="$1"; shift
  local run="/run/spike-c-matrix2/$name"
  pkill -9 -f "cloud-hypervisor --cpus boot=" 2>/dev/null; sleep 0.2
  rm -rf "$run"; mkdir -p "$run"
  cp "$A_OUT/rootfs.ext4" "$run/rootfs.ext4"

  local argv=(cloud-hypervisor --cpus boot=1 --memory size=512M
    --kernel "$KERNEL" --cmdline "$CMDLINE"
    --disk "path=$run/rootfs.ext4"
    --serial "file=$run/console.log" --console off
    --seccomp true --landlock)
  # Remaining args are templated with __RUN__ so each trial gets its own dir.
  for a in "$@"; do argv+=("${a//__RUN__/$run}"); done

  timeout 8 "${argv[@]}" >"$run/ch.log" 2>&1
  local rc=$?
  local err
  err="$(grep -m1 -E '^Error' "$run/ch.log" 2>/dev/null | head -c 170)"
  if [ -z "$err" ]; then
    if grep -qE 'Run /init as init|HELLO from overdrive' "$run/console.log" 2>/dev/null; then
      err="LAUNCHED — guest reached /init"
    elif [ -s "$run/console.log" ]; then
      err="LAUNCHED — kernel output present"
    else
      err="LAUNCHED — no console yet (nested-virt stall)"
    fi
  fi
  printf '%-34s rc=%-4s %s\n' "$name" "$rc" "$err"
}

echo "=== which device makes the implicit ruleset insufficient?"
trial no-sockets-at-all
trial api-socket-only        --api-socket "path=__RUN__/ch-api.sock"
trial vsock-only             --vsock "cid=3,socket=__RUN__/ch.vsock"
trial both-sockets           --api-socket "path=__RUN__/ch-api.sock" --vsock "cid=3,socket=__RUN__/ch.vsock"
echo
echo "=== with an explicit rw rule on the socket directory"
trial vsock-only+dir-rule    --vsock "cid=3,socket=__RUN__/ch.vsock" \
                             --landlock-rules "path=__RUN__,access=rw"
trial both-sockets+dir-rule  --api-socket "path=__RUN__/ch-api.sock" --vsock "cid=3,socket=__RUN__/ch.vsock" \
                             --landlock-rules "path=__RUN__,access=rw"
echo
echo "=== is a READ-ONLY rule on the socket directory enough? (no => needs rw)"
trial vsock-only+dir-ro-rule --vsock "cid=3,socket=__RUN__/ch.vsock" \
                             --landlock-rules "path=__RUN__,access=r"
echo
echo "=== can a rule name the socket PATH itself rather than its directory?"
trial vsock-only+path-rule   --vsock "cid=3,socket=__RUN__/ch.vsock" \
                             --landlock-rules "path=__RUN__/ch.vsock,access=rw"
echo
echo "=== does landlock deny a disk OUTSIDE the config? (CH-side denial attempt)"
# A second disk whose path CH itself put in the config -> auto-added, so this
# is expected to WORK. Recorded to show the auto-derivation is real.
cp "$A_OUT/rootfs.ext4" /var/tmp/spike-increment-c/second-disk.ext4
trial second-disk-elsewhere  --disk "path=/var/tmp/spike-increment-c/second-disk.ext4"
echo
echo "=== full log: api-socket-only"
cat /run/spike-c-matrix2/api-socket-only/ch.log 2>/dev/null | head -3
echo "=== full log: vsock-only+dir-ro-rule"
cat /run/spike-c-matrix2/vsock-only+dir-ro-rule/ch.log 2>/dev/null | head -3
echo "=== full log: vsock-only+path-rule"
cat /run/spike-c-matrix2/vsock-only+path-rule/ch.log 2>/dev/null | head -3
