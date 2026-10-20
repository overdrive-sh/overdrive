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
/evend host clean-g-first:5 clean-r-first:5 clean-simul:5 clean-then-trailing-0x3:5 clean-fullend-0x2-0x3-hostlag:5 g-rst-both-dirs:5 g-rst-hg-only:5 g-close-unread:5 r-rst-both-dirs:5 r-rst-gh-only:5 r-close-unread:5 host-abort:5 guest-abort:5 guest-abort-direct0x3-hostdir-closed:5 guest-abort-0x2then0x3-hostdir-open:5 host-abort-direct0x3-guestdir-closed:5 host-abort-0x2then0x3-guestdir-open:5 dial-watch-halfclose:5 dial-watch-halfclose-recordcheck-at-end:5 dial-watch-reset:5 dial-watch-reset-poll:5 dial-watch-halfclose-then-reset:5 dial-watch-hostdrop:5 storm-g-rst:2 storm-r-rst:2 ctx-release-host:2 ctx-release-guest:2
echo "L1_EVEND_EXIT=$?"
echo "--- L1 dmesg tail"; dmesg | tail -40
echo "--- guest serial tail"; tail -60 /tmp/guest-serial.log
echo "--- ch.log tail"; tail -5 /tmp/ch.log
cat /proc/modules | grep -E "ovd_fwd"
poweroff -f
