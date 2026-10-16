#!/bin/bash
# Build the vmw_vsock_virtio_transport_common.ko variants out of tree against the host's own 7.0.0-29 build tree.
# variants: rebuilt (unmodified v7.0 source) and p1 (credit released by the skb destructor).
set -euo pipefail
W=/tmp/bpf-credit-work; K=$W/src/kmod; O=$W/out/kmod
mkdir -p $O
for v in rebuilt p1; do
  rm -rf $O/$v; mkdir -p $O/$v; cp $K/virtio_transport_common.c $K/Kbuild $O/$v/
  if [ $v = p1 ]; then P=y; else P=; fi
  make -C /lib/modules/$(uname -r)/build M=$O/$v P1=$P modules > $O/$v/build.log 2>&1 || { tail -40 $O/$v/build.log; exit 1; }
  modinfo $O/$v/vmw_vsock_virtio_transport_common.ko | grep -E 'vermagic|name:'
  cp $O/$v/vmw_vsock_virtio_transport_common.ko $O/vmw_vsock_virtio_transport_common.$v.ko
  sha256sum $O/vmw_vsock_virtio_transport_common.$v.ko
done
