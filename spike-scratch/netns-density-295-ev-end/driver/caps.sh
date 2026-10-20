#!/bin/bash
# capability probe (read-only): nested KVM, toolchain, artifacts. Run under the lease by launch.py.
echo "uname -r: $(uname -r)"; echo "virt: $(systemd-detect-virt)"
for m in kvm_intel kvm_amd; do [ -e /sys/module/$m/parameters/nested ] && echo "$m nested=$(cat /sys/module/$m/parameters/nested)"; done
grep -m1 "model name" /proc/cpuinfo; nproc; grep -E "MemTotal|MemAvailable" /proc/meminfo
echo "gcc: $(gcc --version | head -1)"; ls -la /usr/lib/x86_64-linux-gnu/libc.a 2>&1
ls -d /lib/modules/7.0.0-29-generic/build /boot/vmlinuz-7.0.0-29-generic 2>&1
echo "kernel env: $OVERDRIVE_METAL_KERNEL"; ls -la "$OVERDRIVE_METAL_KERNEL" 2>&1
ls -la /home/ubuntu/overdrive/spike-scratch/netns-density-295-v24/out/ch-target/release/cloud-hypervisor 2>&1
ls -la /var/tmp/evmod2-work/out/cloud-hypervisor 2>&1
which qemu-system-x86_64 busybox cpio zstd python3 ip ss
cat /proc/loadavg; uptime
lsmod | grep -E "evmod|shmproxy|tls|kvm|vhost" 
cat /run/lock/overdrive-metal-shared.owner 2>/dev/null || echo "no owner file (own lease held)"
