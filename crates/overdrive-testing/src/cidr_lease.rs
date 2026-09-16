#![allow(clippy::doc_markdown, clippy::print_stderr)]
#![cfg(target_os = "linux")]
//! Cross-process CIDR leases for hand-built Linux integration topologies.
//!
//! The production workload network uses `10.99.0.0/16`. Hand-built worker
//! mTLS fixtures use this module's separate `10.250.0.0/16` pool instead of
//! embedding a route in each test source file. A lease is one `/24` from that
//! pool; its first two usable addresses are exposed as the host gateway and
//! workload address used by the veth fixtures.
//!
//! Allocation is protected by a process-shared `flock(2)` around a durable
//! registry in `/tmp`. Each record contains the caller's name, PID, Linux
//! process start tick, and boot ID. A normal [`TestCidrLease`] drop removes its
//! record. If a test is killed before `Drop`, a later allocator retains the
//! record while its owner is still live or while `ip route` reports a route
//! overlapping the lease. This is intentionally conservative: a stale route
//! must be cleaned by the fixture or operator before that CIDR can be reused.
//!
//! The lease lock is deliberately separate from the worker tests' shared
//! `overdrive-mtls` kernel-state lock. The former protects CIDR ownership; the
//! latter serializes mutation of node-global nftables and policy routing.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read as _, Seek as _, SeekFrom, Write as _};
use std::net::Ipv4Addr;
use std::os::fd::AsRawFd as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use thiserror::Error;

/// The production workload subnet, repeated here only to make the test-pool
/// disjointness contract visible at the support boundary.
pub const PRODUCTION_WORKLOAD_SUBNET: &str = "10.99.0.0/16";
/// The host-only pool from which hand-built test topologies receive `/24`s.
pub const TEST_CIDR_POOL: &str = "10.250.0.0/16";
/// Prefix assigned to one hand-built test topology.
pub const TEST_CIDR_PREFIX: u8 = 24;

const TEST_POOL_OCTET_PREFIX: u8 = 10;
const TEST_POOL_OCTET_SECOND: u8 = 250;
const TEST_POOL_BLOCK_COUNT: u16 = 256;
const LEASE_STATE_PATH: &str = "/tmp/overdrive-test-cidr-leases.v1";
const STATE_VERSION: &str = "v1";

/// Result alias for the test CIDR lease support.
pub type Result<T, E = CidrLeaseError> = std::result::Result<T, E>;

/// A named, process-owned `/24` reserved for a hand-built test topology.
///
/// The lease does not create a route or network namespace. The fixture that
/// owns it remains responsible for provisioning and tearing down those kernel
/// objects before the lease is dropped.
#[derive(Debug)]
pub struct TestCidrLease {
    name: String,
    cidr: String,
    network: Ipv4Addr,
    host_gateway: Ipv4Addr,
    workload_addr: Ipv4Addr,
    record: LeaseRecord,
    state_path: PathBuf,
}

impl TestCidrLease {
    /// Acquire a named CIDR from the global test pool.
    pub fn acquire(owner: &str) -> Result<Self> {
        let state_path = PathBuf::from(LEASE_STATE_PATH);
        validate_name(owner)?;
        let identity = current_process_identity()
            .map_err(|source| CidrLeaseError::CurrentProcessIdentity { source })?;
        let mut registry = open_locked_registry(&state_path)?;
        // Keep the route observation inside the same allocator critical
        // section as the registry read and claim. The kernel can still change
        // independently, but no second allocator can win between our route
        // check and durable ownership record.
        let routes = live_kernel_routes()?;
        acquire_locked(owner, &state_path, &routes, identity, &mut registry)
    }

    /// The name supplied by the test owner and persisted in the registry.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The leased network in CIDR notation, for example `10.250.7.0/24`.
    #[must_use]
    pub fn cidr(&self) -> &str {
        &self.cidr
    }

    /// The address assigned to the host-side veth.
    #[must_use]
    pub const fn host_gateway(&self) -> Ipv4Addr {
        self.host_gateway
    }

    /// The address assigned to the workload-side veth.
    #[must_use]
    pub const fn workload_addr(&self) -> Ipv4Addr {
        self.workload_addr
    }

    /// The leased network address.
    #[must_use]
    pub const fn network(&self) -> Ipv4Addr {
        self.network
    }

    /// The leased prefix length (`24`).
    #[must_use]
    pub const fn prefix_len(&self) -> u8 {
        self.record.cidr.prefix
    }
}

impl Drop for TestCidrLease {
    fn drop(&mut self) {
        // A destructor cannot report an I/O failure. If the state file cannot
        // be locked or rewritten, leaving the record behind is the safe result:
        // the next allocation will inspect the owner identity and live routes
        // before deciding whether it can reclaim the CIDR.
        if let Err(error) = release_record(&self.state_path, &self.record) {
            eprintln!(
                "overdrive test CIDR lease `{}` release retained conservatively: {error}",
                self.name
            );
        }
    }
}

/// Failures surfaced by the global test CIDR allocator.
#[derive(Debug, Error)]
pub enum CidrLeaseError {
    /// The caller supplied an empty name.
    #[error("test CIDR lease name must not be empty")]
    EmptyName,
    /// The process identity required for ownership records could not be read.
    #[error("could not read the current Linux process identity: {source}")]
    CurrentProcessIdentity {
        #[source]
        source: io::Error,
    },
    /// The registry file could not be opened or created.
    #[error("could not open test CIDR lease registry at {path}: {source}")]
    RegistryOpen {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    /// The registry file could not be exclusively locked.
    #[error("could not lock test CIDR lease registry at {path}: {source}")]
    RegistryLock {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    /// The registry bytes could not be read.
    #[error("could not read test CIDR lease registry at {path}: {source}")]
    RegistryRead {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    /// The registry contained a record that could not be understood safely.
    #[error("test CIDR lease registry at {path} has malformed line {line}: {detail}")]
    RegistryCorrupt { path: PathBuf, line: usize, detail: String },
    /// The registry bytes could not be atomically rewritten while holding the
    /// lock.
    #[error("could not write test CIDR lease registry at {path}: {source}")]
    RegistryWrite {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    /// The route probe could not be spawned.
    #[error("could not run `ip -4 route show table all` to inspect live routes: {source}")]
    RouteProbeSpawn {
        #[source]
        source: io::Error,
    },
    /// The route probe ran but did not return a usable result.
    #[error("`ip -4 route show table all` failed (status={status:?}): {stderr}")]
    RouteProbeFailed { status: Option<i32>, stderr: String },
    /// Every `/24` in the disjoint test pool is owned or conflicts with a
    /// route currently visible to the host kernel.
    #[error("test CIDR pool {TEST_CIDR_POOL} is exhausted or route-conflicted")]
    PoolExhausted,
    /// A live owner already holds the requested name.
    #[error("test CIDR lease `{name}` is already held for {cidr}")]
    NameAlreadyHeld { name: String, cidr: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LeaseRecord {
    name: String,
    cidr: Ipv4Cidr,
    identity: ProcessIdentity,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ProcessIdentity {
    pid: u32,
    start_ticks: u64,
    boot_id: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Ipv4Cidr {
    network: u32,
    prefix: u8,
}

impl Ipv4Cidr {
    const fn new(network: u32, prefix: u8) -> Self {
        Self { network: network & mask(prefix), prefix }
    }

    const fn contains(self, address: u32) -> bool {
        address & mask(self.prefix) == self.network
    }

    const fn overlaps(self, other: Self) -> bool {
        self.contains(other.network) || other.contains(self.network)
    }

    fn as_string(self) -> String {
        format!("{}/{}", Ipv4Addr::from(self.network), self.prefix)
    }
}

const fn mask(prefix: u8) -> u32 {
    match prefix {
        0 => 0,
        32 => u32::MAX,
        bits => u32::MAX << (32 - bits),
    }
}

fn parse_cidr(value: &str) -> Option<Ipv4Cidr> {
    let (address, prefix) = value.split_once('/').unwrap_or((value, "32"));
    let address = address.parse::<Ipv4Addr>().ok()?;
    let prefix = prefix.parse::<u8>().ok()?;
    if prefix > 32 {
        return None;
    }
    Some(Ipv4Cidr::new(u32::from(address), prefix))
}

fn pool_candidate(index: u16) -> Ipv4Cidr {
    debug_assert!(index < TEST_POOL_BLOCK_COUNT);
    let third_octet = u8::try_from(index)
        .unwrap_or_else(|_| unreachable!("pool index is bounded to one IPv4 octet"));
    Ipv4Cidr::new(
        u32::from(Ipv4Addr::new(TEST_POOL_OCTET_PREFIX, TEST_POOL_OCTET_SECOND, third_octet, 0)),
        TEST_CIDR_PREFIX,
    )
}

#[cfg(test)]
fn acquire_with_routes(
    name: &str,
    state_path: &Path,
    routes: &[Ipv4Cidr],
) -> Result<TestCidrLease> {
    validate_name(name)?;
    let identity = current_process_identity()
        .map_err(|source| CidrLeaseError::CurrentProcessIdentity { source })?;
    let mut registry = open_locked_registry(state_path)?;
    acquire_locked(name, state_path, routes, identity, &mut registry)
}

const fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(CidrLeaseError::EmptyName);
    }
    Ok(())
}

fn acquire_locked(
    name: &str,
    state_path: &Path,
    routes: &[Ipv4Cidr],
    identity: ProcessIdentity,
    registry: &mut File,
) -> Result<TestCidrLease> {
    let records = read_records(registry, state_path)?;

    let mut active = Vec::with_capacity(records.len());
    let mut live_owner_names = BTreeSet::new();
    let mut changed = false;
    for record in records {
        let owner_live = owner_is_live(&record.identity);
        let route_live = routes.iter().any(|route| route.overlaps(record.cidr));
        if owner_live || route_live {
            if owner_live {
                live_owner_names.insert(record.name.clone());
            }
            active.push(record);
        } else {
            // A dead owner and no overlapping route is the only reclaimable
            // state. This is the abnormal-exit recovery path.
            changed = true;
        }
    }

    if live_owner_names.contains(name) {
        let existing = active
            .iter()
            .find(|record| record.name == name)
            .unwrap_or_else(|| unreachable!("every live owner name has an active record"));
        if changed {
            write_records(registry, state_path, &active)?;
        }
        return Err(CidrLeaseError::NameAlreadyHeld {
            name: name.to_owned(),
            cidr: existing.cidr.as_string(),
        });
    }

    let Some(cidr) = (0..TEST_POOL_BLOCK_COUNT).map(pool_candidate).find(|candidate| {
        !active.iter().any(|record| record.cidr.overlaps(*candidate))
            && !routes.iter().any(|route| route.overlaps(*candidate))
    }) else {
        if changed {
            write_records(registry, state_path, &active)?;
        }
        return Err(CidrLeaseError::PoolExhausted);
    };

    let record = LeaseRecord { name: name.to_owned(), cidr, identity };
    active.push(record.clone());
    // `write_records` runs while `registry` still holds the flock, so no
    // process can observe the candidate between the check and the claim.
    write_records(registry, state_path, &active)?;

    let network = Ipv4Addr::from(cidr.network);
    Ok(TestCidrLease {
        name: name.to_owned(),
        cidr: cidr.as_string(),
        network,
        host_gateway: Ipv4Addr::from(cidr.network + 1),
        workload_addr: Ipv4Addr::from(cidr.network + 2),
        record,
        state_path: state_path.to_owned(),
    })
}

fn open_locked_registry(path: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(path)
        .map_err(|source| CidrLeaseError::RegistryOpen { path: path.to_owned(), source })?;
    lock_exclusive(&file)
        .map_err(|source| CidrLeaseError::RegistryLock { path: path.to_owned(), source })?;
    Ok(file)
}

fn lock_exclusive(file: &File) -> io::Result<()> {
    loop {
        // SAFETY: `file` owns a valid open file descriptor for the duration of
        // this call. `flock` does not retain any borrowed pointer.
        let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
        if result == 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

fn read_records(file: &mut File, path: &Path) -> Result<Vec<LeaseRecord>> {
    file.seek(SeekFrom::Start(0))
        .map_err(|source| CidrLeaseError::RegistryRead { path: path.to_owned(), source })?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)
        .map_err(|source| CidrLeaseError::RegistryRead { path: path.to_owned(), source })?;
    contents
        .lines()
        .enumerate()
        .map(|(line, value)| {
            parse_record(value).map_err(|detail| CidrLeaseError::RegistryCorrupt {
                path: path.to_owned(),
                line: line + 1,
                detail,
            })
        })
        .collect()
}

fn parse_record(value: &str) -> std::result::Result<LeaseRecord, String> {
    let mut fields = BTreeMap::new();
    for field in value.split('\t') {
        let (key, value) = field
            .split_once('=')
            .ok_or_else(|| "registry field is missing its key/value separator".to_owned())?;
        if fields.insert(key, value).is_some() {
            return Err(format!("duplicate registry field `{key}`"));
        }
    }
    if fields.len() != 6 {
        return Err(format!("expected six registry fields, found {}", fields.len()));
    }
    if fields.get("version") != Some(&STATE_VERSION) {
        return Err("unsupported or missing version".to_owned());
    }
    let name =
        fields.get("owner").ok_or_else(|| "missing owner".to_owned()).and_then(|encoded| {
            decode_hex(encoded).ok_or_else(|| "owner is not valid UTF-8 hex".to_owned())
        })?;
    let cidr = fields
        .get("cidr")
        .and_then(|value| parse_cidr(value))
        .ok_or_else(|| "cidr is not a valid IPv4 CIDR".to_owned())?;
    if !is_pool_candidate(cidr) {
        return Err("cidr is outside the test pool or has the wrong prefix".to_owned());
    }
    let pid = fields
        .get("pid")
        .ok_or_else(|| "missing pid".to_owned())?
        .parse::<u32>()
        .map_err(|_| "pid is not a u32".to_owned())?;
    let start_ticks = fields
        .get("start_ticks")
        .ok_or_else(|| "missing start_ticks".to_owned())?
        .parse::<u64>()
        .map_err(|_| "start_ticks is not a u64".to_owned())?;
    let boot_id =
        fields.get("boot_id").ok_or_else(|| "missing boot_id".to_owned()).and_then(|encoded| {
            decode_hex(encoded).ok_or_else(|| "boot_id is not valid UTF-8 hex".to_owned())
        })?;
    if boot_id.is_empty() {
        return Err("boot_id is empty".to_owned());
    }
    Ok(LeaseRecord { name, cidr, identity: ProcessIdentity { pid, start_ticks, boot_id } })
}

fn is_pool_candidate(cidr: Ipv4Cidr) -> bool {
    cidr.prefix == TEST_CIDR_PREFIX
        && cidr.network & mask(16)
            == u32::from(Ipv4Addr::new(TEST_POOL_OCTET_PREFIX, TEST_POOL_OCTET_SECOND, 0, 0))
}

fn write_records(file: &mut File, path: &Path, records: &[LeaseRecord]) -> Result<()> {
    file.seek(SeekFrom::Start(0))
        .map_err(|source| CidrLeaseError::RegistryWrite { path: path.to_owned(), source })?;
    file.set_len(0)
        .map_err(|source| CidrLeaseError::RegistryWrite { path: path.to_owned(), source })?;
    for record in records {
        let line = format_record(record);
        file.write_all(line.as_bytes())
            .map_err(|source| CidrLeaseError::RegistryWrite { path: path.to_owned(), source })?;
    }
    file.sync_data()
        .map_err(|source| CidrLeaseError::RegistryWrite { path: path.to_owned(), source })
}

fn format_record(record: &LeaseRecord) -> String {
    format!(
        "version={STATE_VERSION}\towner={}\tcidr={}\tpid={}\tstart_ticks={}\tboot_id={}\n",
        encode_hex(record.name.as_bytes()),
        record.cidr.as_string(),
        record.identity.pid,
        record.identity.start_ticks,
        encode_hex(record.identity.boot_id.as_bytes()),
    )
}

fn release_record(path: &Path, target: &LeaseRecord) -> Result<()> {
    let mut registry = open_locked_registry(path)?;
    let records = read_records(&mut registry, path)?;
    let mut removed = false;
    let retained: Vec<_> = records
        .into_iter()
        .filter(|record| {
            if record == target {
                removed = true;
                false
            } else {
                true
            }
        })
        .collect();
    if removed {
        // The compare-and-remove happened while the registry flock was held;
        // no second unlocked membership check can race this release.
        write_records(&mut registry, path, &retained)?;
    }
    Ok(())
}

fn current_process_identity() -> io::Result<ProcessIdentity> {
    let pid = std::process::id();
    let start_ticks = process_start_ticks(pid)?;
    let boot_id = fs::read_to_string("/proc/sys/kernel/random/boot_id")?.trim().to_owned();
    if boot_id.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "kernel boot_id is empty"));
    }
    Ok(ProcessIdentity { pid, start_ticks, boot_id })
}

fn process_start_ticks(pid: u32) -> io::Result<u64> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat"))?;
    let after_comm = stat.rsplit_once(')').map(|(_, rest)| rest).ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "process stat has no comm terminator")
    })?;
    after_comm
        .split_whitespace()
        .nth(19)
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "process stat has no start time")
        })?
        .parse::<u64>()
        .map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidData, "process start time is not numeric")
        })
}

fn owner_is_live(identity: &ProcessIdentity) -> bool {
    let Some(boot_id) = current_boot_id() else {
        // An unreadable boot ID makes PID reuse impossible to distinguish;
        // retain the record until a later pass can make that decision.
        return true;
    };
    if identity.boot_id != boot_id {
        return false;
    }
    match process_start_ticks(identity.pid) {
        Ok(start_ticks) => start_ticks == identity.start_ticks,
        // An unreadable owner identity is retained. Reusing its CIDR would be
        // less safe than waiting for a later allocator pass.
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(_) => true,
    }
}

fn current_boot_id() -> Option<String> {
    match fs::read_to_string("/proc/sys/kernel/random/boot_id") {
        Ok(value) if !value.trim().is_empty() => Some(value.trim().to_owned()),
        Ok(_) | Err(_) => None,
    }
}

fn live_kernel_routes() -> Result<Vec<Ipv4Cidr>> {
    let output = Command::new("ip")
        .args(["-4", "route", "show", "table", "all"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|source| CidrLeaseError::RouteProbeSpawn { source })?;
    if !output.status.success() {
        return Err(CidrLeaseError::RouteProbeFailed {
            status: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(parse_route_destination)
        .collect())
}

fn parse_route_destination(line: &str) -> Option<Ipv4Cidr> {
    let mut fields = line.split_whitespace();
    let first = fields.next()?;
    if first == "default" {
        return None;
    }
    // `ip route show table all` prints route kinds (`local`, `broadcast`,
    // `unreachable`, …) before their destination. Ordinary unicast lines put
    // the destination first.
    let destination = if is_route_kind(first) { fields.next()? } else { first };
    parse_cidr(destination)
}

fn is_route_kind(value: &str) -> bool {
    matches!(
        value,
        "unicast"
            | "local"
            | "broadcast"
            | "unreachable"
            | "prohibit"
            | "blackhole"
            | "throw"
            | "nat"
    )
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn decode_hex(value: &str) -> Option<String> {
    if !value.len().is_multiple_of(2) {
        return None;
    }
    let mut bytes = Vec::with_capacity(value.len() / 2);
    let mut chars = value.bytes();
    while let (Some(high), Some(low)) = (chars.next(), chars.next()) {
        let high = hex_value(high)?;
        let low = hex_value(low)?;
        bytes.push((high << 4) | low);
    }
    String::from_utf8(bytes).ok()
}

const fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST_ID: AtomicU64 = AtomicU64::new(0);

    fn test_state_path() -> PathBuf {
        let id = NEXT_TEST_ID.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("overdrive-test-cidr-{id}-{}.state", std::process::id()))
    }

    #[test]
    fn pool_candidates_are_disjoint_from_production_and_each_other() {
        let production = parse_cidr(PRODUCTION_WORKLOAD_SUBNET).unwrap_or_else(|| unreachable!());
        let first = pool_candidate(0);
        let last = pool_candidate(TEST_POOL_BLOCK_COUNT - 1);
        assert!(!production.overlaps(first));
        assert!(!production.overlaps(last));
        assert!(!first.overlaps(last));
        assert_eq!(first.as_string(), "10.250.0.0/24");
        assert_eq!(last.as_string(), "10.250.255.0/24");
    }

    #[test]
    fn allocation_is_atomic_and_release_returns_a_free_cidr() {
        let path = test_state_path();
        let first =
            acquire_with_routes("first", &path, &[]).unwrap_or_else(|error| panic!("{error}"));
        let second =
            acquire_with_routes("second", &path, &[]).unwrap_or_else(|error| panic!("{error}"));
        assert_ne!(first.cidr(), second.cidr());
        let registry = fs::read_to_string(&path).unwrap_or_else(|error| panic!("{error}"));
        assert!(registry.contains("owner="));
        assert!(registry.contains("pid="));
        assert!(registry.contains("start_ticks="));
        assert!(registry.contains("boot_id="));
        assert_eq!(first.host_gateway().octets()[3], 1);
        assert_eq!(first.workload_addr().octets()[3], 2);
        let first_cidr = first.cidr().to_owned();
        drop(first);
        drop(second);
        let replacement = acquire_with_routes("replacement", &path, &[])
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(replacement.cidr(), first_cidr);
        drop(replacement);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn route_residue_blocks_reuse_after_owner_is_dead() {
        let path = test_state_path();
        let dead = LeaseRecord {
            name: "aborted".to_owned(),
            cidr: pool_candidate(0),
            identity: ProcessIdentity {
                pid: u32::MAX,
                start_ticks: 0,
                boot_id: "different-boot".to_owned(),
            },
        };
        let mut file = open_locked_registry(&path).unwrap_or_else(|error| panic!("{error}"));
        write_records(&mut file, &path, &[dead]).unwrap_or_else(|error| panic!("{error}"));
        drop(file);

        let lease = acquire_with_routes("new-owner", &path, &[pool_candidate(0)])
            .unwrap_or_else(|error| panic!("{error}"));
        assert_ne!(lease.cidr(), "10.250.0.0/24");
        drop(lease);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn stale_route_record_does_not_block_a_new_lease_with_the_same_name() {
        let path = test_state_path();
        let dead = LeaseRecord {
            name: "rerun".to_owned(),
            cidr: pool_candidate(0),
            identity: ProcessIdentity {
                pid: u32::MAX,
                start_ticks: 0,
                boot_id: "different-boot".to_owned(),
            },
        };
        let mut file = open_locked_registry(&path).unwrap_or_else(|error| panic!("{error}"));
        write_records(&mut file, &path, &[dead]).unwrap_or_else(|error| panic!("{error}"));
        drop(file);

        let lease = acquire_with_routes("rerun", &path, &[pool_candidate(0)])
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(lease.name(), "rerun");
        assert_ne!(lease.cidr(), "10.250.0.0/24");
        drop(lease);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn route_parser_ignores_default_and_reads_typed_routes() {
        assert_eq!(parse_route_destination("default via 10.250.0.1 dev eth0"), None);
        assert_eq!(
            parse_route_destination("10.250.4.0/24 dev vethH proto kernel scope link"),
            Some(pool_candidate(4))
        );
        assert_eq!(
            parse_route_destination("local 10.250.4.2 dev lo table local"),
            Some(parse_cidr("10.250.4.2/32").unwrap_or_else(|| unreachable!()))
        );
    }
}
