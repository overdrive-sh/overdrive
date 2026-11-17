use nlpv_probe::sh;
use overdrive_testing::overdrive_test;

#[overdrive_test(guest = "cgroup-default", profile = "nested-launch")]
#[test]
fn recon_outer_guest() {
    sh("uname", "uname -a; cat /proc/cmdline; id");
    sh("res", "cat /proc/cpuinfo | grep -m2 'model name\\|processor'; free -m; nproc");
    sh("mounts", "mount; df -h; echo; cat /proc/filesystems | tr '\\n' ' '");
    sh("devs", "cat /proc/devices; ls -l /dev | head -60; ls /dev/mapper 2>&1");
    sh("tools", "echo $PATH; for t in fio dmsetup mkfs.xfs mkfs.ext4 losetup xfs_io blkdiscard strace cloud-hypervisor busybox setpriv prlimit zstd cp dd sync sleep timeout ps kill mknod mount umount ls cat; do printf '%s=' $t; command -v $t || echo MISSING; done");
    sh("fixture", "ls -la /srv/vm /srv/vm/overdrive-testing /boot 2>&1 | head -40; cat /proc/mounts | grep -E 'vm|xfs|loop'; ls /var/lib/overdrive-testing /opt/overdrive-testing 2>&1");
    sh("cg", "cat /proc/self/cgroup; cat /sys/fs/cgroup/cgroup.controllers; cat /sys/fs/cgroup/overdrive.slice/workloads.slice/cgroup.controllers /sys/fs/cgroup/overdrive.slice/workloads.slice/cgroup.subtree_control");
    sh("mods", "ls /lib/modules /lib/overdrive-testing/modules 2>&1 | head -50; ls /sys/module | tr '\\n' ' '; cat /proc/modules | head -50");
    sh("kvm", "ls -l /dev/kvm /dev/vhost-vsock; lsmod 2>&1| head");
    sh("tmp", "df -h /tmp /run /dev/shm /var /root 2>&1; ls -ld /tmp; ulimit -a");
}
