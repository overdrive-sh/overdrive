//! S-ND295-48 / S-ND295-00 Lima real-kernel inventory evidence for
//! D-295-DISTILL-12, extended by D-295-R21 (the egress classifier is a second
//! receipted program).
//!
//! These examples stay separate from the source-local raw projection and
//! capture-failure tables in `guest_tcx.rs`. They own actual enumeration,
//! by-ID observation, exact bpffs paths, and retained-object evidence after
//! loader/adopted-handle release, all through the production dataplane
//! lifecycle (`GuestTcxInventoryIdentity::capture`, `GuestTcxProgram`,
//! `GuestTcxLink`, `GuestTcxAdoptedState`, and D6's free functions).
//!
//! Isolation: every body creates one scratch TAP outside the production
//! `ovd-tp-` namespace and one pin root under `/sys/fs/bpf/overdrive/`
//! (the accepted bpffs hierarchy), both removed on drop. The
//! `overdrive-dataplane` integration binary is `host-kernel-shared`.
//!
//! The fixture (`ScratchTap`, `PinRoot`, `capture`, `establish`, the family
//! observers) is shared with `guest_tcx_egress_lifecycle.rs`.

#![allow(clippy::doc_markdown, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use overdrive_dataplane::DEFAULT_PIN_DIR;
use overdrive_dataplane::guest_tcx::{
    GuestTcxAdoptedState, GuestTcxEndpoint, GuestTcxError, GuestTcxInventoryFamily,
    GuestTcxInventoryIdentity, GuestTcxMapCapacity, GuestTcxMapKeyShape, GuestTcxMapKind,
    GuestTcxMapSchema, GuestTcxMapValueShape, GuestTcxProgram, TcxAttachPoint, detach_pinned_link,
    query_attachment, remove_endpoint,
};

/// The registered guest of every scratch attachment.
pub(super) const GUEST_IPV4: std::net::Ipv4Addr = std::net::Ipv4Addr::new(100, 95, 0, 2);
/// The registered guest MAC (the endpoint record's `source_mac`).
pub(super) const GUEST_MAC: [u8; 6] = [0x02, 0x00, 100, 95, 0, 2];
/// The fixed node bridge MAC (ADR-0126 `GUEST_BRIDGE_MAC`).
pub(super) const BRIDGE_MAC: [u8; 6] = [0x02, 0x01, 0, 0, 0, 1];

/// The eight D12 families in D5's fixed observation order.
const FAMILIES: [GuestTcxInventoryFamily; 8] = [
    GuestTcxInventoryFamily::EndpointMap,
    GuestTcxInventoryFamily::CounterMap,
    GuestTcxInventoryFamily::EndpointEntry,
    GuestTcxInventoryFamily::TcxProgram,
    GuestTcxInventoryFamily::TcxLink,
    GuestTcxInventoryFamily::EndpointMapPin,
    GuestTcxInventoryFamily::CounterMapPin,
    GuestTcxInventoryFamily::TcxLinkPin,
];

/// Nothing observed in any family.
pub(super) const CLEAN: [u32; 8] = [0; 8];

/// One completed D12 lifecycle: both maps, one endpoint entry, the ingress
/// link and the three pins, and BOTH receipted classifier programs — the
/// ingress endpoint classifier and the D-295-R21 egress guest-MAC classifier
/// that `GuestTcxProgram::load` loads from the same object.
pub(super) const RECEIPTED: [u32; 8] = [1, 1, 1, 2, 1, 1, 1, 1];

/// Upper bound for the kernel's deferred reclamation of a released object:
/// a map referenced by a freed program is released after an RCU grace
/// period, so absence is awaited, never assumed.
const SETTLE_BUDGET: Duration = Duration::from_secs(5);
const SETTLE_POLL: Duration = Duration::from_millis(20);

/// A persistent scratch TAP outside the production `ovd-tp-` namespace,
/// deleted on drop. It stays down: nothing egresses it, so the classifier
/// counters stay exact.
pub(super) struct ScratchTap {
    name: String,
    ifindex: u32,
}

impl ScratchTap {
    /// Create `gh295<tag><pid-hex>` (at most 15 bytes).
    pub(super) fn create(tag: char) -> Self {
        let name = format!("gh295{tag}{:x}", std::process::id());
        assert!(name.len() <= 15, "scratch TAP name {name} exceeds IFNAMSIZ");
        assert!(
            !name.starts_with("ovd-tp-"),
            "scratch TAP must stay outside the production namespace"
        );
        // A same-named TAP survives only a SIGKILLed earlier run of this
        // exact process id. Absence is the expected case, so this deletion's
        // outcome is not a precondition; the `tuntap add` below proves the
        // name is free.
        let _stale_delete = Command::new("ip").args(["link", "del", &name]).output();
        run_ip(&["tuntap", "add", "dev", &name, "mode", "tap"]);
        let ifindex = std::fs::read_to_string(format!("/sys/class/net/{name}/ifindex"))
            .unwrap_or_else(|error| panic!("read {name} ifindex: {error}"))
            .trim()
            .parse::<u32>()
            .unwrap_or_else(|error| panic!("parse {name} ifindex: {error}"));
        Self { name, ifindex }
    }

    pub(super) fn name(&self) -> &str {
        &self.name
    }

    pub(super) const fn ifindex(&self) -> u32 {
        self.ifindex
    }
}

impl Drop for ScratchTap {
    fn drop(&mut self) {
        match Command::new("ip").args(["link", "del", &self.name]).output() {
            Ok(output) if output.status.success() => {}
            Ok(output) => report_cleanup_failure(&format!(
                "delete scratch TAP {} (status {:?}): {}",
                self.name,
                output.status.code(),
                String::from_utf8_lossy(&output.stderr).trim()
            )),
            Err(error) => report_cleanup_failure(&format!(
                "spawn ip to delete scratch TAP {}: {error}",
                self.name
            )),
        }
    }
}

fn run_ip(args: &[&str]) {
    let output = Command::new("ip")
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("spawn ip {}: {error}", args.join(" ")));
    assert!(
        output.status.success(),
        "ip {} failed (status {:?}): {}",
        args.join(" "),
        output.status.code(),
        String::from_utf8_lossy(&output.stderr).trim()
    );
}

/// A per-test pin root under the accepted bpffs hierarchy, removed (with
/// every pinned object below it) on drop.
pub(super) struct PinRoot {
    root: PathBuf,
}

impl PinRoot {
    pub(super) fn create(tag: &str) -> Self {
        let root =
            Path::new(DEFAULT_PIN_DIR).join(format!("test-gh295-{tag}-{}", std::process::id()));
        remove_tree(&root)
            .unwrap_or_else(|error| panic!("remove stale pin root {}: {error}", root.display()));
        for directory in [root.join("maps"), root.join("links")] {
            std::fs::create_dir_all(&directory).unwrap_or_else(|error| {
                panic!("create pin directory {}: {error}", directory.display())
            });
        }
        Self { root }
    }

    pub(super) fn endpoint_map(&self) -> PathBuf {
        self.root.join("maps/endpoints")
    }

    pub(super) fn counter_map(&self) -> PathBuf {
        self.root.join("maps/counters")
    }

    /// `links/<tap>-<direction>`, the production link-pin shape.
    pub(super) fn link(&self, tap: &ScratchTap, direction: &str) -> PathBuf {
        self.root.join(format!("links/{}-{direction}", tap.name()))
    }
}

impl Drop for PinRoot {
    fn drop(&mut self) {
        if let Err(error) = remove_tree(&self.root) {
            report_cleanup_failure(&format!("remove pin root {}: {error}", self.root.display()));
        }
    }
}

fn remove_tree(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_dir_all(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

/// A cleanup failure fails the test unless a primary failure is already
/// unwinding, in which case both are preserved: the primary panic, and this
/// report on stderr.
#[allow(clippy::print_stderr, reason = "cleanup evidence must survive an in-flight primary panic")]
fn report_cleanup_failure(message: &str) {
    if std::thread::panicking() {
        eprintln!("cleanup failure while a primary failure unwinds: {message}");
    } else {
        panic!("{message}");
    }
}

/// Capture a complete real inventory identity for the root's planned map
/// pins; a failed capture domain is a precondition failure, not evidence.
pub(super) fn capture(pins: &PinRoot) -> GuestTcxInventoryIdentity {
    let (identity, disposition) =
        GuestTcxInventoryIdentity::capture(pins.endpoint_map(), pins.counter_map()).into_parts();
    disposition.expect("complete real map/program/link baseline capture");
    identity
}

/// The accepted endpoint-map schema.
pub(super) const ENDPOINT_SCHEMA: GuestTcxMapSchema = GuestTcxMapSchema {
    kind: GuestTcxMapKind::Hash,
    key: GuestTcxMapKeyShape::U32,
    value: GuestTcxMapValueShape::EndpointAbi,
    capacity: GuestTcxMapCapacity::EndpointMaximum,
};

/// The accepted counter-array schema.
pub(super) const COUNTER_SCHEMA: GuestTcxMapSchema = GuestTcxMapSchema {
    kind: GuestTcxMapKind::Array,
    key: GuestTcxMapKeyShape::U32,
    value: GuestTcxMapValueShape::CounterU64,
    capacity: GuestTcxMapCapacity::CounterSlots,
};

/// The registered endpoint record for `tap`.
pub(super) const fn endpoint() -> GuestTcxEndpoint {
    GuestTcxEndpoint { source_ipv4: GUEST_IPV4, source_mac: GUEST_MAC, bridge_mac: BRIDGE_MAC }
}

/// The D12 production lifecycle, in D5's order: load, pin and read back
/// both maps, insert and read back the endpoint, attach first-ingress,
/// pin the link, and require the TCX query to report exactly that program.
/// Returns the loader and the ingress classifier's program id.
pub(super) fn establish(
    identity: &GuestTcxInventoryIdentity,
    tap: &ScratchTap,
    pins: &PinRoot,
) -> (GuestTcxProgram, u32) {
    let mut program = GuestTcxProgram::load(identity).expect("load the embedded TCX object");
    assert_eq!(
        program.pin_endpoint_map(&pins.endpoint_map()).expect("pin endpoint map"),
        ENDPOINT_SCHEMA
    );
    assert_eq!(
        program.pin_counter_map(&pins.counter_map()).expect("pin counter map"),
        COUNTER_SCHEMA
    );
    program.insert_endpoint(tap.ifindex(), endpoint()).expect("insert endpoint");
    assert_eq!(program.read_endpoint(tap.ifindex()).expect("read endpoint back"), Some(endpoint()));
    let link = program.attach_first_ingress(tap.name()).expect("attach first-ingress");
    let ingress_program = link.program_id();
    link.pin(&pins.link(tap, "ingress")).expect("pin ingress link");
    assert_eq!(
        query_attachment(tap.name(), TcxAttachPoint::Ingress)
            .expect("query TCX ingress")
            .program_ids,
        vec![ingress_program],
        "exactly the loaded classifier is attached at TCX ingress"
    );
    (program, ingress_program)
}

/// One D12 family observation, by the same eight methods D5 calls.
pub(super) fn observe(
    identity: &GuestTcxInventoryIdentity,
    family: GuestTcxInventoryFamily,
) -> Result<u32, GuestTcxError> {
    match family {
        GuestTcxInventoryFamily::EndpointMap => identity.observe_endpoint_maps(),
        GuestTcxInventoryFamily::CounterMap => identity.observe_counter_maps(),
        GuestTcxInventoryFamily::EndpointEntry => identity.observe_endpoint_entries(),
        GuestTcxInventoryFamily::TcxProgram => identity.observe_tcx_programs(),
        GuestTcxInventoryFamily::TcxLink => identity.observe_tcx_links(),
        GuestTcxInventoryFamily::EndpointMapPin => identity.observe_endpoint_map_pins(),
        GuestTcxInventoryFamily::CounterMapPin => identity.observe_counter_map_pins(),
        GuestTcxInventoryFamily::TcxLinkPin => identity.observe_tcx_link_pins(),
    }
}

/// Await the exact eight-family counts. Every distinct sampled observation is
/// appended to the history (with its first elapsed time and repeat count), and
/// the whole history is the failure message: a transient enumeration error or
/// a not-yet-reclaimed object is retried within the budget, never accepted.
pub(super) fn await_family_counts(
    identity: &GuestTcxInventoryIdentity,
    expected: [u32; 8],
    context: &str,
) {
    let started = Instant::now();
    let mut history: Vec<(Duration, String, u32)> = Vec::new();
    loop {
        let observed: Vec<(GuestTcxInventoryFamily, Result<u32, GuestTcxError>)> =
            FAMILIES.iter().map(|family| (*family, observe(identity, *family))).collect();
        let matched = observed
            .iter()
            .zip(expected)
            .all(|((_, count), want)| matches!(count, Ok(count) if *count == want));
        let rendered = format!("{observed:?}");
        match history.last_mut() {
            Some((_, last, repeats)) if *last == rendered => *repeats += 1,
            _ => history.push((started.elapsed(), rendered, 1)),
        }
        if matched {
            return;
        }
        assert!(
            started.elapsed() < SETTLE_BUDGET,
            "{context}: the eight families never observed {expected:?} (order {FAMILIES:?}) \
             within {SETTLE_BUDGET:?}; observation history:\n{}",
            history
                .iter()
                .map(|(at, observation, repeats)| format!("  +{at:?} (x{repeats}) {observation}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
        std::thread::sleep(SETTLE_POLL);
    }
}

/// A map family whose object a single retained handle can keep alive with
/// no pin, no loader, no link, and no other family.
#[derive(Debug, Clone, Copy)]
enum RetainedMap {
    Endpoint,
    Counter,
}

/// Outcome anchor: OUT-ND295-BORN-CAPTURED.
/// S-ND295-48 — The ownership inventory counts both receipted programs and
/// still reports zero after cleanup (D-295-DISTILL-12 real eight families).
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn clean_and_receipted_inventory_observes_all_eight_exact_families() {
    let tap = ScratchTap::create('i');
    let pins = PinRoot::create("inventory-clean");

    // Clean: a complete real capture with nothing loaded observes exact zero.
    let identity = capture(&pins);
    await_family_counts(&identity, CLEAN, "clean capture before load");

    // Receipted: the full lifecycle; the program family counts both
    // receipted classifiers.
    let (program, _ingress_program) = establish(&identity, &tap, &pins);
    await_family_counts(&identity, RECEIPTED, "receipted lifecycle");

    // Explicit reverse cleanup across both handle-release boundaries, in D5's
    // order: close the loader, adopt, delete the entry, unpin and detach the
    // link, unpin both maps, release the adopted handles.
    drop(program);
    let mut adopted = GuestTcxAdoptedState::for_inventory(&identity, pins.link(&tap, "ingress"));
    assert_eq!(adopted.adopt_endpoint_map().expect("adopt endpoint map"), ENDPOINT_SCHEMA);
    assert_eq!(adopted.adopt_counter_map().expect("adopt counter map"), COUNTER_SCHEMA);
    adopted.adopt_link().expect("adopt the owned ingress link");
    remove_endpoint(pins.endpoint_map(), tap.ifindex()).expect("delete endpoint entry");
    adopted
        .unpin_link()
        .expect("unpin owned link")
        .expect("the owned link was pinned")
        .detach()
        .expect("detach owned link");
    adopted.unpin_counter_map().expect("unpin counter map");
    adopted.unpin_endpoint_map().expect("unpin endpoint map");
    drop(adopted);
    await_family_counts(&identity, CLEAN, "after reverse cleanup and both handle releases");
    assert!(
        query_attachment(tap.name(), TcxAttachPoint::Ingress)
            .expect("query TCX ingress after cleanup")
            .program_ids
            .is_empty(),
        "cleanup leaves nothing attached at TCX ingress"
    );

    // Repeat, deliberately retaining one receipted family: the one handle
    // that keeps exactly one family alive is an adopted, unpinned map. Its
    // family reports its exact positive count while every other family is
    // observed zero; releasing it returns the inventory to zero.
    for retained in [RetainedMap::Endpoint, RetainedMap::Counter] {
        let identity = capture(&pins);
        let (program, _ingress_program) = establish(&identity, &tap, &pins);
        await_family_counts(&identity, RECEIPTED, &format!("receipted lifecycle ({retained:?})"));
        drop(program);
        let mut adopted =
            GuestTcxAdoptedState::for_inventory(&identity, pins.link(&tap, "ingress"));
        match retained {
            RetainedMap::Endpoint => {
                assert_eq!(
                    adopted.adopt_endpoint_map().expect("adopt endpoint map"),
                    ENDPOINT_SCHEMA
                );
            }
            RetainedMap::Counter => {
                assert_eq!(adopted.adopt_counter_map().expect("adopt counter map"), COUNTER_SCHEMA);
            }
        }
        adopted.adopt_link().expect("adopt the owned ingress link");
        remove_endpoint(pins.endpoint_map(), tap.ifindex()).expect("delete endpoint entry");
        adopted
            .unpin_link()
            .expect("unpin owned link")
            .expect("the owned link was pinned")
            .detach()
            .expect("detach owned link");
        adopted.unpin_counter_map().expect("unpin counter map");
        adopted.unpin_endpoint_map().expect("unpin endpoint map");
        let expected = match retained {
            RetainedMap::Endpoint => [1, 0, 0, 0, 0, 0, 0, 0],
            RetainedMap::Counter => [0, 1, 0, 0, 0, 0, 0, 0],
        };
        await_family_counts(&identity, expected, &format!("{retained:?} map retained"));
        drop(adopted);
        await_family_counts(&identity, CLEAN, &format!("{retained:?} map released"));
    }
}

/// Outcome anchor: OUT-ND295-BORN-CAPTURED.
/// S-ND295-48 — Unpinned objects kept alive by one independent kernel
/// reference survive loader and adopted-handle release and stay observable;
/// the unretained egress classifier does not.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn retained_unpinned_maps_programs_and_links_survive_handle_release_and_remain_observable() {
    let tap = ScratchTap::create('r');
    let pins = PinRoot::create("inventory-retained");
    let identity = capture(&pins);
    let (program, ingress_program) = establish(&identity, &tap, &pins);
    await_family_counts(&identity, RECEIPTED, "receipted lifecycle");

    // Release every handle except one independent kernel reference, the
    // unpinned ingress link. Closing the loader releases the unattached egress
    // classifier; the link keeps the ingress classifier, and through it both
    // maps and the endpoint entry, alive.
    drop(program);
    let mut adopted = GuestTcxAdoptedState::for_inventory(&identity, pins.link(&tap, "ingress"));
    assert_eq!(adopted.adopt_endpoint_map().expect("adopt endpoint map"), ENDPOINT_SCHEMA);
    assert_eq!(adopted.adopt_counter_map().expect("adopt counter map"), COUNTER_SCHEMA);
    adopted.adopt_link().expect("adopt the owned ingress link");
    let retained_link =
        adopted.unpin_link().expect("unpin owned link").expect("the owned link was pinned");
    assert_eq!(retained_link.program_id(), ingress_program);
    adopted.unpin_counter_map().expect("unpin counter map");
    adopted.unpin_endpoint_map().expect("unpin endpoint map");
    drop(adopted);

    for pin in [pins.endpoint_map(), pins.counter_map(), pins.link(&tap, "ingress")] {
        assert!(!pin.exists(), "{} is unpinned", pin.display());
    }
    await_family_counts(
        &identity,
        [1, 1, 1, 1, 1, 0, 0, 0],
        "retained unpinned link after loader and adopted-handle release",
    );
    assert_eq!(
        query_attachment(tap.name(), TcxAttachPoint::Ingress)
            .expect("query TCX ingress with the retained link")
            .program_ids,
        vec![ingress_program],
        "the retained unpinned link is still attached"
    );

    retained_link.detach().expect("detach the retained link");
    await_family_counts(&identity, CLEAN, "after the retained link is detached");
    assert!(
        query_attachment(tap.name(), TcxAttachPoint::Ingress)
            .expect("query TCX ingress after detach")
            .program_ids
            .is_empty(),
        "detaching the retained link empties TCX ingress"
    );
}

/// Kernel shape of one BPF map, as read back from a pin.
#[derive(Debug, Clone, Copy)]
struct MapShape {
    map_type: u32,
    key_size: u32,
    value_size: u32,
    max_entries: u32,
    name: [u8; 16],
}

const BPF_MAP_TYPE_HASH: u32 = 1;
const BPF_MAP_TYPE_ARRAY: u32 = 2;

/// The one property a wrong-schema row changes from the accepted shape.
#[derive(Debug, Clone, Copy)]
enum WrongProperty {
    /// A valid map of another accepted kind: projected exactly.
    Kind(GuestTcxMapKind),
    /// Every other wrong property is projected to an opaque value.
    Key,
    Value,
    Capacity,
}

impl MapShape {
    /// Read the exact shape of the map pinned at `pin`.
    fn of_pin(pin: &Path) -> Self {
        let info = aya::maps::MapInfo::from_pin(pin)
            .unwrap_or_else(|error| panic!("read pinned map {}: {error}", pin.display()));
        let mut name = [0_u8; 16];
        let raw = info.name();
        assert!(raw.len() < name.len(), "map name fits BPF_OBJ_NAME_LEN");
        name[..raw.len()].copy_from_slice(raw);
        Self {
            map_type: info.map_type().expect("decode pinned map type") as u32,
            key_size: info.key_size(),
            value_size: info.value_size(),
            max_entries: info.max_entries(),
            name,
        }
    }

    /// Replace whatever is pinned at `pin` with a NEW map of this shape — a
    /// different kernel identity at the exact planned path.
    fn replace_pin(self, pin: &Path) {
        match std::fs::remove_file(pin) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!("unpin {}: {error}", pin.display()),
        }
        create_and_pin_map(self, pin);
    }
}

/// `BPF_MAP_CREATE` then `BPF_OBJ_PIN` through raw `bpf(2)`; the created
/// descriptor is closed once the pin holds the map.
fn create_and_pin_map(shape: MapShape, pin: &Path) {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};

    const BPF_MAP_CREATE: libc::c_long = 0;
    const BPF_OBJ_PIN: libc::c_long = 6;

    #[repr(C)]
    #[derive(Default)]
    struct MapCreateAttr {
        map_type: u32,
        key_size: u32,
        value_size: u32,
        max_entries: u32,
        map_flags: u32,
        inner_map_fd: u32,
        numa_node: u32,
        map_name: [u8; 16],
        map_ifindex: u32,
        btf_fd: u32,
        btf_key_type_id: u32,
        btf_value_type_id: u32,
        tail: [u8; 32],
    }

    #[repr(C)]
    #[derive(Default)]
    struct ObjPinAttr {
        pathname: u64,
        bpf_fd: u32,
        file_flags: u32,
        path_fd: i32,
        tail: [u8; 12],
    }

    let create = MapCreateAttr {
        map_type: shape.map_type,
        key_size: shape.key_size,
        value_size: shape.value_size,
        max_entries: shape.max_entries,
        map_name: shape.name,
        ..MapCreateAttr::default()
    };
    // SAFETY: `create` is a zero-tailed `bpf_attr` prefix for
    // BPF_MAP_CREATE that outlives the call; the size is its exact length.
    let raw = unsafe {
        libc::syscall(
            libc::SYS_bpf,
            BPF_MAP_CREATE,
            std::ptr::from_ref(&create),
            std::mem::size_of::<MapCreateAttr>(),
        )
    };
    assert!(raw >= 0, "BPF_MAP_CREATE {shape:?}: {}", std::io::Error::last_os_error());
    let fd = i32::try_from(raw).expect("a map descriptor fits c_int");
    // SAFETY: a non-negative BPF_MAP_CREATE result is a new descriptor owned
    // by this process and not aliased.
    let map = unsafe { OwnedFd::from_raw_fd(fd) };
    let path = CString::new(pin.as_os_str().as_encoded_bytes()).expect("pin path has no NUL");
    let pin_attr = ObjPinAttr {
        pathname: path.as_ptr() as u64,
        bpf_fd: u32::try_from(map.as_raw_fd()).expect("live descriptor is non-negative"),
        ..ObjPinAttr::default()
    };
    // SAFETY: `pin_attr` is a zero-tailed `bpf_attr` prefix for BPF_OBJ_PIN;
    // `path` and `map` stay live for the duration of the call.
    let pinned = unsafe {
        libc::syscall(
            libc::SYS_bpf,
            BPF_OBJ_PIN,
            std::ptr::from_ref(&pin_attr),
            std::mem::size_of::<ObjPinAttr>(),
        )
    };
    assert!(pinned == 0, "BPF_OBJ_PIN {}: {}", pin.display(), std::io::Error::last_os_error());
}

fn assert_source_less(error: &GuestTcxError) {
    assert!(
        std::error::Error::source(error).is_none(),
        "a semantic inventory refusal carries no fabricated source: {error:?}"
    );
}

/// Outcome anchor: OUT-ND295-BORN-CAPTURED.
/// S-ND295-48 — A wrong owner, an unreceipted candidate, or a valid map of the
/// wrong schema at an exact planned path is a typed, source-less refusal and
/// never an observed zero.
/// CONTRACT_SHAPE: bounded-change.
#[allow(clippy::too_many_lines, reason = "one real-kernel ownership table is audited intact")]
#[test]
fn wrong_exact_path_owner_or_valid_map_schema_is_typed_and_never_fabricates_zero() {
    let tap = ScratchTap::create('w');

    // (a) Unreceipted candidates. The bystander captures a baseline and never
    // loads; another loader then creates one unique, correctly shaped object
    // in every family at the bystander's exact planned paths. Every family
    // with a candidate is ambiguous, never an owned count and never zero.
    let unreceipted = PinRoot::create("inventory-unreceipted");
    let bystander = capture(&unreceipted);
    let intruder_identity = capture(&unreceipted);
    let (intruder, _intruder_program) = establish(&intruder_identity, &tap, &unreceipted);
    drop(GuestTcxAdoptedState::for_inventory(&bystander, unreceipted.link(&tap, "ingress")));
    for family in [
        GuestTcxInventoryFamily::EndpointMap,
        GuestTcxInventoryFamily::CounterMap,
        GuestTcxInventoryFamily::TcxProgram,
        GuestTcxInventoryFamily::TcxLink,
        GuestTcxInventoryFamily::EndpointMapPin,
        GuestTcxInventoryFamily::CounterMapPin,
        GuestTcxInventoryFamily::TcxLinkPin,
    ] {
        let observed = observe(&bystander, family);
        match &observed {
            Err(error @ GuestTcxError::InventoryAmbiguous { family: actual })
                if *actual == family =>
            {
                assert_source_less(error);
            }
            other => panic!("{family:?} with an unreceipted candidate: {other:?}"),
        }
    }
    detach_pinned_link(unreceipted.link(&tap, "ingress")).expect("detach the intruder link");
    drop(intruder);
    drop(unreceipted);

    // (b) A receipted owner whose exact planned paths later hold a
    // different valid object with the accepted schema: OwnershipMismatch.
    let owner_pins = PinRoot::create("inventory-owner");
    let owner = capture(&owner_pins);
    let (owner_program, _owner_ingress) = establish(&owner, &tap, &owner_pins);
    for family in [
        GuestTcxInventoryFamily::EndpointMapPin,
        GuestTcxInventoryFamily::CounterMapPin,
        GuestTcxInventoryFamily::TcxLinkPin,
    ] {
        assert!(
            matches!(observe(&owner, family), Ok(1)),
            "{family:?} observes the owned pin before replacement"
        );
    }
    let endpoint_shape = MapShape::of_pin(&owner_pins.endpoint_map());
    let counter_shape = MapShape::of_pin(&owner_pins.counter_map());
    for (pin, shape, family) in [
        (owner_pins.endpoint_map(), endpoint_shape, GuestTcxInventoryFamily::EndpointMapPin),
        (owner_pins.counter_map(), counter_shape, GuestTcxInventoryFamily::CounterMapPin),
    ] {
        shape.replace_pin(&pin);
        match &observe(&owner, family) {
            Err(error @ GuestTcxError::OwnershipMismatch { family: actual })
                if *actual == family =>
            {
                assert_source_less(error);
            }
            other => panic!("{family:?} holding another valid map: {other:?}"),
        }
    }
    detach_pinned_link(owner_pins.link(&tap, "ingress")).expect("detach the owned link");
    let other_pins = PinRoot::create("inventory-other");
    let other_identity = capture(&other_pins);
    let mut other_program =
        GuestTcxProgram::load(&other_identity).expect("load another TCX object");
    other_program
        .attach_first_ingress(tap.name())
        .expect("attach another classifier first-ingress")
        .pin(&owner_pins.link(&tap, "ingress"))
        .expect("pin another link at the owner's planned path");
    match &observe(&owner, GuestTcxInventoryFamily::TcxLinkPin) {
        Err(
            error
            @ GuestTcxError::OwnershipMismatch { family: GuestTcxInventoryFamily::TcxLinkPin },
        ) => {
            assert_source_less(error);
        }
        other => panic!("TcxLinkPin holding another owner's link: {other:?}"),
    }

    // (c) A valid map of the wrong kind, key, value, or capacity at an exact
    // planned map path: MapSchemaMismatch against the accepted schema, with
    // the wrong property projected to its exact or opaque value.
    let rows = [
        (
            GuestTcxInventoryFamily::EndpointMapPin,
            MapShape { map_type: BPF_MAP_TYPE_ARRAY, ..endpoint_shape },
            WrongProperty::Kind(GuestTcxMapKind::Array),
        ),
        (
            GuestTcxInventoryFamily::EndpointMapPin,
            MapShape { key_size: 8, ..endpoint_shape },
            WrongProperty::Key,
        ),
        (
            GuestTcxInventoryFamily::EndpointMapPin,
            MapShape { value_size: endpoint_shape.value_size + 4, ..endpoint_shape },
            WrongProperty::Value,
        ),
        (
            GuestTcxInventoryFamily::EndpointMapPin,
            MapShape { max_entries: endpoint_shape.max_entries - 1, ..endpoint_shape },
            WrongProperty::Capacity,
        ),
        (
            GuestTcxInventoryFamily::CounterMapPin,
            MapShape { map_type: BPF_MAP_TYPE_HASH, ..counter_shape },
            WrongProperty::Kind(GuestTcxMapKind::Hash),
        ),
        (
            GuestTcxInventoryFamily::CounterMapPin,
            MapShape { value_size: 16, ..counter_shape },
            WrongProperty::Value,
        ),
        (
            GuestTcxInventoryFamily::CounterMapPin,
            MapShape { max_entries: counter_shape.max_entries - 1, ..counter_shape },
            WrongProperty::Capacity,
        ),
        (
            GuestTcxInventoryFamily::CounterMapPin,
            MapShape { max_entries: counter_shape.max_entries + 1, ..counter_shape },
            WrongProperty::Capacity,
        ),
    ];
    let mut opaque_counter_capacities = Vec::new();
    for (family, shape, wrong) in rows {
        let (pin, accepted) = if family == GuestTcxInventoryFamily::EndpointMapPin {
            (owner_pins.endpoint_map(), ENDPOINT_SCHEMA)
        } else {
            (owner_pins.counter_map(), COUNTER_SCHEMA)
        };
        shape.replace_pin(&pin);
        let observed = observe(&owner, family);
        let Err(error @ GuestTcxError::MapSchemaMismatch { expected, observed: schema }) =
            &observed
        else {
            panic!("{family:?} holding {shape:?}: {observed:?}");
        };
        assert_source_less(error);
        assert_eq!(*expected, accepted, "{family:?} reports the accepted schema for {shape:?}");
        // Exactly the wrong property differs: a wrong kind is exact, every
        // other wrong property is an opaque value that renders no raw number.
        let (differing, rendered) = match wrong {
            WrongProperty::Kind(kind) => {
                assert_eq!(*schema, GuestTcxMapSchema { kind, ..accepted });
                continue;
            }
            WrongProperty::Key => {
                assert!(matches!(schema.key, GuestTcxMapKeyShape::Unsupported(_)));
                (GuestTcxMapSchema { key: schema.key, ..accepted }, format!("{:?}", schema.key))
            }
            WrongProperty::Value => {
                assert!(matches!(schema.value, GuestTcxMapValueShape::Unsupported(_)));
                (
                    GuestTcxMapSchema { value: schema.value, ..accepted },
                    format!("{:?}", schema.value),
                )
            }
            WrongProperty::Capacity => {
                assert!(matches!(schema.capacity, GuestTcxMapCapacity::Unsupported(_)));
                if family == GuestTcxInventoryFamily::CounterMapPin {
                    opaque_counter_capacities.push(schema.capacity);
                }
                (
                    GuestTcxMapSchema { capacity: schema.capacity, ..accepted },
                    format!("{:?}", schema.capacity),
                )
            }
        };
        assert_eq!(*schema, differing, "{family:?} changes only {wrong:?} for {shape:?}");
        assert!(
            !rendered.chars().any(|character| character.is_ascii_digit()),
            "{family:?} opaque {wrong:?} renders no raw number: {rendered}"
        );
    }
    assert_eq!(opaque_counter_capacities.len(), 2, "both wrong counter capacities are opaque");
    assert_ne!(
        opaque_counter_capacities[0], opaque_counter_capacities[1],
        "distinct wrong capacities stay distinguishable through the opaque value"
    );

    drop(other_program);
    drop(owner_program);
}
