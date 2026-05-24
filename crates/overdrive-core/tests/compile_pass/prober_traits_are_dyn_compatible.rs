//! Compile-pass fixture — `TcpProber` / `HttpProber` / `ExecProber`
//! are object-safe (dyn-compatible).
//!
//! Per ADR-0054 §3, every prober trait must be reachable behind
//! `Arc<dyn TcpProber>` / `Arc<dyn HttpProber>` / `Arc<dyn ExecProber>`
//! because the `ProbeRunner` (slice 01-03) stores heterogeneous
//! probers in a per-alloc-per-probe task graph keyed by mechanic.
//!
//! `async_trait` rewrites `async fn probe(&self, ...) -> ...` into
//! `fn probe(&self, ...) -> Pin<Box<dyn Future<Output = ...> + Send + '_>>`,
//! which IS object-safe. This fixture pins that property — if a
//! future edit added an `async fn` returning `impl Trait` directly
//! (which is NOT object-safe), the compile would fail here.

use std::sync::Arc;

use overdrive_core::traits::prober::{ExecProber, HttpProber, TcpProber};
use overdrive_sim::adapters::probers::{SimExecProber, SimHttpProber, SimTcpProber};

fn _accepts_dyn_tcp(_p: Arc<dyn TcpProber>) {}
fn _accepts_dyn_http(_p: Arc<dyn HttpProber>) {}
fn _accepts_dyn_exec(_p: Arc<dyn ExecProber>) {}

fn main() {
    let tcp: Arc<dyn TcpProber> = Arc::new(SimTcpProber::new());
    let http: Arc<dyn HttpProber> = Arc::new(SimHttpProber::new());
    let exec: Arc<dyn ExecProber> = Arc::new(SimExecProber::new());
    _accepts_dyn_tcp(tcp);
    _accepts_dyn_http(http);
    _accepts_dyn_exec(exec);
}
