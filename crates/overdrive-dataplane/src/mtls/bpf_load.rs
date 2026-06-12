//! Shared loader for the embedded `overdrive_bpf.o` on the transparent-mTLS path.
//!
//! The single embedded BPF object carries BOTH the phase-2 `SERVICE_MAP`
//! (`BPF_MAP_TYPE_HASH_OF_MAPS`) AND the mTLS programs/maps. aya 0.13.x's ELF
//! loader CANNOT create a `HASH_OF_MAPS` from the ELF alone (`BPF_MAP_CREATE`
//! rejects — the inner-map prototype is unset; research § A.2 / D.3). So a bare
//! `EbpfLoader::load_file` of the shared object fails with `failed to create map
//! SERVICE_MAP` even when only the mTLS programs are wanted.
//!
//! The fix is the same `pinning = ByName` workaround `EbpfDataplane::new` uses
//! (`.claude/rules/development.md` § "Sharing the outer HoM … `pinning = ByName`"):
//! pre-create + pre-pin `SERVICE_MAP` to a bpffs dir ONCE, then load the ELF with
//! `EbpfLoader::map_pin_path(dir)` so aya finds the pinned outer map by name and
//! reuses it instead of trying to create it. Every subsequent load in the same
//! process (the probe's forward-redirect load, each `establish`'s forward-redirect
//! load, and the test-harness workload's `cgroup_connect4_mtls` load) reuses the
//! one pin.

use std::path::Path;
use std::sync::OnceLock;

use aya::Ebpf;
use overdrive_core::dataplane::MaglevTableSize;

use crate::maps::ServiceKey;
use crate::maps::hash_of_maps::HashOfMapsHandle;

/// The bpffs pin dir for the transparent-mTLS shared-object load. Distinct from
/// `EbpfDataplane`'s production `/sys/fs/bpf/overdrive` so the two loaders do not
/// collide on the same `SERVICE_MAP` pin in a combined process.
pub(super) const MTLS_PIN_DIR: &str = "/sys/fs/bpf/overdrive-mtls";

/// `SERVICE_MAP` outer capacity (mirrors `EbpfDataplane`'s SSOT).
const SERVICE_MAP_OUTER_CAPACITY: u32 = 4096;

/// Keeps the pre-pinned `SERVICE_MAP` handle alive for the process lifetime so the
/// bpffs pin stays valid across every shared-object load. Created once.
static PINNED_SERVICE_MAP: OnceLock<HashOfMapsHandle<ServiceKey, u32>> = OnceLock::new();

/// Errors loading the shared BPF object on the mTLS path.
#[derive(Debug, thiserror::Error)]
pub(super) enum BpfLoadError {
    /// Writing the embedded object to a temp file failed.
    #[error("write embedded BPF object: {0}")]
    Write(#[source] std::io::Error),
    /// Pre-creating / pre-pinning `SERVICE_MAP` failed.
    #[error("pin SERVICE_MAP: {0}")]
    Pin(String),
    /// The `EbpfLoader::load_file` of the shared object failed.
    #[error("load BPF object: {0}")]
    Load(String),
}

/// Ensure `SERVICE_MAP` is pre-created and pinned at [`MTLS_PIN_DIR`] (idempotent
/// for the process). Must run before any `EbpfLoader::map_pin_path(MTLS_PIN_DIR)`
/// load of the shared object.
fn ensure_service_map_pinned() -> Result<(), BpfLoadError> {
    if PINNED_SERVICE_MAP.get().is_some() {
        return Ok(());
    }
    let pin_dir = Path::new(MTLS_PIN_DIR);
    std::fs::create_dir_all(pin_dir).map_err(BpfLoadError::Write)?;
    let pin_path = pin_dir.join("SERVICE_MAP");
    // Clean any stale pin from a prior unclean run (the pin survives process exit).
    let _ = std::fs::remove_file(&pin_path);
    let inner_capacity = MaglevTableSize::DEFAULT.get();
    let handle = HashOfMapsHandle::<ServiceKey, u32>::new_pinned_with_array_inner(
        "SERVICE_MAP",
        SERVICE_MAP_OUTER_CAPACITY,
        inner_capacity,
        pin_dir,
    )
    .map_err(|e| BpfLoadError::Pin(format!("{e}")))?;
    // Race-safe: if another thread won, our handle's pin is redundant; keep the
    // winner. `set` returns Err on a lost race — drop our redundant handle then.
    let _ = PINNED_SERVICE_MAP.set(handle);
    Ok(())
}

/// Load the embedded `overdrive_bpf.o` via the `pinning = ByName` `SERVICE_MAP`
/// workaround. Returns a fresh `Ebpf` instance (its mTLS programs/maps are
/// per-instance; the shared `SERVICE_MAP` is the one pinned outer map).
pub(super) fn load_shared_bpf(bpf_obj: &[u8]) -> Result<Ebpf, BpfLoadError> {
    ensure_service_map_pinned()?;
    let temp = std::env::temp_dir().join(format!(
        "overdrive_mtls_bpf_{}_{}.o",
        std::process::id(),
        next_load_seq(),
    ));
    std::fs::write(&temp, bpf_obj).map_err(BpfLoadError::Write)?;
    let result = aya::EbpfLoader::new()
        .map_pin_path(MTLS_PIN_DIR)
        // Tolerate the HASH_OF_MAPS map type (SERVICE_MAP) the ELF declares — aya
        // 0.13.x has no typed HoM variant, so it lands as `Map::Unsupported` and
        // the pinned outer map (created above) is reused via `map_pin_path`. Same
        // flag `EbpfDataplane::new` sets for the identical reason.
        .allow_unsupported_maps()
        .load_file(&temp)
        .map_err(|e| BpfLoadError::Load(format!("{e}")));
    let _ = std::fs::remove_file(&temp);
    result
}

/// Monotonic per-process load counter so concurrent loads do not collide on the
/// temp object path.
fn next_load_seq() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    SEQ.fetch_add(1, Ordering::Relaxed)
}
