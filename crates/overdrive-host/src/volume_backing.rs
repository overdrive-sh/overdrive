//! `LvmVolumeBacking` and `LvmVolumePoolProbe`: the production volume pool binding.
//!
//! GH #348, ADR-0312, ADR-0313, ADR-0314. RED scaffold (DISTILL): the probe
//! reports "no labelled partition" until DELIVER implements discovery and the probe.

#![allow(clippy::unused_async, clippy::unused_self, reason = "DISTILL RED scaffold")]

use std::sync::Arc;

use async_trait::async_trait;
use overdrive_core::traits::volume_backing::{VolumeBacking, VolumePoolProbe, VolumeProbeError};

/// The only place that spawns `lvm`.
///
/// Its constructor is `pub(crate)` (landing with DELIVER, over the node the probe
/// discovered), so only [`LvmVolumePoolProbe`] builds one, after the probe scenarios
/// pass; the private field already keeps every other crate from constructing it.
#[derive(Debug)]
pub struct LvmVolumeBacking {
    _private: (),
}

/// The production [`VolumePoolProbe`]: discovers the partition labelled
/// `overdrive-volumes`, probes it, and builds the one [`LvmVolumeBacking`] over it.
#[derive(Debug, Default)]
pub struct LvmVolumePoolProbe {
    _private: (),
}

impl LvmVolumePoolProbe {
    /// Infallible and kernel-silent.
    #[must_use]
    pub const fn new() -> Self {
        Self { _private: () }
    }
}

#[async_trait]
impl VolumePoolProbe for LvmVolumePoolProbe {
    async fn probe(&self) -> Result<Option<Arc<dyn VolumeBacking>>, VolumeProbeError> {
        // RED scaffold: reports "no labelled partition" so every existing boot is
        // unchanged until DELIVER implements discovery and the probe (ADR-0314).
        Ok(None)
    }
}
