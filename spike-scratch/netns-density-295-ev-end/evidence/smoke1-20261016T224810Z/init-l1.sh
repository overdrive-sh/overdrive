#!/bin/sh
export PATH=/usr/bin:/usr/sbin:/bin:/sbin
mount -t proc proc /proc; mount -t sysfs sysfs /sys; mount -t devtmpfs devtmpfs /dev; mkdir -p /tmp; mount -t tmpfs tmpfs /tmp
echo "L1_BOOT uname_r=$(uname -r)"
for m in vsock.ko vhost_iotlb.ko vhost.ko vmw_vsock_virtio_transport_common.ko vhost_vsock.ko irqbypass.ko kvm.ko ccp.ko kvm-amd.ko; do insmod /mods/$m || echo "L1_INSMOD_FAIL $m"; done
cat /sys/module/kvm_amd/parameters/nested; ls -la /dev/kvm /dev/vhost-vsock
insmod /mods/ovd_fwd.ko hazard=0 || echo "L1_INSMOD_FAIL ovd_fwd"
ls -la /dev/ovd_fwd; grep -E "ovd_fwd" /proc/modules
ip link set lo up
ulimit -n 65536
/evend host clean-g-first:5 g-rst-both-dirs:5 r-rst-both-dirs:5
echo "L1_EVEND_EXIT=$?"
echo "--- L1 dmesg tail"; dmesg | tail -40
echo "--- guest serial tail"; tail -60 /tmp/guest-serial.log
echo "--- ch.log tail"; tail -5 /tmp/ch.log
cat /proc/modules | grep -E "ovd_fwd"
poweroff -f
