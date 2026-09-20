//! Semantic boundary for the shared guest-network TCX adapter (GH #295).

// SCAFFOLD: true — netns-density-295 D-295-DISTILL-6.

use std::path::Path;

/// Adapter-neutral TCX attachment point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TcxAttachPoint {
    /// TCX ingress.
    Ingress,
    /// TCX egress.
    Egress,
    /// Kernel custom parent handle.
    Custom(u32),
}

/// Semantic snapshot of one queried TCX attachment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestTcxAttachment {
    /// Kernel TCX attachment revision.
    pub revision: u64,
    /// Attached program identifiers in ascending order.
    pub program_ids: Vec<u32>,
}

/// Closed semantic counter vocabulary for the guest TCX classifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestTcxCounter {
    GatewayHostPass,
    Intercept,
    EndpointMapMiss,
    SourceMacSpoof,
    SourceIpArpSpoof,
    DirectBypassDrop,
    ArpPass,
    MalformedDrop,
}

#[allow(dead_code, reason = "exact private ABI helper activated by D6 read_counter")]
const fn counter_index(counter: GuestTcxCounter) -> u32 {
    match counter {
        GuestTcxCounter::GatewayHostPass => 0,
        GuestTcxCounter::Intercept => 1,
        GuestTcxCounter::EndpointMapMiss => 2,
        GuestTcxCounter::SourceMacSpoof => 3,
        GuestTcxCounter::SourceIpArpSpoof => 4,
        GuestTcxCounter::DirectBypassDrop => 5,
        GuestTcxCounter::ArpPass => 6,
        GuestTcxCounter::MalformedDrop => 7,
    }
}

#[allow(dead_code, reason = "exact query normalization helper activated by D6 query_attachment")]
fn sorted_program_ids(mut program_ids: Vec<u32>) -> Vec<u32> {
    program_ids.sort_unstable();
    program_ids
}

impl From<aya::programs::TcAttachType> for TcxAttachPoint {
    fn from(value: aya::programs::TcAttachType) -> Self {
        match value {
            aya::programs::TcAttachType::Ingress => Self::Ingress,
            aya::programs::TcAttachType::Egress => Self::Egress,
            aya::programs::TcAttachType::Custom(parent) => Self::Custom(parent),
        }
    }
}

/// Canonical TCX adapter failure retaining the exact aya source.
#[derive(Debug, thiserror::Error)]
pub enum GuestTcxError {
    /// BPF map operation failed.
    #[error("guest TCX map operation failed")]
    Map {
        /// Exact aya source.
        #[source]
        source: aya::maps::MapError,
    },
    /// BPF program/link operation failed.
    #[error("guest TCX program operation failed")]
    Program {
        /// Exact aya source.
        #[source]
        source: aya::programs::ProgramError,
    },
    /// bpffs pin operation failed.
    #[error("guest TCX pin operation failed")]
    Pin {
        /// Exact aya source.
        #[source]
        source: aya::pin::PinError,
    },
    /// BPF link operation failed.
    #[error("guest TCX link operation failed")]
    Link {
        /// Exact aya source.
        #[source]
        source: aya::programs::links::LinkError,
    },
    /// Filesystem I/O operation failed.
    #[error("guest TCX I/O operation failed")]
    Io {
        /// Exact standard-library source.
        #[source]
        source: std::io::Error,
    },
}

/// Query one real interface/attach-point pair.
#[doc(hidden)]
#[expect(clippy::panic, reason = "RED scaffold; DELIVER binds aya TCX query")]
pub fn query_attachment(
    _interface: &str,
    _attach_point: TcxAttachPoint,
) -> std::result::Result<GuestTcxAttachment, GuestTcxError> {
    panic!("Not yet implemented -- RED scaffold (GH #295 query TCX attachment)")
}

/// Detach the exact owned pinned TCX link.
#[doc(hidden)]
#[expect(clippy::panic, reason = "RED scaffold; DELIVER binds pinned-link detach")]
pub fn detach_pinned_link(_link_pin: impl AsRef<Path>) -> std::result::Result<(), GuestTcxError> {
    panic!("Not yet implemented -- RED scaffold (GH #295 detach pinned TCX link)")
}

/// Report whether one ifindex is present in the pinned endpoint map.
#[doc(hidden)]
#[expect(clippy::panic, reason = "RED scaffold; DELIVER binds endpoint lookup")]
pub fn endpoint_present(
    _endpoint_map_pin: impl AsRef<Path>,
    _ifindex: u32,
) -> std::result::Result<bool, GuestTcxError> {
    panic!("Not yet implemented -- RED scaffold (GH #295 endpoint presence)")
}

/// Remove one ifindex from the pinned endpoint map.
#[doc(hidden)]
#[expect(clippy::panic, reason = "RED scaffold; DELIVER binds endpoint removal")]
pub fn remove_endpoint(
    _endpoint_map_pin: impl AsRef<Path>,
    _ifindex: u32,
) -> std::result::Result<(), GuestTcxError> {
    panic!("Not yet implemented -- RED scaffold (GH #295 endpoint removal)")
}

/// Read one semantic slot from the pinned counter array.
#[doc(hidden)]
#[expect(clippy::panic, reason = "RED scaffold; DELIVER binds counter read")]
pub fn read_counter(
    _counter_map_pin: impl AsRef<Path>,
    _counter: GuestTcxCounter,
) -> std::result::Result<u64, GuestTcxError> {
    panic!("Not yet implemented -- RED scaffold (GH #295 guest TCX counter read)")
}

#[cfg(test)]
#[allow(clippy::doc_markdown)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn semantic_attach_point_projection_is_exhaustive() {
        let cases = [
            (aya::programs::TcAttachType::Ingress, TcxAttachPoint::Ingress),
            (aya::programs::TcAttachType::Egress, TcxAttachPoint::Egress),
            (aya::programs::TcAttachType::Custom(0), TcxAttachPoint::Custom(0)),
            (aya::programs::TcAttachType::Custom(u32::MAX), TcxAttachPoint::Custom(u32::MAX)),
        ];

        for (raw, semantic) in cases {
            assert_eq!(TcxAttachPoint::from(raw), semantic);
        }
    }

    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn canonical_error_family_retains_each_exact_aya_source() {
        let errors = [
            GuestTcxError::Map { source: aya::maps::MapError::KeyNotFound },
            GuestTcxError::Program { source: aya::programs::ProgramError::NotLoaded },
            GuestTcxError::Pin {
                source: aya::pin::PinError::NoFd { name: "guest_tcx".to_owned() },
            },
            GuestTcxError::Link { source: aya::programs::links::LinkError::InvalidLink },
            GuestTcxError::Io { source: std::io::Error::other("guest TCX I/O") },
        ];

        assert!(matches!(errors[0], GuestTcxError::Map { .. }));
        assert!(matches!(errors[1], GuestTcxError::Program { .. }));
        assert!(matches!(errors[2], GuestTcxError::Pin { .. }));
        assert!(matches!(errors[3], GuestTcxError::Link { .. }));
        assert!(matches!(errors[4], GuestTcxError::Io { .. }));
    }

    proptest! {
        /// CONTRACT_SHAPE: pure-function.
        #[test]
        fn queried_program_identity_is_sorted_without_losing_attachment_multiplicity(
            program_ids in proptest::collection::vec(any::<u32>(), 0..128)
        ) {
            let mut expected = program_ids.clone();
            expected.sort_unstable();
            let observed = sorted_program_ids(program_ids.clone());
            prop_assert_eq!(&observed, &expected);
            prop_assert_eq!(observed.len(), program_ids.len());
        }
    }

    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn semantic_counter_vocabulary_maps_to_the_exact_private_array_slots() {
        let cases = [
            (GuestTcxCounter::GatewayHostPass, 0),
            (GuestTcxCounter::Intercept, 1),
            (GuestTcxCounter::EndpointMapMiss, 2),
            (GuestTcxCounter::SourceMacSpoof, 3),
            (GuestTcxCounter::SourceIpArpSpoof, 4),
            (GuestTcxCounter::DirectBypassDrop, 5),
            (GuestTcxCounter::ArpPass, 6),
            (GuestTcxCounter::MalformedDrop, 7),
        ];
        for (counter, expected) in cases {
            assert_eq!(counter_index(counter), expected);
        }
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    #[ignore = "pending DELIVER step for GH #295 typed aya query/open error projection"]
    fn absent_real_objects_preserve_the_operation_specific_source_family() {
        let missing = format!("/sys/fs/bpf/overdrive/absent-{}", std::process::id());
        assert!(matches!(
            query_attachment("overdrive-absent-interface", TcxAttachPoint::Ingress),
            Err(GuestTcxError::Program { .. })
        ));
        assert!(matches!(detach_pinned_link(&missing), Err(GuestTcxError::Link { .. })));
        assert!(matches!(endpoint_present(&missing, 1), Err(GuestTcxError::Map { .. })));
        assert!(matches!(remove_endpoint(&missing, 1), Err(GuestTcxError::Map { .. })));
        assert!(matches!(
            read_counter(&missing, GuestTcxCounter::MalformedDrop),
            Err(GuestTcxError::Map { .. })
        ));
    }
}
