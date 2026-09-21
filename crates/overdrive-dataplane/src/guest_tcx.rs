//! Semantic boundary for the shared guest-network TCX adapter (GH #295).

use std::collections::{BTreeMap, BTreeSet};
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use aya::Pod;
use aya::maps::{Array, HashMap, Map, MapData};
use aya::programs::links::{FdLink, Link as _};
use aya::programs::tc::{SchedClassifier, TcAttachOptions, TcAttachType};

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
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum GuestTcxMapKind {
    Hash,
    Array,
    Unsupported(GuestTcxUnsupportedMapKind),
}

impl std::fmt::Debug for GuestTcxMapKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Hash => formatter.write_str("Hash"),
            Self::Array => formatter.write_str("Array"),
            Self::Unsupported(_) => formatter.write_str("Unsupported"),
        }
    }
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

const TCX_INGRESS_ATTACH_TYPE: u32 = 46;

const fn expected_map_schema(endpoint: bool) -> GuestTcxMapSchema {
    if endpoint {
        GuestTcxMapSchema {
            kind: GuestTcxMapKind::Hash,
            key: GuestTcxMapKeyShape::U32,
            value: GuestTcxMapValueShape::EndpointAbi,
            capacity: GuestTcxMapCapacity::EndpointMaximum,
        }
    } else {
        GuestTcxMapSchema {
            kind: GuestTcxMapKind::Array,
            key: GuestTcxMapKeyShape::U32,
            value: GuestTcxMapValueShape::CounterU64,
            capacity: GuestTcxMapCapacity::CounterSlots,
        }
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
#[allow(dead_code, reason = "private aya projection records remain source-local")]
struct RawGuestTcxMapObservation {
    id: u32,
    kind: aya::maps::MapType,
    key_size: u32,
    value_size: u32,
    max_entries: u32,
    name: Vec<u8>,
}

#[derive(Clone)]
#[allow(dead_code, reason = "private aya projection records remain source-local")]
struct RawGuestTcxProgramObservation {
    id: u32,
    tag: u64,
    name: Vec<u8>,
    program_type: aya::programs::ProgramType,
    map_ids: Vec<u32>,
}

#[derive(Clone)]
#[allow(dead_code, reason = "private aya projection records remain source-local")]
struct RawGuestTcxLinkObservation {
    id: u32,
    program_id: u32,
    target_ifindex: u32,
    attach_type: u32,
}

#[derive(Clone)]
#[allow(dead_code, reason = "private aya projection records remain source-local")]
enum RawGuestTcxPinObservation {
    Absent,
    Map(RawGuestTcxMapObservation),
    Link(RawGuestTcxLinkObservation),
    Other,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct EndpointAbi {
    source_ip: u32,
    source_mac: [u8; 6],
    source_pad: [u8; 2],
    bridge_mac: [u8; 6],
    bridge_pad: [u8; 2],
}

// SAFETY: EndpointAbi is a C-layout collection of integers and byte arrays.
unsafe impl Pod for EndpointAbi {}

impl From<GuestTcxEndpoint> for EndpointAbi {
    fn from(value: GuestTcxEndpoint) -> Self {
        Self {
            source_ip: u32::from_be_bytes(value.source_ipv4.octets()),
            source_mac: value.source_mac,
            source_pad: [0; 2],
            bridge_mac: value.bridge_mac,
            bridge_pad: [0; 2],
        }
    }
}

impl From<EndpointAbi> for GuestTcxEndpoint {
    fn from(value: EndpointAbi) -> Self {
        Self {
            source_ipv4: Ipv4Addr::from(value.source_ip.to_be_bytes()),
            source_mac: value.source_mac,
            bridge_mac: value.bridge_mac,
        }
    }
}

fn schema_for_map_data(map: &MapData, endpoint: bool) -> Result<GuestTcxMapSchema, GuestTcxError> {
    let info = map.info().map_err(|source| GuestTcxError::Map { source })?;
    let raw = RawGuestTcxMapObservation {
        id: info.id(),
        kind: info.map_type().map_err(|source| GuestTcxError::Map { source })?,
        key_size: info.key_size(),
        value_size: info.value_size(),
        max_entries: info.max_entries(),
        name: if endpoint { b"ENDPOINTS".to_vec() } else { b"COUNTERS".to_vec() },
    };
    let observed = project_map_schema(&raw);
    let expected = if endpoint {
        GuestTcxMapSchema {
            kind: GuestTcxMapKind::Hash,
            key: GuestTcxMapKeyShape::U32,
            value: GuestTcxMapValueShape::EndpointAbi,
            capacity: GuestTcxMapCapacity::EndpointMaximum,
        }
    } else {
        GuestTcxMapSchema {
            kind: GuestTcxMapKind::Array,
            key: GuestTcxMapKeyShape::U32,
            value: GuestTcxMapValueShape::CounterU64,
            capacity: GuestTcxMapCapacity::CounterSlots,
        }
    };
    if observed != expected {
        return Err(GuestTcxError::MapSchemaMismatch { expected, observed });
    }
    Ok(observed)
}

fn schema_for_map(
    map: &aya::maps::Map,
    endpoint: bool,
) -> Result<GuestTcxMapSchema, GuestTcxError> {
    match map {
        aya::maps::Map::Array(data)
        | aya::maps::Map::HashMap(data)
        | aya::maps::Map::Unsupported(data)
        | aya::maps::Map::PerCpuArray(data)
        | aya::maps::Map::PerCpuHashMap(data)
        | aya::maps::Map::LruHashMap(data)
        | aya::maps::Map::PerCpuLruHashMap(data)
        | aya::maps::Map::BloomFilter(data)
        | aya::maps::Map::CpuMap(data)
        | aya::maps::Map::DevMap(data)
        | aya::maps::Map::DevMapHash(data)
        | aya::maps::Map::LpmTrie(data)
        | aya::maps::Map::PerfEventArray(data)
        | aya::maps::Map::ProgramArray(data)
        | aya::maps::Map::Queue(data)
        | aya::maps::Map::RingBuf(data)
        | aya::maps::Map::SockHash(data)
        | aya::maps::Map::SockMap(data)
        | aya::maps::Map::Stack(data)
        | aya::maps::Map::StackTraceMap(data)
        | aya::maps::Map::XskMap(data) => schema_for_map_data(data, endpoint),
    }
}

#[allow(dead_code, reason = "private source trait is exercised by the production aya adapter")]
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
    fn loaded_maps(&self) -> Result<Vec<RawGuestTcxMapObservation>, GuestTcxError> {
        aya::maps::loaded_maps()
            .map(|result| {
                let map = result.map_err(|source| GuestTcxError::Map { source })?;
                Ok(RawGuestTcxMapObservation {
                    id: map.id(),
                    kind: map.map_type().map_err(|source| GuestTcxError::Map { source })?,
                    key_size: map.key_size(),
                    value_size: map.value_size(),
                    max_entries: map.max_entries(),
                    name: map.name().to_vec(),
                })
            })
            .collect()
    }
    fn loaded_programs(&self) -> Result<Vec<RawGuestTcxProgramObservation>, GuestTcxError> {
        aya::programs::loaded_programs()
            .map(|result| {
                let program = result.map_err(|source| GuestTcxError::Program { source })?;
                Ok(RawGuestTcxProgramObservation {
                    id: program.id(),
                    tag: program.tag(),
                    name: program.name().to_vec(),
                    program_type: program
                        .program_type()
                        .map_err(|source| GuestTcxError::Program { source })?,
                    map_ids: program
                        .map_ids()
                        .map_err(|source| GuestTcxError::Program { source })?
                        .unwrap_or_default(),
                })
            })
            .collect()
    }
    fn loaded_links(&self) -> Result<Vec<RawGuestTcxLinkObservation>, GuestTcxError> {
        aya::programs::loaded_links()
            .map(|result| {
                let link = result.map_err(|source| GuestTcxError::Program { source })?;
                // `bpf_link_info` is intentionally decoded only here.  The
                // control-plane sees the semantic projection below.
                let id = link.id;
                let mut observation = RawGuestTcxLinkObservation {
                    id,
                    program_id: link.prog_id,
                    target_ifindex: 0,
                    attach_type: 0,
                };
                if link.type_ == 11 {
                    // SAFETY: the kernel reports a TCX link and initializes
                    // the tcx union arm as part of BPF_OBJ_GET_INFO_BY_FD.
                    let tcx = unsafe { link.__bindgen_anon_1.tcx };
                    observation.target_ifindex = tcx.ifindex;
                    observation.attach_type = tcx.attach_type;
                }
                Ok(observation)
            })
            .collect()
    }
    fn map_by_id(&self, id: u32) -> Result<Option<RawGuestTcxMapObservation>, GuestTcxError> {
        let info =
            aya::maps::MapInfo::from_id(id).map_err(|source| GuestTcxError::Map { source })?;
        Ok(Some(RawGuestTcxMapObservation {
            id: info.id(),
            kind: info.map_type().map_err(|source| GuestTcxError::Map { source })?,
            key_size: info.key_size(),
            value_size: info.value_size(),
            max_entries: info.max_entries(),
            name: info.name().to_vec(),
        }))
    }
    fn endpoint_present_by_id(&self, map_id: u32, ifindex: u32) -> Result<bool, GuestTcxError> {
        let map = aya::maps::HashMap::<_, u32, EndpointAbi>::try_from(Map::HashMap(
            aya::maps::MapData::from_id(map_id).map_err(|source| GuestTcxError::Map { source })?,
        ))
        .map_err(|source| GuestTcxError::Map { source })?;
        map.get(&ifindex, 0).map(|_| true).or_else(|error| match error {
            aya::maps::MapError::KeyNotFound => Ok(false),
            source => Err(GuestTcxError::Map { source }),
        })
    }
    fn observe_pin(&self, path: &Path) -> Result<RawGuestTcxPinObservation, GuestTcxError> {
        if path.components().any(|component| component.as_os_str() == "links") {
            match aya::programs::links::PinnedLink::from_pin(path) {
                Ok(_link) => {
                    // Aya does not expose the pinned link's info fd.  Resolve
                    // the exact kernel identity from the private link dump,
                    // using the interface encoded by the accepted pin name.
                    // A missing or non-unique candidate is ambiguity, never a
                    // fabricated zero identity.
                    let name = path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .and_then(|name| name.strip_suffix("-ingress"));
                    let Some(name) = name else {
                        return Err(GuestTcxError::InventoryAmbiguous {
                            family: GuestTcxInventoryFamily::TcxLinkPin,
                        });
                    };
                    let ifindex = std::fs::read_to_string(format!("/sys/class/net/{name}/ifindex"))
                        .map_err(|source| GuestTcxError::Io { source })?
                        .trim()
                        .parse::<u32>()
                        .map_err(|source| GuestTcxError::Io {
                            source: std::io::Error::new(std::io::ErrorKind::InvalidData, source),
                        })?;
                    let candidates = self
                        .loaded_links()?
                        .into_iter()
                        .filter(|link| {
                            link.target_ifindex == ifindex
                                && link.attach_type == TCX_INGRESS_ATTACH_TYPE
                        })
                        .collect::<Vec<_>>();
                    return match candidates.as_slice() {
                        [link] => Ok(RawGuestTcxPinObservation::Link(link.clone())),
                        _ => Err(GuestTcxError::InventoryAmbiguous {
                            family: GuestTcxInventoryFamily::TcxLinkPin,
                        }),
                    };
                }
                Err(aya::programs::links::LinkError::SyscallError(error))
                    if error.io_error.kind() == std::io::ErrorKind::NotFound =>
                {
                    return Ok(RawGuestTcxPinObservation::Absent);
                }
                Err(source) => return Err(GuestTcxError::Link { source }),
            }
        }
        match aya::maps::MapInfo::from_pin(path) {
            Ok(map) => Ok(RawGuestTcxPinObservation::Map(RawGuestTcxMapObservation {
                id: map.id(),
                kind: map.map_type().map_err(|source| GuestTcxError::Map { source })?,
                key_size: map.key_size(),
                value_size: map.value_size(),
                max_entries: map.max_entries(),
                name: map.name().to_vec(),
            })),
            Err(aya::maps::MapError::SyscallError(error))
                if error.io_error.kind() == std::io::ErrorKind::NotFound =>
            {
                Ok(RawGuestTcxPinObservation::Absent)
            }
            Err(source) => Err(GuestTcxError::Map { source }),
        }
    }
}

fn capture_with_source(
    endpoint_map_pin: PathBuf,
    counter_map_pin: PathBuf,
    source: Arc<dyn GuestTcxInventorySource>,
) -> GuestTcxInventoryCapture {
    let mut first_error = None;
    let (maps, maps_available) = match source.loaded_maps() {
        Ok(maps) => (maps, true),
        Err(error) => {
            first_error = Some(error);
            (Vec::new(), false)
        }
    };
    let (programs, programs_available) = match source.loaded_programs() {
        Ok(programs) => (programs, true),
        Err(error) => {
            if first_error.is_none() {
                first_error = Some(error);
            }
            (Vec::new(), false)
        }
    };
    let (links, links_available) = match source.loaded_links() {
        Ok(links) => (links, true),
        Err(error) => {
            if first_error.is_none() {
                first_error = Some(error);
            }
            (Vec::new(), false)
        }
    };
    let identity = GuestTcxInventoryIdentity {
        source,
        endpoint_map_pin,
        counter_map_pin,
        link_pin: Arc::new(parking_lot::Mutex::new(None)),
        receipts: Arc::new(parking_lot::Mutex::new(GuestTcxInventoryReceipts::default())),
        baseline: Arc::new(parking_lot::Mutex::new(GuestTcxInventoryBaseline {
            maps: maps.into_iter().map(|map| (map.id, map)).collect(),
            programs: programs.into_iter().map(|program| (program.id, program)).collect(),
            links: links.into_iter().map(|link| (link.id, link)).collect(),
            maps_available,
            programs_available,
            links_available,
        })),
    };
    GuestTcxInventoryCapture { identity, disposition: first_error.map_or(Ok(()), Err) }
}

const fn project_map_kind(raw: aya::maps::MapType) -> GuestTcxMapKind {
    match raw {
        aya::maps::MapType::Hash => GuestTcxMapKind::Hash,
        aya::maps::MapType::Array => GuestTcxMapKind::Array,
        unsupported => GuestTcxMapKind::Unsupported(GuestTcxUnsupportedMapKind(unsupported as u32)),
    }
}

fn project_map_schema(raw: &RawGuestTcxMapObservation) -> GuestTcxMapSchema {
    let key = if raw.key_size == 4 {
        GuestTcxMapKeyShape::U32
    } else {
        GuestTcxMapKeyShape::Unsupported(GuestTcxUnsupportedMapProperty(raw.key_size))
    };
    let endpoint = raw.name.as_slice() == b"ENDPOINTS";
    let counter = raw.name.as_slice() == b"COUNTERS";
    let value = if endpoint && (raw.value_size == 16 || raw.value_size == 20) {
        GuestTcxMapValueShape::EndpointAbi
    } else if counter && raw.value_size == 8 {
        GuestTcxMapValueShape::CounterU64
    } else {
        GuestTcxMapValueShape::Unsupported(GuestTcxUnsupportedMapProperty(raw.value_size))
    };
    let capacity = if endpoint && raw.max_entries == 65_536 {
        GuestTcxMapCapacity::EndpointMaximum
    } else if counter && raw.max_entries == 8 {
        GuestTcxMapCapacity::CounterSlots
    } else {
        GuestTcxMapCapacity::Unsupported(GuestTcxUnsupportedMapProperty(raw.max_entries))
    };
    GuestTcxMapSchema { kind: project_map_kind(raw.kind), key, value, capacity }
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
    baseline: Arc<parking_lot::Mutex<GuestTcxInventoryBaseline>>,
}

#[derive(Default)]
struct GuestTcxInventoryBaseline {
    maps: BTreeMap<u32, RawGuestTcxMapObservation>,
    programs: BTreeMap<u32, RawGuestTcxProgramObservation>,
    links: BTreeMap<u32, RawGuestTcxLinkObservation>,
    maps_available: bool,
    programs_available: bool,
    links_available: bool,
}

#[derive(Default)]
#[allow(dead_code, reason = "D12 exact private receipt scaffold")]
struct GuestTcxInventoryReceipts {
    endpoint_map_id: Option<u32>,
    counter_map_id: Option<u32>,
    endpoint_ifindices: BTreeSet<u32>,
    program_id: Option<u32>,
    link_id: Option<u32>,
    link_program_id: Option<u32>,
    link_target_ifindex: Option<u32>,
    link_attach_type: Option<u32>,
    endpoint_map_pin_id: Option<u32>,
    counter_map_pin_id: Option<u32>,
    link_pin_id: Option<u32>,
}

#[doc(hidden)]
pub struct GuestTcxProgram {
    inventory: GuestTcxInventoryIdentity,
    bpf: aya::Ebpf,
    endpoint_map: Option<HashMap<MapData, u32, EndpointAbi>>,
    counter_map: Option<Array<MapData, u64>>,
    endpoint_map_pin: Option<PathBuf>,
    counter_map_pin: Option<PathBuf>,
    endpoints: BTreeMap<u32, GuestTcxEndpoint>,
    program_id: u32,
}

#[doc(hidden)]
pub struct GuestTcxLink {
    link: Option<FdLink>,
    program_id: u32,
    target_ifindex: u32,
    attach_type: u32,
    inventory: GuestTcxInventoryIdentity,
}

#[doc(hidden)]
pub struct GuestTcxAdoptedState {
    inventory: GuestTcxInventoryIdentity,
    link_pin: PathBuf,
    endpoint_map: Option<HashMap<MapData, u32, EndpointAbi>>,
    counter_map: Option<Array<MapData, u64>>,
    link: Option<aya::programs::links::PinnedLink>,
}

impl GuestTcxProgram {
    pub fn load(inventory: &GuestTcxInventoryIdentity) -> Result<Self, GuestTcxError> {
        let mut bpf = aya::EbpfLoader::new()
            .allow_unsupported_maps()
            .load(super::OVERDRIVE_BPF_OBJ)
            .map_err(|source| GuestTcxError::Load { source })?;
        for (name, object) in
            [("ENDPOINTS", GuestTcxObject::EndpointMap), ("COUNTERS", GuestTcxObject::CounterMap)]
        {
            if bpf.map(name).is_none() {
                return Err(GuestTcxError::ObjectMissing { object });
            }
        }
        let program = bpf
            .program_mut("gh295c_endpoint")
            .ok_or(GuestTcxError::ObjectMissing { object: GuestTcxObject::Classifier })?;
        let classifier: &mut SchedClassifier =
            program.try_into().map_err(|source| GuestTcxError::Program { source })?;
        classifier.load().map_err(|source| GuestTcxError::Program { source })?;
        let program_id =
            classifier.info().map_err(|source| GuestTcxError::Program { source })?.id();
        inventory.receipts.lock().program_id = Some(program_id);
        Ok(Self {
            inventory: inventory.clone(),
            bpf,
            endpoint_map: None,
            counter_map: None,
            endpoint_map_pin: None,
            counter_map_pin: None,
            endpoints: BTreeMap::new(),
            program_id,
        })
    }
    pub fn pin_endpoint_map(&mut self, pin: &Path) -> Result<GuestTcxMapSchema, GuestTcxError> {
        let map = self
            .bpf
            .take_map("ENDPOINTS")
            .ok_or(GuestTcxError::ObjectMissing { object: GuestTcxObject::EndpointMap })?;
        let schema = schema_for_map(&map, true)?;
        let map_id = if let Map::HashMap(data) = &map {
            Some(data.info().map_err(|source| GuestTcxError::Map { source })?.id())
        } else {
            None
        };
        let map_id = map_id.ok_or(GuestTcxError::MapSchemaMismatch {
            expected: expected_map_schema(true),
            observed: schema,
        })?;
        map.pin(pin).map_err(|source| GuestTcxError::Pin { source })?;
        self.endpoint_map =
            Some(HashMap::try_from(map).map_err(|source| GuestTcxError::Map { source })?);
        self.inventory.receipts.lock().endpoint_map_id = Some(map_id);
        self.endpoint_map_pin = Some(pin.to_path_buf());
        Ok(schema)
    }
    pub fn pin_counter_map(&mut self, pin: &Path) -> Result<GuestTcxMapSchema, GuestTcxError> {
        let map = self
            .bpf
            .take_map("COUNTERS")
            .ok_or(GuestTcxError::ObjectMissing { object: GuestTcxObject::CounterMap })?;
        let schema = schema_for_map(&map, false)?;
        let map_id = if let Map::Array(data) = &map {
            Some(data.info().map_err(|source| GuestTcxError::Map { source })?.id())
        } else {
            None
        };
        let map_id = map_id.ok_or(GuestTcxError::MapSchemaMismatch {
            expected: expected_map_schema(false),
            observed: schema,
        })?;
        map.pin(pin).map_err(|source| GuestTcxError::Pin { source })?;
        self.counter_map =
            Some(Array::try_from(map).map_err(|source| GuestTcxError::Map { source })?);
        self.inventory.receipts.lock().counter_map_id = Some(map_id);
        self.counter_map_pin = Some(pin.to_path_buf());
        Ok(schema)
    }
    pub fn insert_endpoint(
        &mut self,
        ifindex: u32,
        endpoint: GuestTcxEndpoint,
    ) -> Result<(), GuestTcxError> {
        let map = self
            .endpoint_map
            .as_mut()
            .ok_or(GuestTcxError::ObjectMissing { object: GuestTcxObject::EndpointMap })?;
        self.inventory.receipts.lock().endpoint_ifindices.insert(ifindex);
        map.insert(ifindex, EndpointAbi::from(endpoint), 0)
            .map_err(|source| GuestTcxError::Map { source })?;
        self.endpoints.insert(ifindex, endpoint);
        Ok(())
    }
    pub fn read_endpoint(&self, ifindex: u32) -> Result<Option<GuestTcxEndpoint>, GuestTcxError> {
        if let Some(endpoint) = self.endpoints.get(&ifindex) {
            return Ok(Some(*endpoint));
        }
        let map = self
            .endpoint_map
            .as_ref()
            .ok_or(GuestTcxError::ObjectMissing { object: GuestTcxObject::EndpointMap })?;
        map.get(&ifindex, 0).map(|endpoint| Some(endpoint.into())).or_else(|error| match error {
            aya::maps::MapError::KeyNotFound => Ok(None),
            source => Err(GuestTcxError::Map { source }),
        })
    }
    pub fn attach_first_ingress(&mut self, interface: &str) -> Result<GuestTcxLink, GuestTcxError> {
        let target_ifindex = std::fs::read_to_string(format!("/sys/class/net/{interface}/ifindex"))
            .map_err(|source| GuestTcxError::Io { source })?
            .trim()
            .parse::<u32>()
            .map_err(|source| GuestTcxError::Io {
                source: std::io::Error::new(std::io::ErrorKind::InvalidData, source),
            })?;
        let program = self
            .bpf
            .program_mut("gh295c_endpoint")
            .ok_or(GuestTcxError::ObjectMissing { object: GuestTcxObject::Classifier })?;
        let classifier: &mut SchedClassifier =
            program.try_into().map_err(|source| GuestTcxError::Program { source })?;
        let id = classifier
            .attach_with_options(
                interface,
                TcAttachType::Ingress,
                TcAttachOptions::TcxOrder(aya::programs::LinkOrder::first()),
            )
            .map_err(|source| GuestTcxError::Program { source })?;
        let link = classifier.take_link(id).map_err(|source| GuestTcxError::Program { source })?;
        let fd_link: FdLink = link.try_into().map_err(|source| GuestTcxError::Link { source })?;
        Ok(GuestTcxLink {
            link: Some(fd_link),
            program_id: self.program_id,
            target_ifindex,
            attach_type: TCX_INGRESS_ATTACH_TYPE,
            inventory: self.inventory.clone(),
        })
    }
}

impl GuestTcxLink {
    pub const fn program_id(&self) -> u32 {
        self.program_id
    }
    pub fn pin(mut self, pin: &Path) -> Result<(), GuestTcxError> {
        *self.inventory.link_pin.lock() = Some(pin.to_path_buf());
        let candidates = self
            .inventory
            .source
            .loaded_links()?
            .into_iter()
            .filter(|link| {
                link.program_id == self.program_id
                    && link.target_ifindex == self.target_ifindex
                    && link.attach_type == self.attach_type
            })
            .collect::<Vec<_>>();
        let link_id = match candidates.as_slice() {
            [link] => link.id,
            _ => {
                return Err(GuestTcxError::InventoryAmbiguous {
                    family: GuestTcxInventoryFamily::TcxLink,
                });
            }
        };
        let link = self.link.take().ok_or_else(|| GuestTcxError::Io {
            source: std::io::Error::other("TCX link already consumed"),
        })?;
        link.pin(pin).map_err(|source| GuestTcxError::Pin { source })?;
        let mut receipts = self.inventory.receipts.lock();
        receipts.link_id = Some(link_id);
        receipts.link_pin_id = Some(link_id);
        receipts.link_program_id = Some(self.program_id);
        receipts.link_target_ifindex = Some(self.target_ifindex);
        receipts.link_attach_type = Some(self.attach_type);
        drop(receipts);
        Ok(())
    }
    pub fn detach(mut self) -> Result<(), GuestTcxError> {
        if let Some(link) = self.link.take() {
            link.detach().map_err(|source| GuestTcxError::Program { source })?;
        }
        Ok(())
    }
}

impl GuestTcxAdoptedState {
    pub fn for_inventory(inventory: &GuestTcxInventoryIdentity, link_pin: PathBuf) -> Self {
        *inventory.link_pin.lock() = Some(link_pin.clone());
        Self {
            inventory: inventory.clone(),
            link_pin,
            endpoint_map: None,
            counter_map: None,
            link: None,
        }
    }
    pub fn adopt_endpoint_map(&mut self) -> Result<GuestTcxMapSchema, GuestTcxError> {
        if self.endpoint_map.is_some() {
            return Ok(expected_map_schema(true));
        }
        let map = aya::maps::MapData::from_pin(&self.inventory.endpoint_map_pin)
            .map_err(|source| GuestTcxError::Map { source })?;
        let schema = schema_for_map_data(&map, true)?;
        let map_id = map.info().map_err(|source| GuestTcxError::Map { source })?.id();
        if self.inventory.receipts.lock().endpoint_map_id != Some(map_id) {
            return Err(GuestTcxError::OwnershipMismatch {
                family: GuestTcxInventoryFamily::EndpointMapPin,
            });
        }
        self.endpoint_map = Some(
            HashMap::try_from(Map::HashMap(map)).map_err(|source| GuestTcxError::Map { source })?,
        );
        Ok(schema)
    }
    pub fn adopt_counter_map(&mut self) -> Result<GuestTcxMapSchema, GuestTcxError> {
        if self.counter_map.is_some() {
            return Ok(expected_map_schema(false));
        }
        let map = aya::maps::MapData::from_pin(&self.inventory.counter_map_pin)
            .map_err(|source| GuestTcxError::Map { source })?;
        let schema = schema_for_map_data(&map, false)?;
        let map_id = map.info().map_err(|source| GuestTcxError::Map { source })?.id();
        if self.inventory.receipts.lock().counter_map_id != Some(map_id) {
            return Err(GuestTcxError::OwnershipMismatch {
                family: GuestTcxInventoryFamily::CounterMapPin,
            });
        }
        self.counter_map =
            Some(Array::try_from(Map::Array(map)).map_err(|source| GuestTcxError::Map { source })?);
        Ok(schema)
    }
    pub fn adopt_link(&mut self) -> Result<(), GuestTcxError> {
        if self.link.is_some() {
            return Ok(());
        }
        let receipts = self.inventory.receipts.lock();
        let Some(link_id) = receipts.link_id else {
            return Err(GuestTcxError::CaptureUnavailable {
                family: GuestTcxInventoryFamily::TcxLink,
            });
        };
        let Some(candidate) =
            self.inventory.source.loaded_links()?.into_iter().find(|link| link.id == link_id)
        else {
            return Err(GuestTcxError::OwnershipMismatch {
                family: GuestTcxInventoryFamily::TcxLinkPin,
            });
        };
        if receipts.link_program_id != Some(candidate.program_id)
            || receipts.link_target_ifindex != Some(candidate.target_ifindex)
            || receipts.link_attach_type != Some(candidate.attach_type)
        {
            return Err(GuestTcxError::OwnershipMismatch {
                family: GuestTcxInventoryFamily::TcxLinkPin,
            });
        }
        drop(receipts);
        self.link = Some(
            aya::programs::links::PinnedLink::from_pin(&self.link_pin)
                .map_err(|source| GuestTcxError::Link { source })?,
        );
        Ok(())
    }
    pub fn read_endpoint(&self, ifindex: u32) -> Result<Option<GuestTcxEndpoint>, GuestTcxError> {
        let map = self
            .endpoint_map
            .as_ref()
            .ok_or(GuestTcxError::ObjectMissing { object: GuestTcxObject::EndpointMap })?;
        map.get(&ifindex, 0).map(|endpoint| Some(endpoint.into())).or_else(|error| match error {
            aya::maps::MapError::KeyNotFound => Ok(None),
            source => Err(GuestTcxError::Map { source }),
        })
    }
    pub fn unpin_link(&mut self) -> Result<Option<GuestTcxLink>, GuestTcxError> {
        let Some(link) = self.link.take() else { return Ok(None) };
        let fd = link.unpin().map_err(|source| GuestTcxError::Io { source })?;
        Ok(Some(GuestTcxLink {
            link: Some(fd),
            program_id: self.inventory.receipts.lock().program_id.unwrap_or_default(),
            target_ifindex: self.inventory.receipts.lock().link_target_ifindex.unwrap_or_default(),
            attach_type: self
                .inventory
                .receipts
                .lock()
                .link_attach_type
                .unwrap_or(TCX_INGRESS_ATTACH_TYPE),
            inventory: self.inventory.clone(),
        }))
    }
    pub fn unpin_counter_map(&mut self) -> Result<(), GuestTcxError> {
        let _ = self.counter_map.take();
        match std::fs::remove_file(&self.inventory.counter_map_pin) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(GuestTcxError::Io { source }),
        }
    }
    pub fn unpin_endpoint_map(&mut self) -> Result<(), GuestTcxError> {
        let _ = self.endpoint_map.take();
        match std::fs::remove_file(&self.inventory.endpoint_map_pin) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(GuestTcxError::Io { source }),
        }
    }
}

impl GuestTcxInventoryIdentity {
    pub fn capture(
        endpoint_map_pin: PathBuf,
        counter_map_pin: PathBuf,
    ) -> GuestTcxInventoryCapture {
        capture_with_source(endpoint_map_pin, counter_map_pin, Arc::new(AyaGuestTcxInventorySource))
    }

    pub fn observe_endpoint_maps(&self) -> Result<u32, GuestTcxError> {
        self.observe_maps(true)
    }
    pub fn observe_counter_maps(&self) -> Result<u32, GuestTcxError> {
        self.observe_maps(false)
    }
    pub fn observe_endpoint_entries(&self) -> Result<u32, GuestTcxError> {
        let maps_available = self.baseline.lock().maps_available;
        if !maps_available {
            return Err(GuestTcxError::CaptureUnavailable {
                family: GuestTcxInventoryFamily::EndpointEntry,
            });
        }
        let (endpoint_map_id, endpoint_ifindices) = {
            let receipts = self.receipts.lock();
            (receipts.endpoint_map_id, receipts.endpoint_ifindices.clone())
        };
        let candidate_maps = self.source.loaded_maps().map_err(|_| {
            GuestTcxError::InventoryAmbiguous { family: GuestTcxInventoryFamily::EndpointEntry }
        })?;
        if !endpoint_ifindices.is_empty() {
            let Some(map_id) = endpoint_map_id else { return Ok(0) };
            let Some(map) = self.source.map_by_id(map_id)? else { return Ok(0) };
            let observed_schema = project_map_schema(&map);
            let expected_schema = expected_map_schema(true);
            if observed_schema != expected_schema {
                return Err(GuestTcxError::MapSchemaMismatch {
                    expected: expected_schema,
                    observed: observed_schema,
                });
            }
            let mut count = 0;
            for ifindex in &endpoint_ifindices {
                if self.source.endpoint_present_by_id(map_id, *ifindex)? {
                    count += 1;
                }
            }
            return Ok(count);
        }
        for map in candidate_maps {
            if project_map_schema(&map).kind == GuestTcxMapKind::Hash
                && self.source.endpoint_present_by_id(map.id, map.id)?
            {
                return Err(GuestTcxError::InventoryAmbiguous {
                    family: GuestTcxInventoryFamily::EndpointEntry,
                });
            }
        }
        Ok(0)
    }
    pub fn observe_tcx_programs(&self) -> Result<u32, GuestTcxError> {
        self.observe_programs()
    }
    pub fn observe_tcx_links(&self) -> Result<u32, GuestTcxError> {
        self.observe_links()
    }
    pub fn observe_endpoint_map_pins(&self) -> Result<u32, GuestTcxError> {
        self.observe_pin(true)
    }
    pub fn observe_counter_map_pins(&self) -> Result<u32, GuestTcxError> {
        self.observe_pin(false)
    }
    pub fn observe_tcx_link_pins(&self) -> Result<u32, GuestTcxError> {
        let path = self.link_pin.lock().clone();
        let Some(path) = path else { return Ok(0) };
        self.observe_link_pin_path(&path)
    }

    fn observe_maps(&self, endpoint: bool) -> Result<u32, GuestTcxError> {
        let family = if endpoint {
            GuestTcxInventoryFamily::EndpointMap
        } else {
            GuestTcxInventoryFamily::CounterMap
        };
        if !self.baseline.lock().maps_available {
            return Err(GuestTcxError::CaptureUnavailable { family });
        }
        let receipt = if endpoint {
            self.receipts.lock().endpoint_map_id
        } else {
            self.receipts.lock().counter_map_id
        };
        let maps =
            self.source.loaded_maps().map_err(|_| GuestTcxError::InventoryAmbiguous { family })?;
        if let Some(id) = receipt {
            let Some(map) = maps.iter().find(|map| map.id == id) else {
                return Ok(0);
            };
            let observed = project_map_schema(map);
            let expected = expected_map_schema(endpoint);
            if observed != expected {
                return Err(GuestTcxError::MapSchemaMismatch { expected, observed });
            }
            let expected_name: &[u8] = if endpoint { b"ENDPOINTS" } else { b"COUNTERS" };
            return Ok(u32::from(map.name.as_slice() == expected_name));
        }
        let baseline = self.baseline.lock();
        let expected = maps
            .iter()
            .filter(|map| {
                let schema = project_map_schema(map);
                (endpoint && map.name == b"ENDPOINTS" && schema.kind == GuestTcxMapKind::Hash)
                    || (!endpoint
                        && map.name == b"COUNTERS"
                        && schema.kind == GuestTcxMapKind::Array)
            })
            .filter(|map| !baseline.maps.contains_key(&map.id))
            .count();
        if expected > 0 { Err(GuestTcxError::InventoryAmbiguous { family }) } else { Ok(0) }
    }

    fn observe_programs(&self) -> Result<u32, GuestTcxError> {
        let family = GuestTcxInventoryFamily::TcxProgram;
        if !self.baseline.lock().programs_available {
            return Err(GuestTcxError::CaptureUnavailable { family });
        }
        let programs = self
            .source
            .loaded_programs()
            .map_err(|_| GuestTcxError::InventoryAmbiguous { family })?;
        let program_receipt = self.receipts.lock().program_id;
        if let Some(id) = program_receipt {
            let Some(program) = programs.iter().find(|program| program.id == id) else {
                return Ok(0);
            };
            let (endpoint_map_id, counter_map_id) = {
                let expected_maps = self.receipts.lock();
                (expected_maps.endpoint_map_id, expected_maps.counter_map_id)
            };
            let map_ids_match = endpoint_map_id
                .is_none_or(|map_id| program.map_ids.contains(&map_id))
                && counter_map_id.is_none_or(|map_id| program.map_ids.contains(&map_id));
            if !map_ids_match {
                return Err(GuestTcxError::OwnershipMismatch {
                    family: GuestTcxInventoryFamily::TcxProgram,
                });
            }
            return Ok(u32::from(
                program.name.as_slice() == b"gh295c_endpoint"
                    && program.program_type == aya::programs::ProgramType::SchedClassifier,
            ));
        }
        let baseline = self.baseline.lock();
        let candidates =
            programs.iter().filter(|program| !baseline.programs.contains_key(&program.id)).count();
        if candidates > 0 { Err(GuestTcxError::InventoryAmbiguous { family }) } else { Ok(0) }
    }

    fn observe_links(&self) -> Result<u32, GuestTcxError> {
        let family = GuestTcxInventoryFamily::TcxLink;
        if !self.baseline.lock().links_available {
            return Err(GuestTcxError::CaptureUnavailable { family });
        }
        let links =
            self.source.loaded_links().map_err(|_| GuestTcxError::InventoryAmbiguous { family })?;
        let link_receipt = self.receipts.lock().link_id;
        if let Some(id) = link_receipt {
            let Some(link) = links.iter().find(|link| link.id == id) else {
                return Ok(0);
            };
            let (link_program_id, link_target_ifindex, link_attach_type) = {
                let receipts = self.receipts.lock();
                (receipts.link_program_id, receipts.link_target_ifindex, receipts.link_attach_type)
            };
            if link_program_id != Some(link.program_id)
                || link_target_ifindex != Some(link.target_ifindex)
                || link_attach_type != Some(link.attach_type)
            {
                return Err(GuestTcxError::OwnershipMismatch { family });
            }
            return Ok(1);
        }
        let baseline = self.baseline.lock();
        let candidates = links.iter().filter(|link| !baseline.links.contains_key(&link.id)).count();
        if candidates > 0 { Err(GuestTcxError::InventoryAmbiguous { family }) } else { Ok(0) }
    }

    fn observe_pin(&self, endpoint: bool) -> Result<u32, GuestTcxError> {
        let family = if endpoint {
            GuestTcxInventoryFamily::EndpointMapPin
        } else {
            GuestTcxInventoryFamily::CounterMapPin
        };
        let path = if endpoint { &self.endpoint_map_pin } else { &self.counter_map_pin };
        match self.source.observe_pin(path)? {
            RawGuestTcxPinObservation::Absent => Ok(0),
            RawGuestTcxPinObservation::Map(map) => {
                let receipt = if endpoint {
                    self.receipts.lock().endpoint_map_pin_id
                } else {
                    self.receipts.lock().counter_map_pin_id
                };
                let observed = project_map_schema(&map);
                let expected = expected_map_schema(endpoint);
                if observed != expected {
                    return Err(GuestTcxError::MapSchemaMismatch { expected, observed });
                }
                let expected_name: &[u8] = if endpoint { b"ENDPOINTS" } else { b"COUNTERS" };
                match receipt {
                    Some(id) if id == map.id && map.name.as_slice() == expected_name => Ok(1),
                    Some(_) => Err(GuestTcxError::OwnershipMismatch { family }),
                    None => Err(GuestTcxError::InventoryAmbiguous { family }),
                }
            }
            _ => Err(GuestTcxError::InventoryAmbiguous { family }),
        }
    }

    fn observe_link_pin_path(&self, path: &Path) -> Result<u32, GuestTcxError> {
        let family = GuestTcxInventoryFamily::TcxLinkPin;
        match self.source.observe_pin(path)? {
            RawGuestTcxPinObservation::Absent => Ok(0),
            RawGuestTcxPinObservation::Link(link) => {
                let receipts = self.receipts.lock();
                match receipts.link_pin_id {
                    Some(id)
                        if id == link.id
                            && receipts.link_program_id == Some(link.program_id)
                            && receipts.link_target_ifindex == Some(link.target_ifindex)
                            && receipts.link_attach_type == Some(link.attach_type) =>
                    {
                        Ok(1)
                    }
                    Some(_) => Err(GuestTcxError::OwnershipMismatch { family }),
                    None => Err(GuestTcxError::InventoryAmbiguous { family }),
                }
            }
            _ => Err(GuestTcxError::InventoryAmbiguous { family }),
        }
    }
}

/// Query one real interface/attach-point pair.
#[doc(hidden)]
pub fn query_attachment(
    interface: &str,
    attach_point: TcxAttachPoint,
) -> std::result::Result<GuestTcxAttachment, GuestTcxError> {
    let attach = match attach_point {
        TcxAttachPoint::Ingress => TcAttachType::Ingress,
        TcxAttachPoint::Egress => TcAttachType::Egress,
        TcxAttachPoint::Custom(parent) => TcAttachType::Custom(parent),
    };
    let (revision, programs) = SchedClassifier::query_tcx(interface, attach)
        .map_err(|source| GuestTcxError::Program { source })?;
    Ok(GuestTcxAttachment {
        revision,
        program_ids: sorted_program_ids(programs.into_iter().map(|program| program.id()).collect()),
    })
}

/// Detach the exact owned pinned TCX link.
#[doc(hidden)]
pub fn detach_pinned_link(link_pin: impl AsRef<Path>) -> std::result::Result<(), GuestTcxError> {
    let link = aya::programs::links::PinnedLink::from_pin(link_pin.as_ref())
        .map_err(|source| GuestTcxError::Link { source })?;
    let fd = link.unpin().map_err(|source| GuestTcxError::Io { source })?;
    fd.detach().map_err(|source| GuestTcxError::Program { source })
}

/// Report whether one ifindex is present in the pinned endpoint map.
#[doc(hidden)]
pub fn endpoint_present(
    endpoint_map_pin: impl AsRef<Path>,
    ifindex: u32,
) -> std::result::Result<bool, GuestTcxError> {
    let map = HashMap::<_, u32, EndpointAbi>::try_from(Map::HashMap(
        MapData::from_pin(endpoint_map_pin).map_err(|source| GuestTcxError::Map { source })?,
    ))
    .map_err(|source| GuestTcxError::Map { source })?;
    map.get(&ifindex, 0).map(|_| true).or_else(|error| match error {
        aya::maps::MapError::KeyNotFound => Ok(false),
        source => Err(GuestTcxError::Map { source }),
    })
}

/// Remove one ifindex from the pinned endpoint map.
#[doc(hidden)]
pub fn remove_endpoint(
    endpoint_map_pin: impl AsRef<Path>,
    ifindex: u32,
) -> std::result::Result<(), GuestTcxError> {
    let mut map = HashMap::<_, u32, EndpointAbi>::try_from(Map::HashMap(
        MapData::from_pin(endpoint_map_pin).map_err(|source| GuestTcxError::Map { source })?,
    ))
    .map_err(|source| GuestTcxError::Map { source })?;
    map.remove(&ifindex).or_else(|error| match error {
        aya::maps::MapError::KeyNotFound => Ok(()),
        source => Err(GuestTcxError::Map { source }),
    })
}

/// Read one semantic slot from the pinned counter array.
#[doc(hidden)]
pub fn read_counter(
    counter_map_pin: impl AsRef<Path>,
    counter: GuestTcxCounter,
) -> std::result::Result<u64, GuestTcxError> {
    let map = Array::<_, u64>::try_from(Map::Array(
        MapData::from_pin(counter_map_pin).map_err(|source| GuestTcxError::Map { source })?,
    ))
    .map_err(|source| GuestTcxError::Map { source })?;
    map.get(&counter_index(counter), 0).map_err(|source| GuestTcxError::Map { source })
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
            name: b"gh295c_endpoint".to_vec(),
            program_type: aya::programs::ProgramType::SchedClassifier,
            map_ids,
        }
    }

    fn ingress_link(id: u32, program_id: u32) -> RawGuestTcxLinkObservation {
        RawGuestTcxLinkObservation {
            id,
            program_id,
            target_ifindex: 295,
            attach_type: TCX_INGRESS_ATTACH_TYPE,
        }
    }

    /// S-ND295-00 — capture failure preserves observation and first-source truth.
    /// CONTRACT_SHAPE: bounded-change.
    #[test]
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
            link_program_id: Some(297),
            link_target_ifindex: Some(295),
            link_attach_type: Some(TCX_INGRESS_ATTACH_TYPE),
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
    #[ignore = "pending DELIVER step 02-01: typed aya query/open error projection is outside the ten non-waived bodies"]
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
