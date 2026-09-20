//! Semantic boundary for the shared guest-network TCX adapter (GH #295).

// SCAFFOLD: true — netns-density-295 D-295-DISTILL-6.

use std::collections::BTreeSet;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

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

/// Closed semantic identity for one required embedded TCX object.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestTcxObject {
    EndpointMap,
    CounterMap,
    Classifier,
}

/// Endpoint value crossing the control-plane/dataplane boundary.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuestTcxEndpoint {
    pub source_ipv4: Ipv4Addr,
    pub source_mac: [u8; 6],
    pub bridge_mac: [u8; 6],
}

/// Opaque identity for an unsupported kernel map kind.
#[doc(hidden)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct GuestTcxUnsupportedMapKind(u32);

impl std::fmt::Debug for GuestTcxUnsupportedMapKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Unsupported")
    }
}

/// Opaque identity for an unsupported kernel map property.
#[doc(hidden)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct GuestTcxUnsupportedMapProperty(u32);

impl std::fmt::Debug for GuestTcxUnsupportedMapProperty {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Unsupported")
    }
}

#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestTcxMapKind {
    Hash,
    Array,
    Unsupported(GuestTcxUnsupportedMapKind),
}

#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestTcxMapKeyShape {
    U32,
    Unsupported(GuestTcxUnsupportedMapProperty),
}

#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestTcxMapValueShape {
    EndpointAbi,
    CounterU64,
    Unsupported(GuestTcxUnsupportedMapProperty),
}

#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestTcxMapCapacity {
    EndpointMaximum,
    CounterSlots,
    Unsupported(GuestTcxUnsupportedMapProperty),
}

#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuestTcxMapSchema {
    pub kind: GuestTcxMapKind,
    pub key: GuestTcxMapKeyShape,
    pub value: GuestTcxMapValueShape,
    pub capacity: GuestTcxMapCapacity,
}

#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestTcxInventoryFamily {
    EndpointMap,
    CounterMap,
    EndpointEntry,
    TcxProgram,
    TcxLink,
    EndpointMapPin,
    CounterMapPin,
    TcxLinkPin,
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
    /// Embedded BPF object load failed.
    #[error("guest TCX object load failed")]
    Load {
        #[source]
        source: aya::EbpfError,
    },
    /// A required embedded object was absent.
    #[error("guest TCX object is missing required {object:?}")]
    ObjectMissing { object: GuestTcxObject },
    /// An observed map has the wrong semantic schema.
    #[error("guest TCX map schema does not match the accepted identity")]
    MapSchemaMismatch { expected: GuestTcxMapSchema, observed: GuestTcxMapSchema },
    /// A post-baseline candidate lacks an ownership receipt.
    #[error("guest TCX inventory is ambiguous for {family:?}")]
    InventoryAmbiguous { family: GuestTcxInventoryFamily },
    /// A planned pin contains an object with another identity.
    #[error("guest TCX ownership identity does not match for {family:?}")]
    OwnershipMismatch { family: GuestTcxInventoryFamily },
    /// One capture domain was unavailable and no later receipt supersedes it.
    #[error("guest TCX inventory capture is unavailable for {family:?}")]
    CaptureUnavailable { family: GuestTcxInventoryFamily },
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

#[derive(Clone)]
#[allow(dead_code, reason = "D12 exact private RED scaffold")]
struct RawGuestTcxMapObservation {
    id: u32,
    kind: aya::maps::MapType,
    key_size: u32,
    value_size: u32,
    max_entries: u32,
    name: Vec<u8>,
}

#[derive(Clone)]
#[allow(dead_code, reason = "D12 exact private RED scaffold")]
struct RawGuestTcxProgramObservation {
    id: u32,
    tag: u64,
    name: Vec<u8>,
    program_type: aya::programs::ProgramType,
    map_ids: Vec<u32>,
}

#[derive(Clone)]
#[allow(dead_code, reason = "D12 exact private RED scaffold")]
struct RawGuestTcxLinkObservation {
    id: u32,
    program_id: u32,
    target_ifindex: u32,
    attach_type: u32,
}

#[derive(Clone)]
#[allow(dead_code, reason = "D12 exact private RED scaffold")]
enum RawGuestTcxPinObservation {
    Absent,
    Map(RawGuestTcxMapObservation),
    Link(RawGuestTcxLinkObservation),
    Other,
}

#[allow(dead_code, reason = "D12 exact private RED scaffold")]
trait GuestTcxInventorySource: Send + Sync {
    fn loaded_maps(&self) -> Result<Vec<RawGuestTcxMapObservation>, GuestTcxError>;
    fn loaded_programs(&self) -> Result<Vec<RawGuestTcxProgramObservation>, GuestTcxError>;
    fn loaded_links(&self) -> Result<Vec<RawGuestTcxLinkObservation>, GuestTcxError>;
    fn map_by_id(&self, id: u32) -> Result<Option<RawGuestTcxMapObservation>, GuestTcxError>;
    fn endpoint_present_by_id(&self, map_id: u32, ifindex: u32) -> Result<bool, GuestTcxError>;
    fn observe_pin(&self, path: &Path) -> Result<RawGuestTcxPinObservation, GuestTcxError>;
}

struct AyaGuestTcxInventorySource;

impl GuestTcxInventorySource for AyaGuestTcxInventorySource {
    #[expect(clippy::panic, reason = "D12 RED scaffold")]
    fn loaded_maps(&self) -> Result<Vec<RawGuestTcxMapObservation>, GuestTcxError> {
        panic!("Not yet implemented -- RED scaffold (GH #295 D12 loaded maps)")
    }
    #[expect(clippy::panic, reason = "D12 RED scaffold")]
    fn loaded_programs(&self) -> Result<Vec<RawGuestTcxProgramObservation>, GuestTcxError> {
        panic!("Not yet implemented -- RED scaffold (GH #295 D12 loaded programs)")
    }
    #[expect(clippy::panic, reason = "D12 RED scaffold")]
    fn loaded_links(&self) -> Result<Vec<RawGuestTcxLinkObservation>, GuestTcxError> {
        panic!("Not yet implemented -- RED scaffold (GH #295 D12 loaded links)")
    }
    #[expect(clippy::panic, reason = "D12 RED scaffold")]
    fn map_by_id(&self, _id: u32) -> Result<Option<RawGuestTcxMapObservation>, GuestTcxError> {
        panic!("Not yet implemented -- RED scaffold (GH #295 D12 map-by-id)")
    }
    #[expect(clippy::panic, reason = "D12 RED scaffold")]
    fn endpoint_present_by_id(&self, _map_id: u32, _ifindex: u32) -> Result<bool, GuestTcxError> {
        panic!("Not yet implemented -- RED scaffold (GH #295 D12 endpoint-by-id)")
    }
    #[expect(clippy::panic, reason = "D12 RED scaffold")]
    fn observe_pin(&self, _path: &Path) -> Result<RawGuestTcxPinObservation, GuestTcxError> {
        panic!("Not yet implemented -- RED scaffold (GH #295 D12 pin observation)")
    }
}

#[expect(clippy::panic, reason = "D12 RED scaffold; DELIVER binds real aya inventory")]
fn capture_with_source(
    _endpoint_map_pin: PathBuf,
    _counter_map_pin: PathBuf,
    _source: Arc<dyn GuestTcxInventorySource>,
) -> GuestTcxInventoryCapture {
    panic!("Not yet implemented -- RED scaffold (GH #295 D12 inventory capture)")
}

#[expect(clippy::panic, reason = "D12 RED scaffold; DELIVER projects every aya map kind")]
#[allow(dead_code, reason = "D12 source-local table activates this projection")]
fn project_map_kind(_raw: aya::maps::MapType) -> GuestTcxMapKind {
    panic!("Not yet implemented -- RED scaffold (GH #295 D12 map-kind projection)")
}

#[expect(clippy::panic, reason = "D12 RED scaffold; DELIVER projects private map metadata")]
#[allow(dead_code, reason = "D12 source-local table activates this projection")]
fn project_map_schema(_raw: &RawGuestTcxMapObservation) -> GuestTcxMapSchema {
    panic!("Not yet implemented -- RED scaffold (GH #295 D12 map-schema projection)")
}

#[doc(hidden)]
pub struct GuestTcxInventoryCapture {
    identity: GuestTcxInventoryIdentity,
    disposition: Result<(), GuestTcxError>,
}

impl GuestTcxInventoryCapture {
    pub fn into_parts(self) -> (GuestTcxInventoryIdentity, Result<(), GuestTcxError>) {
        (self.identity, self.disposition)
    }
}

#[doc(hidden)]
#[derive(Clone)]
#[allow(dead_code, reason = "D12 exact identity fields are populated in DELIVER")]
pub struct GuestTcxInventoryIdentity {
    source: Arc<dyn GuestTcxInventorySource>,
    endpoint_map_pin: PathBuf,
    counter_map_pin: PathBuf,
    link_pin: Arc<parking_lot::Mutex<Option<PathBuf>>>,
    receipts: Arc<parking_lot::Mutex<GuestTcxInventoryReceipts>>,
}

#[derive(Default)]
#[allow(dead_code, reason = "D12 exact private receipt scaffold")]
struct GuestTcxInventoryReceipts {
    endpoint_map_id: Option<u32>,
    counter_map_id: Option<u32>,
    endpoint_ifindices: BTreeSet<u32>,
    program_id: Option<u32>,
    link_id: Option<u32>,
    endpoint_map_pin_id: Option<u32>,
    counter_map_pin_id: Option<u32>,
    link_pin_id: Option<u32>,
}

#[doc(hidden)]
pub struct GuestTcxProgram {
    _private: (),
}

#[doc(hidden)]
pub struct GuestTcxLink {
    _private: (),
}

#[doc(hidden)]
pub struct GuestTcxAdoptedState {
    _private: (),
}

#[allow(
    clippy::needless_pass_by_ref_mut,
    clippy::unused_self,
    reason = "D12 exact stateful RED signatures precede implementation"
)]
impl GuestTcxProgram {
    #[expect(clippy::panic, reason = "D12 RED scaffold")]
    pub fn load(_inventory: &GuestTcxInventoryIdentity) -> Result<Self, GuestTcxError> {
        panic!("Not yet implemented -- RED scaffold (GH #295 D12 TCX load)")
    }
    #[expect(clippy::panic, reason = "D12 RED scaffold")]
    pub fn pin_endpoint_map(&mut self, _pin: &Path) -> Result<GuestTcxMapSchema, GuestTcxError> {
        panic!("Not yet implemented -- RED scaffold (GH #295 D12 endpoint-map pin)")
    }
    #[expect(clippy::panic, reason = "D12 RED scaffold")]
    pub fn pin_counter_map(&mut self, _pin: &Path) -> Result<GuestTcxMapSchema, GuestTcxError> {
        panic!("Not yet implemented -- RED scaffold (GH #295 D12 counter-map pin)")
    }
    #[expect(clippy::panic, reason = "D12 RED scaffold")]
    pub fn insert_endpoint(
        &mut self,
        _ifindex: u32,
        _endpoint: GuestTcxEndpoint,
    ) -> Result<(), GuestTcxError> {
        panic!("Not yet implemented -- RED scaffold (GH #295 D12 endpoint insert)")
    }
    #[expect(clippy::panic, reason = "D12 RED scaffold")]
    pub fn read_endpoint(&self, _ifindex: u32) -> Result<Option<GuestTcxEndpoint>, GuestTcxError> {
        panic!("Not yet implemented -- RED scaffold (GH #295 D12 endpoint read-back)")
    }
    #[expect(clippy::panic, reason = "D12 RED scaffold")]
    pub fn attach_first_ingress(
        &mut self,
        _interface: &str,
    ) -> Result<GuestTcxLink, GuestTcxError> {
        panic!("Not yet implemented -- RED scaffold (GH #295 D12 first-ingress attach)")
    }
}

#[allow(clippy::unused_self, reason = "D12 exact consuming RED signatures")]
impl GuestTcxLink {
    #[expect(clippy::panic, reason = "D12 RED scaffold")]
    pub fn program_id(&self) -> u32 {
        panic!("Not yet implemented -- RED scaffold (GH #295 D12 link program identity)")
    }
    #[expect(clippy::panic, reason = "D12 RED scaffold")]
    pub fn pin(self, _pin: &Path) -> Result<(), GuestTcxError> {
        panic!("Not yet implemented -- RED scaffold (GH #295 D12 link pin)")
    }
    #[expect(clippy::panic, reason = "D12 RED scaffold")]
    pub fn detach(self) -> Result<(), GuestTcxError> {
        panic!("Not yet implemented -- RED scaffold (GH #295 D12 link detach)")
    }
}

#[allow(
    clippy::needless_pass_by_ref_mut,
    clippy::unused_self,
    reason = "D12 exact stateful RED signatures precede implementation"
)]
impl GuestTcxAdoptedState {
    #[expect(clippy::panic, reason = "D12 RED scaffold")]
    pub fn for_inventory(_inventory: &GuestTcxInventoryIdentity, _link_pin: PathBuf) -> Self {
        panic!("Not yet implemented -- RED scaffold (GH #295 D12 adopted state)")
    }
    #[expect(clippy::panic, reason = "D12 RED scaffold")]
    pub fn adopt_endpoint_map(&mut self) -> Result<GuestTcxMapSchema, GuestTcxError> {
        panic!("Not yet implemented -- RED scaffold (GH #295 D12 adopt endpoint map)")
    }
    #[expect(clippy::panic, reason = "D12 RED scaffold")]
    pub fn adopt_counter_map(&mut self) -> Result<GuestTcxMapSchema, GuestTcxError> {
        panic!("Not yet implemented -- RED scaffold (GH #295 D12 adopt counter map)")
    }
    #[expect(clippy::panic, reason = "D12 RED scaffold")]
    pub fn adopt_link(&mut self) -> Result<(), GuestTcxError> {
        panic!("Not yet implemented -- RED scaffold (GH #295 D12 adopt link)")
    }
    #[expect(clippy::panic, reason = "D12 RED scaffold")]
    pub fn read_endpoint(&self, _ifindex: u32) -> Result<Option<GuestTcxEndpoint>, GuestTcxError> {
        panic!("Not yet implemented -- RED scaffold (GH #295 D12 adopted endpoint read-back)")
    }
    #[expect(clippy::panic, reason = "D12 RED scaffold")]
    pub fn unpin_link(&mut self) -> Result<Option<GuestTcxLink>, GuestTcxError> {
        panic!("Not yet implemented -- RED scaffold (GH #295 D12 adopted link unpin)")
    }
    #[expect(clippy::panic, reason = "D12 RED scaffold")]
    pub fn unpin_counter_map(&mut self) -> Result<(), GuestTcxError> {
        panic!("Not yet implemented -- RED scaffold (GH #295 D12 counter-map unpin)")
    }
    #[expect(clippy::panic, reason = "D12 RED scaffold")]
    pub fn unpin_endpoint_map(&mut self) -> Result<(), GuestTcxError> {
        panic!("Not yet implemented -- RED scaffold (GH #295 D12 endpoint-map unpin)")
    }
}

#[allow(clippy::unused_self, reason = "D12 exact observation RED signatures")]
impl GuestTcxInventoryIdentity {
    pub fn capture(
        endpoint_map_pin: PathBuf,
        counter_map_pin: PathBuf,
    ) -> GuestTcxInventoryCapture {
        capture_with_source(endpoint_map_pin, counter_map_pin, Arc::new(AyaGuestTcxInventorySource))
    }

    #[expect(clippy::panic, reason = "D12 RED scaffold")]
    pub fn observe_endpoint_maps(&self) -> Result<u32, GuestTcxError> {
        panic!("Not yet implemented -- RED scaffold (GH #295 D12 endpoint-map inventory)")
    }
    #[expect(clippy::panic, reason = "D12 RED scaffold")]
    pub fn observe_counter_maps(&self) -> Result<u32, GuestTcxError> {
        panic!("Not yet implemented -- RED scaffold (GH #295 D12 counter-map inventory)")
    }
    #[expect(clippy::panic, reason = "D12 RED scaffold")]
    pub fn observe_endpoint_entries(&self) -> Result<u32, GuestTcxError> {
        panic!("Not yet implemented -- RED scaffold (GH #295 D12 endpoint-entry inventory)")
    }
    #[expect(clippy::panic, reason = "D12 RED scaffold")]
    pub fn observe_tcx_programs(&self) -> Result<u32, GuestTcxError> {
        panic!("Not yet implemented -- RED scaffold (GH #295 D12 program inventory)")
    }
    #[expect(clippy::panic, reason = "D12 RED scaffold")]
    pub fn observe_tcx_links(&self) -> Result<u32, GuestTcxError> {
        panic!("Not yet implemented -- RED scaffold (GH #295 D12 link inventory)")
    }
    #[expect(clippy::panic, reason = "D12 RED scaffold")]
    pub fn observe_endpoint_map_pins(&self) -> Result<u32, GuestTcxError> {
        panic!("Not yet implemented -- RED scaffold (GH #295 D12 endpoint-map-pin inventory)")
    }
    #[expect(clippy::panic, reason = "D12 RED scaffold")]
    pub fn observe_counter_map_pins(&self) -> Result<u32, GuestTcxError> {
        panic!("Not yet implemented -- RED scaffold (GH #295 D12 counter-map-pin inventory)")
    }
    #[expect(clippy::panic, reason = "D12 RED scaffold")]
    pub fn observe_tcx_link_pins(&self) -> Result<u32, GuestTcxError> {
        panic!("Not yet implemented -- RED scaffold (GH #295 D12 link-pin inventory)")
    }
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
#[allow(clippy::doc_markdown, clippy::expect_used, dead_code)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::error::Error as _;

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

    /// S-ND295-00 — every valid raw map kind keeps an honest semantic identity.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER step 02-01: S-ND295-00 D12 closed raw map-kind projection"]
    fn every_locked_aya_map_kind_projects_to_exact_or_opaque_semantics() {
        use aya::maps::MapType;

        assert_eq!(project_map_kind(MapType::Hash), GuestTcxMapKind::Hash);
        assert_eq!(project_map_kind(MapType::Array), GuestTcxMapKind::Array);

        let unsupported = [
            MapType::Unspecified,
            MapType::ProgramArray,
            MapType::PerfEventArray,
            MapType::PerCpuHash,
            MapType::PerCpuArray,
            MapType::StackTrace,
            MapType::CgroupArray,
            MapType::LruHash,
            MapType::LruPerCpuHash,
            MapType::LpmTrie,
            MapType::ArrayOfMaps,
            MapType::HashOfMaps,
            MapType::DevMap,
            MapType::SockMap,
            MapType::CpuMap,
            MapType::XskMap,
            MapType::SockHash,
            MapType::CgroupStorage,
            MapType::ReuseportSockArray,
            MapType::PerCpuCgroupStorage,
            MapType::Queue,
            MapType::Stack,
            MapType::SkStorage,
            MapType::DevMapHash,
            MapType::StructOps,
            MapType::RingBuf,
            MapType::InodeStorage,
            MapType::TaskStorage,
            MapType::BloomFilter,
            MapType::UserRingBuf,
            MapType::CgrpStorage,
            MapType::Arena,
        ];
        let projected: Vec<_> = unsupported.into_iter().map(project_map_kind).collect();
        assert!(projected.iter().all(|kind| matches!(kind, GuestTcxMapKind::Unsupported(_))));
        for (index, left) in projected.iter().enumerate() {
            assert_eq!(format!("{left:?}"), "Unsupported");
            for right in &projected[index + 1..] {
                assert_ne!(left, right, "distinct valid kernel kinds retain opaque equality");
            }
        }
    }

    /// S-ND295-00 — wrong valid properties remain opaque and source-less.
    /// CONTRACT_SHAPE: pure-function.
    #[allow(clippy::too_many_lines, reason = "one closed schema table is easier to audit intact")]
    #[test]
    #[ignore = "pending DELIVER step 02-01: S-ND295-00 D12 opaque schema projection"]
    fn wrong_valid_map_properties_remain_opaque_and_schema_mismatch_is_source_less() {
        let endpoint = RawGuestTcxMapObservation {
            id: 17,
            kind: aya::maps::MapType::Hash,
            key_size: 4,
            value_size: 16,
            max_entries: 65_536,
            name: b"ENDPOINTS".to_vec(),
        };
        let counter = RawGuestTcxMapObservation {
            id: 18,
            kind: aya::maps::MapType::Array,
            key_size: 4,
            value_size: 8,
            max_entries: 8,
            name: b"COUNTERS".to_vec(),
        };
        let mut wrong_kind = endpoint.clone();
        wrong_kind.kind = aya::maps::MapType::Array;
        let mut unsupported_kind = endpoint.clone();
        unsupported_kind.kind = aya::maps::MapType::LruHash;
        let mut wrong_key_a = endpoint.clone();
        wrong_key_a.key_size = 8;
        let mut wrong_key_b = endpoint.clone();
        wrong_key_b.key_size = 16;
        let mut wrong_value_a = endpoint.clone();
        wrong_value_a.value_size = 17;
        let mut wrong_value_b = endpoint.clone();
        wrong_value_b.value_size = 18;
        let mut wrong_capacity_a = endpoint.clone();
        wrong_capacity_a.max_entries = 65_535;
        let mut wrong_capacity_b = endpoint.clone();
        wrong_capacity_b.max_entries = 65_534;
        let mut wrong_capacity_above = endpoint.clone();
        wrong_capacity_above.max_entries = 65_537;
        let mut wrong_counter_kind = counter.clone();
        wrong_counter_kind.kind = aya::maps::MapType::Hash;
        let mut wrong_counter_key_a = counter.clone();
        wrong_counter_key_a.key_size = 8;
        let mut wrong_counter_key_b = counter.clone();
        wrong_counter_key_b.key_size = 16;
        let mut wrong_counter_value_a = counter.clone();
        wrong_counter_value_a.value_size = 9;
        let mut wrong_counter_value_b = counter.clone();
        wrong_counter_value_b.value_size = 10;
        let mut wrong_counter_capacity_a = counter.clone();
        wrong_counter_capacity_a.max_entries = 7;
        let mut wrong_counter_capacity_b = counter.clone();
        wrong_counter_capacity_b.max_entries = 9;

        let endpoint_schema = project_map_schema(&endpoint);
        let counter_schema = project_map_schema(&counter);
        assert_eq!(
            endpoint_schema,
            GuestTcxMapSchema {
                kind: GuestTcxMapKind::Hash,
                key: GuestTcxMapKeyShape::U32,
                value: GuestTcxMapValueShape::EndpointAbi,
                capacity: GuestTcxMapCapacity::EndpointMaximum,
            }
        );
        assert_eq!(
            counter_schema,
            GuestTcxMapSchema {
                kind: GuestTcxMapKind::Array,
                key: GuestTcxMapKeyShape::U32,
                value: GuestTcxMapValueShape::CounterU64,
                capacity: GuestTcxMapCapacity::CounterSlots,
            }
        );

        let observed_kind = project_map_schema(&wrong_kind);
        let observed_unsupported_kind = project_map_schema(&unsupported_kind);
        let observed_key_a = project_map_schema(&wrong_key_a);
        let observed_key_b = project_map_schema(&wrong_key_b);
        let observed_value_a = project_map_schema(&wrong_value_a);
        let observed_value_b = project_map_schema(&wrong_value_b);
        let observed_capacity_a = project_map_schema(&wrong_capacity_a);
        let observed_capacity_b = project_map_schema(&wrong_capacity_b);
        let observed_capacity_above = project_map_schema(&wrong_capacity_above);
        let observed_counter_kind = project_map_schema(&wrong_counter_kind);
        let observed_counter_key_a = project_map_schema(&wrong_counter_key_a);
        let observed_counter_key_b = project_map_schema(&wrong_counter_key_b);
        let observed_counter_value_a = project_map_schema(&wrong_counter_value_a);
        let observed_counter_value_b = project_map_schema(&wrong_counter_value_b);
        let observed_counter_capacity_a = project_map_schema(&wrong_counter_capacity_a);
        let observed_counter_capacity_b = project_map_schema(&wrong_counter_capacity_b);
        assert_eq!(observed_kind.kind, GuestTcxMapKind::Array);
        assert!(matches!(observed_unsupported_kind.kind, GuestTcxMapKind::Unsupported(_)));
        assert!(matches!(observed_key_a.key, GuestTcxMapKeyShape::Unsupported(_)));
        assert_ne!(observed_key_a.key, observed_key_b.key);
        assert!(matches!(observed_value_a.value, GuestTcxMapValueShape::Unsupported(_)));
        assert!(matches!(observed_capacity_a.capacity, GuestTcxMapCapacity::Unsupported(_)));
        assert_ne!(observed_value_a.value, observed_value_b.value);
        assert_ne!(observed_capacity_a.capacity, observed_capacity_b.capacity);
        assert_ne!(observed_capacity_above.capacity, endpoint_schema.capacity);
        assert_eq!(observed_counter_kind.kind, GuestTcxMapKind::Hash);
        assert!(matches!(observed_counter_key_a.key, GuestTcxMapKeyShape::Unsupported(_)));
        assert_ne!(observed_counter_key_a.key, observed_counter_key_b.key);
        assert!(matches!(observed_counter_value_a.value, GuestTcxMapValueShape::Unsupported(_)));
        assert!(matches!(
            observed_counter_capacity_a.capacity,
            GuestTcxMapCapacity::Unsupported(_)
        ));
        assert_ne!(observed_counter_value_a.value, observed_counter_value_b.value);
        assert_ne!(observed_counter_capacity_a.capacity, observed_counter_capacity_b.capacity);
        for opaque in [
            format!("{:?}", observed_unsupported_kind.kind),
            format!("{:?}", observed_key_a.key),
            format!("{:?}", observed_value_a.value),
            format!("{:?}", observed_capacity_a.capacity),
            format!("{:?}", observed_counter_value_a.value),
            format!("{:?}", observed_counter_capacity_a.capacity),
        ] {
            assert!(opaque.contains("Unsupported"));
            assert!(!opaque.chars().any(|character| character.is_ascii_digit()));
        }

        let error = GuestTcxError::MapSchemaMismatch {
            expected: endpoint_schema,
            observed: observed_value_a,
        };
        assert!(error.source().is_none(), "semantic mismatch never fabricates an aya source");
        for semantic in [
            GuestTcxError::InventoryAmbiguous { family: GuestTcxInventoryFamily::EndpointMap },
            GuestTcxError::OwnershipMismatch { family: GuestTcxInventoryFamily::EndpointMapPin },
            GuestTcxError::CaptureUnavailable { family: GuestTcxInventoryFamily::TcxProgram },
            GuestTcxError::ObjectMissing { object: GuestTcxObject::Classifier },
        ] {
            assert!(semantic.source().is_none());
        }
        let sourced = GuestTcxError::Map {
            source: aya::maps::MapError::IoError(std::io::Error::from_raw_os_error(libc::EIO)),
        };
        assert!(sourced.source().is_some());
    }

    #[derive(Default)]
    struct ScriptedInventorySource {
        fail_maps: bool,
        fail_programs: bool,
        fail_links: bool,
        maps: parking_lot::Mutex<Vec<RawGuestTcxMapObservation>>,
        programs: parking_lot::Mutex<Vec<RawGuestTcxProgramObservation>>,
        links: parking_lot::Mutex<Vec<RawGuestTcxLinkObservation>>,
        endpoint_entries: parking_lot::Mutex<BTreeSet<(u32, u32)>>,
        pins: parking_lot::Mutex<std::collections::BTreeMap<PathBuf, RawGuestTcxPinObservation>>,
        calls: parking_lot::Mutex<Vec<&'static str>>,
    }

    impl GuestTcxInventorySource for ScriptedInventorySource {
        fn loaded_maps(&self) -> Result<Vec<RawGuestTcxMapObservation>, GuestTcxError> {
            self.calls.lock().push("maps");
            if self.fail_maps {
                Err(GuestTcxError::Map {
                    source: aya::maps::MapError::IoError(std::io::Error::from_raw_os_error(
                        libc::EIO,
                    )),
                })
            } else {
                Ok(self.maps.lock().clone())
            }
        }

        fn loaded_programs(&self) -> Result<Vec<RawGuestTcxProgramObservation>, GuestTcxError> {
            self.calls.lock().push("programs");
            if self.fail_programs {
                Err(GuestTcxError::Program {
                    source: aya::programs::ProgramError::IOError(
                        std::io::Error::from_raw_os_error(libc::EIO),
                    ),
                })
            } else {
                Ok(self.programs.lock().clone())
            }
        }

        fn loaded_links(&self) -> Result<Vec<RawGuestTcxLinkObservation>, GuestTcxError> {
            self.calls.lock().push("links");
            if self.fail_links {
                Err(GuestTcxError::Program {
                    source: aya::programs::ProgramError::IOError(
                        std::io::Error::from_raw_os_error(libc::ENOENT),
                    ),
                })
            } else {
                Ok(self.links.lock().clone())
            }
        }

        fn map_by_id(&self, id: u32) -> Result<Option<RawGuestTcxMapObservation>, GuestTcxError> {
            Ok(self.maps.lock().iter().find(|map| map.id == id).cloned())
        }

        fn endpoint_present_by_id(&self, map_id: u32, ifindex: u32) -> Result<bool, GuestTcxError> {
            Ok(self.endpoint_entries.lock().contains(&(map_id, ifindex)))
        }

        fn observe_pin(&self, path: &Path) -> Result<RawGuestTcxPinObservation, GuestTcxError> {
            Ok(self.pins.lock().get(path).cloned().unwrap_or(RawGuestTcxPinObservation::Absent))
        }
    }

    fn assert_capture_family(
        observed: Result<u32, GuestTcxError>,
        unavailable: bool,
        family: GuestTcxInventoryFamily,
    ) {
        if unavailable {
            let error = observed.expect_err("failed capture domain must be unavailable");
            assert!(matches!(
                &error,
                GuestTcxError::CaptureUnavailable { family: actual } if *actual == family
            ));
            assert!(error.source().is_none(), "later family unavailability has no cloned source");
        } else {
            assert!(matches!(observed, Ok(0)), "independent completed observation is exact zero");
        }
    }

    fn endpoint_map(id: u32) -> RawGuestTcxMapObservation {
        RawGuestTcxMapObservation {
            id,
            kind: aya::maps::MapType::Hash,
            key_size: 4,
            value_size: 16,
            max_entries: 65_536,
            name: b"ENDPOINTS".to_vec(),
        }
    }

    fn counter_map(id: u32) -> RawGuestTcxMapObservation {
        RawGuestTcxMapObservation {
            id,
            kind: aya::maps::MapType::Array,
            key_size: 4,
            value_size: 8,
            max_entries: 8,
            name: b"COUNTERS".to_vec(),
        }
    }

    fn classifier(id: u32, map_ids: Vec<u32>) -> RawGuestTcxProgramObservation {
        RawGuestTcxProgramObservation {
            id,
            tag: 0x295,
            name: b"guest_tcx_classifier".to_vec(),
            program_type: aya::programs::ProgramType::SchedClassifier,
            map_ids,
        }
    }

    fn ingress_link(id: u32, program_id: u32) -> RawGuestTcxLinkObservation {
        RawGuestTcxLinkObservation { id, program_id, target_ifindex: 295, attach_type: 0 }
    }

    /// S-ND295-00 — capture failure preserves observation and first-source truth.
    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    #[ignore = "pending DELIVER step 02-01: S-ND295-00 D12 once-moved capture failure and eight-family continuation"]
    fn capture_failure_keeps_an_observation_identity_and_only_the_first_genuine_source() {
        for (fail_maps, fail_programs, fail_links) in [
            (true, false, false),
            (false, true, false),
            (false, false, true),
            (false, true, true),
            (true, true, true),
        ] {
            let source = Arc::new(ScriptedInventorySource {
                fail_maps,
                fail_programs,
                fail_links,
                ..ScriptedInventorySource::default()
            });
            let capture = capture_with_source(
                PathBuf::from("/sys/fs/bpf/overdrive/mtls-endpoints/maps/endpoints"),
                PathBuf::from("/sys/fs/bpf/overdrive/mtls-endpoints/maps/counters"),
                source.clone(),
            );
            let (identity, disposition) = capture.into_parts();
            let first = disposition.expect_err("at least one capture domain fails in each row");
            if fail_maps {
                assert!(matches!(
                    &first,
                    GuestTcxError::Map {
                        source: aya::maps::MapError::IoError(source)
                    } if source.raw_os_error() == Some(libc::EIO)
                ));
            } else if fail_programs {
                assert!(matches!(
                    &first,
                    GuestTcxError::Program {
                        source: aya::programs::ProgramError::IOError(source),
                    } if source.raw_os_error() == Some(libc::EIO)
                ));
                assert!(first.source().is_some(), "the genuine program-domain source is retained");
            } else {
                assert!(matches!(
                    &first,
                    GuestTcxError::Program {
                        source: aya::programs::ProgramError::IOError(source),
                    } if source.raw_os_error() == Some(libc::ENOENT)
                ));
                assert!(
                    first.source().is_some(),
                    "the outer Program retains the genuine link-enumeration source"
                );
            }
            assert_eq!(
                source.calls.lock().as_slice(),
                ["maps", "programs", "links"],
                "capture exhausts every domain in fixed order despite the first failure"
            );

            assert_capture_family(
                identity.observe_endpoint_maps(),
                fail_maps,
                GuestTcxInventoryFamily::EndpointMap,
            );
            assert_capture_family(
                identity.observe_counter_maps(),
                fail_maps,
                GuestTcxInventoryFamily::CounterMap,
            );
            assert_capture_family(
                identity.observe_endpoint_entries(),
                fail_maps,
                GuestTcxInventoryFamily::EndpointEntry,
            );
            assert_capture_family(
                identity.observe_tcx_programs(),
                fail_programs,
                GuestTcxInventoryFamily::TcxProgram,
            );
            assert_capture_family(
                identity.observe_tcx_links(),
                fail_links,
                GuestTcxInventoryFamily::TcxLink,
            );
            if !fail_maps && fail_programs && fail_links {
                let unavailable = identity
                    .observe_tcx_links()
                    .expect_err("the later failed link domain remains unavailable");
                assert!(matches!(
                    &unavailable,
                    GuestTcxError::CaptureUnavailable { family: GuestTcxInventoryFamily::TcxLink }
                ));
                assert!(unavailable.source().is_none());
            }
            assert!(matches!(identity.observe_endpoint_map_pins(), Ok(0)));
            assert!(matches!(identity.observe_counter_map_pins(), Ok(0)));
            assert!(matches!(identity.observe_tcx_link_pins(), Ok(0)));
        }
    }

    /// S-ND295-00 — an unreceipted candidate is never attributed as owned.
    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    #[ignore = "pending DELIVER step 02-01: S-ND295-00 D12 receipt-only owned inventory"]
    fn a_unique_unreceipted_candidate_is_ambiguous_and_never_an_owned_count() {
        let source = Arc::new(ScriptedInventorySource::default());
        let endpoint_pin = PathBuf::from("/sys/fs/bpf/overdrive/mtls-endpoints/maps/endpoints");
        let counter_pin = PathBuf::from("/sys/fs/bpf/overdrive/mtls-endpoints/maps/counters");
        let link_pin =
            PathBuf::from("/sys/fs/bpf/overdrive/mtls-endpoints/links/ovd-tp-0127-ingress");
        let capture =
            capture_with_source(endpoint_pin.clone(), counter_pin.clone(), source.clone());
        let (identity, disposition) = capture.into_parts();
        disposition.expect("complete empty baseline capture");
        *identity.link_pin.lock() = Some(link_pin.clone());
        source.maps.lock().extend([endpoint_map(295), counter_map(296)]);
        source.programs.lock().push(classifier(297, vec![295, 296]));
        source.links.lock().push(ingress_link(298, 297));
        source.endpoint_entries.lock().insert((295, 295));
        source.pins.lock().extend([
            (endpoint_pin, RawGuestTcxPinObservation::Map(endpoint_map(295))),
            (counter_pin, RawGuestTcxPinObservation::Map(counter_map(296))),
            (link_pin, RawGuestTcxPinObservation::Link(ingress_link(298, 297))),
        ]);

        let observations = [
            (identity.observe_endpoint_maps(), GuestTcxInventoryFamily::EndpointMap),
            (identity.observe_counter_maps(), GuestTcxInventoryFamily::CounterMap),
            (identity.observe_endpoint_entries(), GuestTcxInventoryFamily::EndpointEntry),
            (identity.observe_tcx_programs(), GuestTcxInventoryFamily::TcxProgram),
            (identity.observe_tcx_links(), GuestTcxInventoryFamily::TcxLink),
            (identity.observe_endpoint_map_pins(), GuestTcxInventoryFamily::EndpointMapPin),
            (identity.observe_counter_map_pins(), GuestTcxInventoryFamily::CounterMapPin),
            (identity.observe_tcx_link_pins(), GuestTcxInventoryFamily::TcxLinkPin),
        ];
        for (observed, family) in observations {
            let error = observed.expect_err("an unreceipted candidate is never an owned count");
            assert!(matches!(
                &error,
                GuestTcxError::InventoryAmbiguous { family: actual } if *actual == family
            ));
            assert!(error.source().is_none());
        }

        source.maps.lock().extend([endpoint_map(395), counter_map(396)]);
        source.programs.lock().push(classifier(397, vec![395, 396]));
        source.links.lock().push(ingress_link(398, 397));
        source.endpoint_entries.lock().insert((395, 396));
        for (observed, family) in [
            (identity.observe_endpoint_maps(), GuestTcxInventoryFamily::EndpointMap),
            (identity.observe_counter_maps(), GuestTcxInventoryFamily::CounterMap),
            (identity.observe_endpoint_entries(), GuestTcxInventoryFamily::EndpointEntry),
            (identity.observe_tcx_programs(), GuestTcxInventoryFamily::TcxProgram),
            (identity.observe_tcx_links(), GuestTcxInventoryFamily::TcxLink),
        ] {
            assert!(matches!(
                observed,
                Err(GuestTcxError::InventoryAmbiguous { family: actual }) if actual == family
            ));
        }
    }

    /// S-ND295-00 — exact private receipts are the only source of positive counts.
    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    #[ignore = "pending DELIVER step 02-01: S-ND295-00 exact positive receipted inventory"]
    fn every_receipted_family_returns_one_and_clean_families_return_exact_zero() {
        let source = Arc::new(ScriptedInventorySource::default());
        let endpoint_pin = PathBuf::from("/sys/fs/bpf/overdrive/mtls-endpoints/maps/endpoints");
        let counter_pin = PathBuf::from("/sys/fs/bpf/overdrive/mtls-endpoints/maps/counters");
        let link_pin =
            PathBuf::from("/sys/fs/bpf/overdrive/mtls-endpoints/links/ovd-tp-0127-ingress");
        let capture =
            capture_with_source(endpoint_pin.clone(), counter_pin.clone(), source.clone());
        let (identity, disposition) = capture.into_parts();
        disposition.expect("complete empty baseline capture");
        for observed in [
            identity.observe_endpoint_maps(),
            identity.observe_counter_maps(),
            identity.observe_endpoint_entries(),
            identity.observe_tcx_programs(),
            identity.observe_tcx_links(),
            identity.observe_endpoint_map_pins(),
            identity.observe_counter_map_pins(),
            identity.observe_tcx_link_pins(),
        ] {
            assert!(matches!(observed, Ok(0)));
        }

        *identity.link_pin.lock() = Some(link_pin.clone());
        source.maps.lock().extend([endpoint_map(295), counter_map(296)]);
        source.programs.lock().push(classifier(297, vec![295, 296]));
        source.links.lock().push(ingress_link(298, 297));
        source.endpoint_entries.lock().insert((295, 295));
        source.pins.lock().extend([
            (endpoint_pin, RawGuestTcxPinObservation::Map(endpoint_map(295))),
            (counter_pin, RawGuestTcxPinObservation::Map(counter_map(296))),
            (link_pin, RawGuestTcxPinObservation::Link(ingress_link(298, 297))),
        ]);
        *identity.receipts.lock() = GuestTcxInventoryReceipts {
            endpoint_map_id: Some(295),
            counter_map_id: Some(296),
            endpoint_ifindices: BTreeSet::from([295]),
            program_id: Some(297),
            link_id: Some(298),
            endpoint_map_pin_id: Some(295),
            counter_map_pin_id: Some(296),
            link_pin_id: Some(298),
        };
        for observed in [
            identity.observe_endpoint_maps(),
            identity.observe_counter_maps(),
            identity.observe_endpoint_entries(),
            identity.observe_tcx_programs(),
            identity.observe_tcx_links(),
            identity.observe_endpoint_map_pins(),
            identity.observe_counter_map_pins(),
            identity.observe_tcx_link_pins(),
        ] {
            assert!(matches!(observed, Ok(1)));
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
