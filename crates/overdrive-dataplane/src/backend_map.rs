//! The crate-private seam over `BACKEND_MAP`.
//!
//! `EbpfDataplane` reaches `BACKEND_MAP` only through [`BackendMapOps`]. The
//! production implementation is the real aya hash map, unchanged in behaviour;
//! a test-support decorator may wrap it in-crate. Every call the seam carries
//! is a `bpf(2)` map syscall made by the control-plane `update_service`, the
//! probe, the orphan sweep or an inspector. The XDP and `cgroup_sock_addr`
//! programs read the kernel map directly and never pass through this handle.

use aya::maps::{HashMap, MapData, MapError};

use crate::maps::BackendEntryPod;

/// The `BACKEND_MAP` operations `EbpfDataplane` performs, keyed by the raw
/// `BackendId`.
pub trait BackendMapOps {
    /// The entry stored under `id`; `MapError::KeyNotFound` when absent.
    fn get(&self, id: u32) -> Result<BackendEntryPod, MapError>;

    /// Store `entry` under `id`, replacing any prior entry.
    fn insert(&mut self, id: u32, entry: BackendEntryPod) -> Result<(), MapError>;

    /// Remove the entry under `id`.
    fn remove(&mut self, id: u32) -> Result<(), MapError>;

    /// Every key present, in the kernel's traversal order.
    fn keys(&self) -> Box<dyn Iterator<Item = Result<u32, MapError>> + '_>;
}

impl<T> BackendMapOps for HashMap<T, u32, BackendEntryPod>
where
    T: std::borrow::BorrowMut<MapData>,
{
    fn get(&self, id: u32) -> Result<BackendEntryPod, MapError> {
        Self::get(self, &id, 0)
    }

    fn insert(&mut self, id: u32, entry: BackendEntryPod) -> Result<(), MapError> {
        Self::insert(self, id, entry, 0)
    }

    fn remove(&mut self, id: u32) -> Result<(), MapError> {
        Self::remove(self, &id)
    }

    fn keys(&self) -> Box<dyn Iterator<Item = Result<u32, MapError>> + '_> {
        Box::new(Self::keys(self))
    }
}
