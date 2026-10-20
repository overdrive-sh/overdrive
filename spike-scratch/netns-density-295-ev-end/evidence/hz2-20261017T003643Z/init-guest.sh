#!/bin/sh
export PATH=/usr/bin:/usr/sbin:/bin:/sbin
mount -t proc proc /proc; mount -t sysfs sysfs /sys; mount -t devtmpfs devtmpfs /dev; mkdir -p /tmp; mount -t tmpfs tmpfs /tmp
echo "GUEST_BOOT uname_r=$(uname -r)"
for m in vsock.ko vmw_vsock_virtio_transport_common.ko vmw_vsock_virtio_transport.ko; do insmod /mods/$m || echo "GUEST_INSMOD_FAIL $m"; done
insmod /mods/ovd_fwd.ko hazard=2 || echo "GUEST_INSMOD_FAIL ovd_fwd"
ip link set lo up
ulimit -n 65536
echo "GUEST_READY"
/evend agent > /dev/ttyS0 2>&1 &
while true; do sleep 3600; done
