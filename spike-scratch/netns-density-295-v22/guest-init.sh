#!/bin/sh
/bin/busybox mount -t proc proc /proc
/bin/busybox mount -t sysfs sysfs /sys
/bin/busybox mount -t devtmpfs devtmpfs /dev
echo "GUEST_BOOT uname_r=$(uname -r) uptime=$(cut -d' ' -f1 /proc/uptime)"
for m in vsock vmw_vsock_virtio_transport_common vmw_vsock_virtio_transport; do
  insmod /$m.ko || echo "GUEST_INSMOD_FAIL $m"
done
/probe
echo "GUEST_AGENT_EXITED rc=$?"
poweroff -f
