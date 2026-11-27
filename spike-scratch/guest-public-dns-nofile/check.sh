#!/usr/bin/env bash
# ADR-0363 "no cap" assumption: do `ip` and `nft` iterate descriptors up to the soft limit?
set -u
uname -r
hard=$(ulimit -Hn); echo "hard nofile=$hard"
ulimit -n "$hard"; echo "soft nofile=$(ulimit -n)"
command -v strace || { echo "strace missing"; exit 2; }
for cmd in "ip link show" "nft list tables"; do
  echo "=== $cmd ==="
  strace -f -c -e trace=close,close_range -o /tmp/nofile-strace.txt $cmd >/dev/null 2>&1
  echo "exit=$?"
  cat /tmp/nofile-strace.txt
done
