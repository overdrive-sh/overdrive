#!/usr/bin/env bash
# Runs ON the box. Usage: kill-and-watch.sh <run-id> <SIGNAL> <nextest|broker>
# One ssh session: sample, signal this run's process, then sample every 0.25 s.
R=$1 SIG=$2 WHO=$3
want=cargo-nextest; [ "$WHO" = broker ] && want=modguest-broker
sudo -n bash -s "$R" "$SIG" "$want" <<'ROOT'
R=$1 SIG=$2 want=$3
list() { for p in /proc/[0-9]*; do c=$(cat $p/comm 2>/dev/null) || continue
  case "$c" in modguest-broker|cloud-hyperviso*) echo -n "${p#/proc/}:$c ";; esac; done; echo; }
target=""
for p in /proc/[0-9]*; do
  grep -qa "OVERDRIVE_METAL_RUN_ID=$R" $p/environ 2>/dev/null || continue
  [ "$(cat $p/comm 2>/dev/null)" = "$want" ] && target=${p#/proc/}
done
echo "$(date +%T.%N | cut -c1-12) before: $(list)"
echo "$(date +%T.%N | cut -c1-12) sending SIG$SIG to $want pid $target"
kill -$SIG $target
for i in $(seq 1 24); do sleep 0.25; echo "$(date +%T.%N | cut -c1-12) +$((i*250))ms: $(list)"; done
ROOT
