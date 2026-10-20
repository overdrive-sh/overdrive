#!/bin/bash
# recon on the metal host (read-only, short): kernel, tools, modules, caches. No module is loaded.
uname -r; systemd-detect-virt; echo "kver dirs:"; ls /lib/modules
K=7.0.0-29-generic
echo "--- vsock modules"; find /lib/modules/$K -name 'vsock_loopback*' -o -name 'vmw_vsock_virtio_transport*' -o -name 'vsock.ko*' -o -name 'vhost_vsock*' | sort
echo "--- nft"; which nft; nft --version 2>&1 | head -1; ldd $(which nft) 2>&1 | head -20
echo "--- nf_tables modules"; find /lib/modules/$K -name 'nf_tables*' -o -name 'nft_reject*' -o -name 'nft_chain_*' -o -name 'nf_reject_ipv4*' -o -name 'nft_ct*' | sort
echo "--- modprobe deps for nf_tables / nft_reject_ipv4 / vsock_loopback"
for m in nf_tables nft_reject_ipv4 nft_reject vsock_loopback vmw_vsock_virtio_transport; do echo "# $m"; modprobe -S $K --show-depends $m 2>&1 | sed 's#/lib/modules/[^ ]*/##' | head -12; done
echo "--- tools"; which qemu-system-x86_64 busybox cpio zstd gcc make bpf-linker cargo 2>&1; ls /usr/bin/busybox
echo "--- prior work"; ls -d /var/tmp/*-work 2>&1; ls /var/tmp/evmod2-work/out 2>&1 | head -40
echo "--- cargo homes"; du -sh /var/tmp/evmod2-work/out/cargo-home-bench /var/tmp/evmod2-work/out/cargo-home-valve 2>&1
echo "--- ch / kernel"; ls -la $OVERDRIVE_METAL_KERNEL /var/tmp/evmod2-work/out/cloud-hypervisor 2>&1
echo "--- resources"; nproc; free -g | head -2; df -h /var/tmp | tail -1; cat /proc/loadavg
echo "--- loaded evmod/tls/shmproxy on physical host:"; lsmod | grep -E '^(evmod|tls|shmproxy)' || echo none
ls /sys/fs/bpf >/dev/null 2>&1; rustup toolchain list 2>&1 | head; ls ~/.rustup 2>&1 | head -2
