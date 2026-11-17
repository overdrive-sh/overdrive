use nlpv_v1_probe::*;
use overdrive_testing::overdrive_test;

#[overdrive_test(host_pure)]
#[test]
fn v1_recon_host() {
    sh("uname", "uname -a; id; echo $PATH");
    sh("lvm", "command -v lvm lvm.static; ls -l /usr/sbin/lvm /usr/sbin/lvm.static /sbin/lvm 2>&1; lvm version; sha256sum $(command -v lvm); file -L $(command -v lvm); dpkg -S $(command -v lvm); dpkg-query -W lvm2 libdevmapper1.02.1 dmsetup");
    sh("ldd", "ldd /usr/sbin/lvm");
    sh("kcfg", "for o in BLK_DEV_DM EFI_PARTITION DEVTMPFS DEVTMPFS_MOUNT VIRTIO_BLK EXT4_FS DM_UEVENT DM_CRYPT DM_SNAPSHOT DM_MIRROR DM_THIN_PROVISIONING MODULES; do grep -h \"CONFIG_$o=\\|CONFIG_$o is\" /boot/config-$(uname -r) || echo \"CONFIG_$o absent\"; done");
    sh("manifest", &format!("cat <<'EOT'\n{}EOT", LVM_MANIFEST));
    sh("tools", "for t in lvm dmsetup sha256sum file ldd rustc cargo; do printf '%s=' $t; command -v $t || echo MISSING; done; rustup target list --installed 2>&1");
}
