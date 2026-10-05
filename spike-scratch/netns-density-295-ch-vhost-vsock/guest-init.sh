#!/bin/sh
# Guest PID 1 for the CH vhost-vsock spike. Unmodified stock kernel; stock
# vsock modules are loaded from the initramfs; the probe agent serves
# litmus listeners (5000 stream, 5001 seqpacket) and a control port (4000).
/bin/busybox mount -t proc proc /proc
/bin/busybox mount -t sysfs sysfs /sys
/bin/busybox mount -t devtmpfs devtmpfs /dev
echo "GUEST_BOOT uname_r=$(uname -r) uptime=$(cut -d' ' -f1 /proc/uptime)"
for m in vsock vmw_vsock_virtio_transport_common vmw_vsock_virtio_transport; do
  insmod /$m.ko || echo "GUEST_INSMOD_FAIL $m"
done
for d in /sys/bus/virtio/devices/*; do
  echo "GUEST_VIRTIO $(basename $d) device=$(cat $d/device) driver=$(basename $(readlink $d/driver 2>/dev/null) 2>/dev/null) features=$(cat $d/features 2>/dev/null)"
done
ls -l /dev/vsock
/probe local-cid
/probe agent
echo "GUEST_AGENT_EXITED rc=$?"
poweroff -f
