#!/usr/bin/env bash
# re-run of case R4 only (after the probe fix); same topology as run.sh
set -u
here="$(cd "$(dirname "$0")" && pwd)"
ip netns add v17-host; ip -n v17-host link set lo up
ip link add v17-h1 type veth peer name v17-p1; ip link set v17-h1 netns v17-host; ip link set v17-p1 netns v17-host
ip -n v17-host link set v17-h1 up
echo "uname -r: $(uname -r)"
ip netns exec v17-host "$here/out/v17" case R4 "$here/out/v17-bpf.o"
ip netns del v17-host
