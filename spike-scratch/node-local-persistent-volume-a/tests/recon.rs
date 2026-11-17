use nlpv_probe::sh;
use overdrive_testing::overdrive_test;

#[overdrive_test(host_pure)]
#[test]
fn recon_host() {
    sh("uname", "uname -a; cat /proc/cmdline");
    sh("cpu", "lscpu | head -30; free -m; nproc");
    sh("virt", "systemd-detect-virt; ls -l /dev/kvm");
    sh("tools", "for t in fio dmsetup mkfs.xfs mkfs.ext4 losetup xfs_io blkdiscard strace cloud-hypervisor busybox setpriv prlimit lvm fallocate; do printf '%s=' $t; command -v $t || echo MISSING; done");
    sh("ch", "cloud-hypervisor --version; ls -l $(command -v cloud-hypervisor)");
    sh("mounts", "findmnt -o TARGET,SOURCE,FSTYPE,OPTIONS | head -40; df -h /srv/vm /tmp /home; lsblk -o NAME,SIZE,TYPE,FSTYPE,MOUNTPOINT,ROTA,MODEL");
    sh("kcfg", "ls /boot | head; for o in BLK_DEV_LOOP BLK_DEV_DM DM_LOG_WRITES DM_FLAKEY DM_ERROR DM_THIN_PROVISIONING XFS_FS EXT4_FS BTRFS_FS VIRTIO_BLK BLK_DEV_WRITE_MOUNTED IO_URING MEMCG BLK_CGROUP DM_DUST DM_ZERO DM_LINEAR FAULT_INJECTION FAIL_MAKE_REQUEST; do grep -h \"CONFIG_$o=\\|CONFIG_$o is\" /boot/config-$(uname -r) || echo \"CONFIG_$o not set/absent\"; done");
    sh("mods", "R=$(uname -r); for m in loop dm-mod dm-log-writes dm-flakey dm-linear dm-bufio dm-thin-pool dm-persistent-data dm-dust xfs ext4 virtio_blk raid0 libcrc32c crc32c_generic; do echo \"== $m\"; find /lib/modules/$R -name \"$m.ko*\" ; grep -h \"/$m.ko\" /lib/modules/$R/modules.builtin 2>/dev/null; modinfo -F depends $m 2>/dev/null; done");
    sh("vmdir", "ls -la /srv/vm/overdrive-testing /srv/vm | head -40; file /srv/vm/overdrive-testing/kernel | head -3");
    sh("fs", "xfs_info /srv/vm 2>&1 | head -20; mount | grep -E ' /srv| xfs'");
    sh("chsha", "sha256sum /usr/local/bin/cloud-hypervisor; file /usr/local/bin/cloud-hypervisor | cut -c1-200; sysctl kernel.io_uring_disabled; cat /sys/block/nvme1n1/queue/write_cache /sys/block/nvme1n1/queue/rotational");
    sh("cgroup", "cat /proc/self/cgroup; stat -fc %T /sys/fs/cgroup; ls /sys/fs/cgroup | head -30");
}
