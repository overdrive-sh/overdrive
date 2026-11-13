#!/usr/bin/env bash
# Runs ON the box. Usage: kill-nextest-watch.sh <run-id>
# SIGKILLs THIS run's cargo-nextest (matched by OVERDRIVE_METAL_RUN_ID in its
# environment), then samples every 20 ms for 1 s and every 250 ms to 6 s which
# of this run's processes (cmdline naming /run/ovd-gk/<run-id>) are alive, and
# keeps copying the owner log so a line written before teardown survives it.
R=$1
sudo -n bash -s "$R" <<'ROOT'
R=$1
now() { date +%s.%N | cut -c1-14; }
alive() { for p in /proc/[0-9]*; do
    tr "\0" " " < $p/cmdline 2>/dev/null | grep -q "/run/ovd-gk/$R" || continue
    c=$(cat $p/comm 2>/dev/null); st=$(awk '{print $3}' $p/stat 2>/dev/null)
    echo -n "${p#/proc/}:$c:$st "; done; echo; }
target=""
for p in /proc/[0-9]*; do
  grep -qa "OVERDRIVE_METAL_RUN_ID=$R" $p/environ 2>/dev/null || continue
  [ "$(cat $p/comm 2>/dev/null)" = cargo-nextest ] && target=${p#/proc/}
done
echo "$(now) before: $(alive)"
echo "$(now) SIGKILL cargo-nextest pid $target"
kill -9 $target
LOGCOPY=""
for i in $(seq 1 50); do
  sleep 0.02
  l=$(cat /run/ovd-gk/$R/shared/v4r.owner.log 2>/dev/null); [ -n "$l" ] && LOGCOPY="$l"
  echo "$(now) +$((i*20))ms: $(alive)"
done
for i in $(seq 1 20); do
  sleep 0.25
  l=$(cat /run/ovd-gk/$R/shared/v4r.owner.log 2>/dev/null); [ -n "$l" ] && LOGCOPY="$l"
  echo "$(now) +$((1000+i*250))ms: $(alive)"
done
echo "--- last readable copy of the owner log (lines with ANCHOR/QUIT):"
echo "$LOGCOPY" | grep -E "ANCHOR|QUIT|serving" 
ROOT
