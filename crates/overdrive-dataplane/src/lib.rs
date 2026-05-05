//! `overdrive-dataplane` — userspace BPF loader per ADR-0038.
//!
//! Owns [`EbpfDataplane`], the production binding of the
//! [`Dataplane`] port trait from `overdrive-core`. The kernel-side
//! object produced by `overdrive-bpf` is embedded at compile time via
//! `include_bytes!`; on Linux the loader attaches the `xdp_pass`
//! program to the configured interface. On non-Linux build targets
//! (developer macOS, primarily) the constructor returns
//! [`DataplaneError::LoadFailed`] with a `"non-Linux build target"`
//! diagnostic — the rest of the workspace still compiles.
//!
//! Phase 2.1 step 01-02 ships the loader skeleton. The three trait
//! methods (`update_policy`, `update_service`, `drain_flow_events`)
//! are stubbed pending #24 (`POLICY_MAP`), #25 (`SERVICE_MAP`), and
//! #27 (telemetry ringbuf) per `architecture.md` §7.

// Phase 2.2 module scaffolds per
// `docs/feature/phase-2-xdp-service-map/distill/wave-decisions.md`
// DWD-3 file-path inventory. Bodies panic via `todo!()` until
// DELIVER fills them per the carpaccio slice plan.
pub mod loader;
pub mod maglev;
pub mod maps;
pub mod swap;

use std::net::Ipv4Addr;

use async_trait::async_trait;
use overdrive_core::traits::dataplane::{
    Backend, Dataplane, DataplaneError, FlowEvent, PolicyKey, Verdict,
};

/// Embedded kernel-side BPF object. Produced by
/// `cargo xtask bpf-build` (step 02-01) and copied to the stable path
/// `target/xtask/bpf-objects/overdrive_bpf.o`. The `build.rs` shim
/// (step 01-03) converts a missing artifact into a single-line
/// actionable error.
///
/// Lives behind `#[cfg(target_os = "linux")]` so non-Linux builds do
/// not need the artifact present at compile time — the
/// `cfg(not(target_os = "linux"))` `new()` returns an error before
/// any aya code runs.
#[cfg(target_os = "linux")]
const OVERDRIVE_BPF_OBJ: &[u8] = include_bytes!(concat!(
    env!("CARGO_WORKSPACE_DIR"),
    "/target/xtask/bpf-objects/overdrive_bpf.o",
));

/// Production dataplane — loads `overdrive_bpf.o` and attaches its
/// `xdp_pass` program to the configured interface.
pub struct EbpfDataplane {
    /// Owns the loaded BPF maps and programs. Dropping this releases
    /// kernel-side resources. Field is kept live so the BPF object's
    /// maps/programs survive across `Dataplane` trait calls; the
    /// stubbed trait methods do not yet read it (deferred to #24/#25/
    /// #27 per architecture.md §7).
    #[cfg(target_os = "linux")]
    #[allow(dead_code)]
    bpf: aya::Ebpf,

    /// Owns the XDP attachment. Dropping detaches `xdp_pass`. Read
    /// only via Drop.
    #[cfg(target_os = "linux")]
    #[allow(dead_code)]
    _link: aya::programs::xdp::XdpLinkId,
}

impl EbpfDataplane {
    /// Construct an `EbpfDataplane` by loading `OVERDRIVE_BPF_OBJ` and
    /// attaching `xdp_pass` to `iface`. Mirrors the `SimDataplane::new`
    /// seam in `overdrive-sim` so production / sim wirings are
    /// substitutable behind the `Dataplane` port trait.
    ///
    /// Interface name is resolved via `nix::net::if_::if_nametoindex`
    /// before any BPF program is loaded; missing interfaces produce
    /// [`DataplaneError::IfaceNotFound`] (S-2.2-03) rather than a
    /// generic `LoadFailed`. Other errno values from `if_nametoindex`
    /// pass through as `LoadFailed` with the originating errno text —
    /// per `.claude/rules/development.md` § Errors, distinct failure
    /// modes get distinct variants; only `ENODEV` / `ENOENT` map to
    /// `IfaceNotFound`.
    #[cfg(target_os = "linux")]
    pub fn new(iface: &str) -> Result<Self, DataplaneError> {
        use aya::programs::{Xdp, XdpFlags};
        use nix::errno::Errno;
        use nix::net::if_::if_nametoindex;

        // Resolve iface name → ifindex first. ENODEV / ENOENT map to
        // the typed IfaceNotFound variant; everything else surfaces
        // as LoadFailed with the errno text.
        if_nametoindex(iface).map_err(|errno| match errno {
            Errno::ENODEV | Errno::ENOENT => {
                DataplaneError::IfaceNotFound { iface: iface.to_string() }
            }
            other => DataplaneError::LoadFailed(format!("if_nametoindex({iface}): {other}")),
        })?;

        let mut bpf = aya::Ebpf::load(OVERDRIVE_BPF_OBJ)
            .map_err(|e| DataplaneError::LoadFailed(format!("aya load: {e}")))?;
        let prog: &mut Xdp = bpf
            .program_mut("xdp_pass")
            .ok_or_else(|| {
                DataplaneError::LoadFailed("xdp_pass program not found in BPF object".into())
            })?
            .try_into()
            .map_err(|e| DataplaneError::LoadFailed(format!("xdp program type: {e}")))?;
        prog.load().map_err(|e| DataplaneError::LoadFailed(format!("xdp_pass.load: {e}")))?;
        let link = prog
            .attach(iface, XdpFlags::default())
            .map_err(|e| DataplaneError::LoadFailed(format!("xdp_pass.attach({iface}): {e}")))?;
        Ok(Self { bpf, _link: link })
    }

    /// Non-Linux fallthrough — returns
    /// [`DataplaneError::LoadFailed`] with a `"non-Linux build
    /// target"` diagnostic. Lets the rest of the workspace compile on
    /// macOS without aya in the dep graph (architecture.md §5.2).
    #[cfg(not(target_os = "linux"))]
    pub fn new(_iface: &str) -> Result<Self, DataplaneError> {
        Err(DataplaneError::LoadFailed("overdrive-dataplane: non-Linux build target".into()))
    }
}

#[async_trait]
impl Dataplane for EbpfDataplane {
    /// see #24 (`POLICY_MAP`)
    async fn update_policy(
        &self,
        _key: PolicyKey,
        _verdict: Verdict,
    ) -> Result<(), DataplaneError> {
        Ok(())
    }

    /// see #25 (`SERVICE_MAP`)
    async fn update_service(
        &self,
        _vip: Ipv4Addr,
        _backends: Vec<Backend>,
    ) -> Result<(), DataplaneError> {
        Ok(())
    }

    /// see #27 (telemetry `ringbuf`)
    async fn drain_flow_events(&self) -> Result<Vec<FlowEvent>, DataplaneError> {
        Ok(vec![])
    }
}

#[cfg(test)]
mod tests {
    //! macOS-side regression guards for the `#[cfg(not(target_os =
    //! "linux"))]` stub branch. The branch is one line of code, but
    //! the test exists to prevent silent erosion of the boundary —
    //! a future refactor that drops the cfg gate, weakens the
    //! diagnostic, or returns a different error variant trips this
    //! assertion on macOS CI before the change reaches Linux.
    //!
    //! On Linux the test is `#[cfg(not(target_os = "linux"))]`-gated
    //! and silently absent — the Tier 3 LVH smoke (`cargo xtask
    //! integration-test vm latest`, step 03-02) is the corresponding
    //! Linux-side gate.

    // Imports are only consumed by the `#[cfg(not(target_os =
    // "linux"))]` test below, so they're dead on Linux. The cfg gate
    // can't sit on `use` directly without complicating the macOS
    // path; allowing here keeps both paths clean.
    #[cfg(not(target_os = "linux"))]
    use super::{DataplaneError, EbpfDataplane};

    /// On non-Linux build targets the constructor returns
    /// [`DataplaneError::LoadFailed`] carrying the `"non-Linux build
    /// target"` diagnostic — never any other variant, never a
    /// surprise `Ok(_)`.
    #[cfg(not(target_os = "linux"))]
    #[test]
    fn new_returns_load_failed_with_non_linux_diagnostic() {
        // `EbpfDataplane` does not implement `Debug` (its inner aya
        // types do not, and adding a manual impl is noise for a stub
        // that lives only on Linux). Unwrap the `Result` via match
        // rather than `expect_err`, which would require `T: Debug`.
        match EbpfDataplane::new("lo") {
            Err(DataplaneError::LoadFailed(msg)) => {
                assert!(msg.contains("non-Linux build target"), "unexpected diagnostic: {msg}");
            }
            Err(other) => panic!("expected DataplaneError::LoadFailed, got {other:?}"),
            Ok(_) => panic!("expected Err on non-Linux build target"),
        }
    }
}
