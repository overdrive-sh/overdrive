#!/bin/bash
# Build (on the metal host, inside the lease, a few seconds): the module against the guest's 7.0.0-29-generic tree, the static probe,
# and stage the Cloud Hypervisor fork binary used by the earlier increments. Everything lands in /var/tmp/ev-pair-work/out.
set -euo pipefail
W=/var/tmp/ev-pair-work
KVER=7.0.0-29-generic
mkdir -p $W/out
rm -rf $W/out/mod && mkdir -p $W/out/mod && cp $W/src/module/* $W/out/mod/
make -C /lib/modules/$KVER/build M=$W/out/mod W=1 modules 2>&1 | grep -v "^make" | tail -15
if gcc -O2 -static -o $W/out/pairprobe $W/src/probe/pairprobe.c -I $W/src/probe -lpthread 2>$W/out/gcc-static.err; then echo "pairprobe: static"; else echo "pairprobe: static build failed, dynamic"; cat $W/out/gcc-static.err | head -5; gcc -O2 -o $W/out/pairprobe $W/src/probe/pairprobe.c -I $W/src/probe -lpthread; fi
[ -f $W/out/cloud-hypervisor ] || cp /var/tmp/evmod2-work/out/cloud-hypervisor $W/out/cloud-hypervisor
sha256sum $W/out/mod/ovdpair.ko $W/out/pairprobe $W/out/cloud-hypervisor $W/src/module/ovdpair.c $W/src/probe/pairprobe.c
modinfo $W/out/mod/ovdpair.ko | grep -E "vermagic|srcversion"
