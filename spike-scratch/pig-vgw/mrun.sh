#!/usr/bin/env bash
# usage: mrun.sh <logname> <remote bash script path/args...>
set -u
cd "$(dirname "$0")/../.."
log="spike-scratch/pig-vgw/out/$1.log"; shift
export OVERDRIVE_METAL_KERNEL=/srv/vm/overdrive-testing/kernel OVERDRIVE_METAL_ROOTFS=/srv/vm/overdrive-testing/rootfs.ext4
timeout 1750 cargo xtask metal run ${SYNC:-} -- bash -c "umask 022; ulimit -n 1048576; $*" > "$log" 2>&1
echo "rc=$? log=$log"
