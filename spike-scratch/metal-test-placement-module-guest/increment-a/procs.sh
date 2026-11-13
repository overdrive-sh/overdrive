#!/usr/bin/env bash
# Runs ON the box. Lists broker / cloud-hypervisor processes by scanning /proc
# (comm match), as `ps` run from the ssh session does not list them reliably.
# Usage: procs.sh [substring-filter-on-cmdline]
sudo -n bash -c '
for p in /proc/[0-9]*; do
  c=$(cat $p/comm 2>/dev/null) || continue
  case "$c" in modguest-broker|cloud-hyperviso*)
    line="$(basename $p) $c $(tr "\0" " " < $p/cmdline | sed -E "s#/home/ubuntu/overdrive-worktrees/[0-9a-f]*/spike-scratch/metal-test-placement-module-guest/increment-a/target/debug/##;s#--(kernel|cmdline|disk|landlock-rules|api-socket|serial|vsock|cpus|memory|seccomp|console)[ =][^ ]*##g" | cut -c1-400)"
    echo "$line";;
  esac
done' | { if [ -n "${1:-}" ]; then grep -- "$1"; else cat; fi; }
