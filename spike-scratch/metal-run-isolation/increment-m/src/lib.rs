#![allow(dead_code)]
#[cfg(test)]
mod integration {
    fn production_vmm_identity() -> overdrive_core::vm::config::VmmIdentity {
        use std::os::unix::fs::MetadataExt as _;

        use overdrive_core::vm::config::{Gid, OVERDRIVE_VMM_UID, VmmIdentity};

        let group_of = |device: &str| {
            Gid::new(
                std::fs::metadata(device)
                    .unwrap_or_else(|error| panic!("stat {device}: {error}"))
                    .gid(),
            )
        };
        let kvm = group_of("/dev/kvm");
        let vhost = group_of("/dev/vhost-vsock");
        // The production composition uses the reserved numeric pair 4200:4200
        // and discovers the device groups into a non-empty supplementary set.
        // Keep KVM first as the existing owner path does; add vhost-vsock only
        // when it is a distinct group. `--groups=` must never render empty.
        let mut supplementary = vec![kvm];
        if vhost != kvm {
            supplementary.push(vhost);
        }
        VmmIdentity { uid: OVERDRIVE_VMM_UID, gid: Gid::new(OVERDRIVE_VMM_UID), supplementary }
    }
mod vsock_vmm_device;
}
