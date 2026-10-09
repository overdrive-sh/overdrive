#!/bin/sh
# Guest PID 1 for the V-24 spike. Stock kernel, stock vsock modules from the initramfs.
/bin/busybox mount -t proc proc /proc
/bin/busybox mount -t sysfs sysfs /sys
/bin/busybox mount -t devtmpfs devtmpfs /dev
echo "GUEST_BOOT uname_r=$(uname -r)"
for m in vsock vmw_vsock_virtio_transport_common vmw_vsock_virtio_transport vsock_diag; do
  insmod /$m.ko || echo "GUEST_INSMOD_FAIL $m"
done
/v24g
echo "GUEST_AGENT_EXITED rc=$?"
poweroff -f
