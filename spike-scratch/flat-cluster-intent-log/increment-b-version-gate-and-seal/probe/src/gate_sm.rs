//! An application-level committed cluster-version gate, built ONLY on viewstamp's `StateMachine`
//! trait (`apply` / `checkpoint_image` / `materialize` / `restore_seed` / `restore`).
//!
//! Every committed body is `[tag: u16 BE][kind: u8][payload]`. `tag` is the entry-format version
//! the writer used. Kinds:
//!   0 = Op(bytes)
//!   1 = Advertise { member: u128, max: u16 }        — a node's supported version, as a log fact
//!   2 = Gate { v: u16, voters: [u128] }             — "ClusterVersion(v)", checked at apply time
//!
//! Rules (identical on every replica, since they read only replicated state):
//!   - an entry tagged above the COMMITTED cluster version is rejected (no-op) — nobody halts on an
//!     entry written in a format the cluster has not committed;
//!   - a Gate is accepted iff v > cluster_version and every listed voter has advertised >= v;
//!   - a replica whose binary supports < v HALTS at an accepted Gate: it does not apply it, and
//!     every later entry is skipped (its logical state stays exactly the pre-gate prefix).
//!
//! The one thing the SM CANNOT do on this API: know the real voter set at the apply position.
//! `apply(op, body)` carries no membership, so the Gate trusts the voter list its proposer wrote.

use std::collections::BTreeMap;

use bytes::{Buf, BufMut, Bytes, BytesMut};
use viewstamp_proto::{BlockAddress, BlockStore, OpNumber, RestoreError, StateMachine, VerifiedView};

pub const KIND_OP: u8 = 0;
pub const KIND_ADVERTISE: u8 = 1;
pub const KIND_GATE: u8 = 2;

pub fn op(tag: u16, payload: &[u8]) -> Bytes {
  let mut b = BytesMut::new();
  b.put_u16(tag);
  b.put_u8(KIND_OP);
  b.put_slice(payload);
  b.freeze()
}

pub fn advertise(tag: u16, member: u128, max: u16) -> Bytes {
  let mut b = BytesMut::new();
  b.put_u16(tag);
  b.put_u8(KIND_ADVERTISE);
  b.put_u128(member);
  b.put_u16(max);
  b.freeze()
}

pub fn gate(tag: u16, v: u16, voters: &[u128]) -> Bytes {
  let mut b = BytesMut::new();
  b.put_u16(tag);
  b.put_u8(KIND_GATE);
  b.put_u16(v);
  for m in voters {
    b.put_u128(*m);
  }
  b.freeze()
}

/// FNV-1a over `(op, body)` of every entry this replica applied: equal digests at equal ops mean
/// identical applied prefixes.
fn fnv(mut h: u64, bytes: &[u8]) -> u64 {
  for b in bytes {
    h ^= u64::from(*b);
    h = h.wrapping_mul(0x0000_0100_0000_01B3);
  }
  h
}
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateSm {
  /// CONFIGURATION (the binary's capability) — carried by `restore_seed`, never by the checkpoint.
  supported_max: u16,
  cluster_version: u16,
  /// `(op, version needed)` where this replica halted.
  halted_at: Option<(u64, u16)>,
  adverts: BTreeMap<u128, u16>,
  applied: u64,
  digest: u64,
}

impl GateSm {
  pub fn new(supported_max: u16) -> Self {
    Self {
      supported_max,
      cluster_version: 1,
      halted_at: None,
      adverts: BTreeMap::new(),
      applied: 0,
      digest: FNV_OFFSET,
    }
  }

  fn absorb(&mut self, op: u64, body: &[u8]) {
    self.digest = fnv(fnv(self.digest, &op.to_be_bytes()), body);
    self.applied += 1;
  }

  fn reply(&self, verdict: &str) -> Bytes {
    Bytes::from(format!(
      "{verdict} cv={} applied={} digest={:016x}",
      self.cluster_version, self.applied, self.digest
    ))
  }

  fn encode_image(&self) -> Bytes {
    let mut b = BytesMut::new();
    b.put_u16(self.cluster_version);
    match self.halted_at {
      Some((op, need)) => {
        b.put_u8(1);
        b.put_u64(op);
        b.put_u16(need);
      }
      None => b.put_u8(0),
    }
    b.put_u32(self.adverts.len() as u32);
    for (m, v) in &self.adverts {
      b.put_u128(*m);
      b.put_u16(*v);
    }
    b.put_u64(self.applied);
    b.put_u64(self.digest);
    b.freeze()
  }

  fn decode_image_into(&mut self, mut b: Bytes) {
    self.cluster_version = b.get_u16();
    self.halted_at = match b.get_u8() {
      1 => Some((b.get_u64(), b.get_u16())),
      _ => None,
    };
    let n = b.get_u32();
    self.adverts.clear();
    for _ in 0..n {
      let m = b.get_u128();
      let v = b.get_u16();
      self.adverts.insert(m, v);
    }
    self.applied = b.get_u64();
    self.digest = b.get_u64();
  }
}

impl StateMachine for GateSm {
  type Image = Bytes;

  fn apply(&mut self, op: OpNumber, body: &[u8]) -> Bytes {
    let op = op.get();
    if let Some((at, need)) = self.halted_at {
      return self.reply(&format!(
        "SKIP-HALTED op={op} halted_at={at} needs=v{need} supports=v{}",
        self.supported_max
      ));
    }
    let mut b = Bytes::copy_from_slice(body);
    if b.remaining() < 3 {
      self.absorb(op, body);
      return self.reply(&format!("REJECTED-MALFORMED op={op}"));
    }
    let tag = b.get_u16();
    let kind = b.get_u8();
    // Deterministic on every replica: a format the cluster has not committed is refused, not halted on.
    if tag > self.cluster_version {
      self.absorb(op, body);
      return self.reply(&format!("REJECTED-UNCOMMITTED-FORMAT op={op} tag=v{tag}"));
    }
    // Defensive: only reachable if this replica somehow did not halt at the gate.
    if tag > self.supported_max {
      self.halted_at = Some((op, tag));
      return self.reply(&format!("HALT op={op} tag=v{tag} supports=v{}", self.supported_max));
    }
    match kind {
      KIND_OP => {
        self.absorb(op, body);
        self.reply(&format!("APPLIED op={op} tag=v{tag}"))
      }
      KIND_ADVERTISE => {
        let member = b.get_u128();
        let max = b.get_u16();
        self.adverts.insert(member, max);
        self.absorb(op, body);
        self.reply(&format!("ADVERTISE op={op} member={member} max=v{max}"))
      }
      KIND_GATE => {
        let v = b.get_u16();
        let mut voters = Vec::new();
        while b.remaining() >= 16 {
          voters.push(b.get_u128());
        }
        let all_support =
          !voters.is_empty() && voters.iter().all(|m| self.adverts.get(m).is_some_and(|x| *x >= v));
        if v <= self.cluster_version || !all_support {
          self.absorb(op, body);
          return self.reply(&format!("GATE-REJECTED op={op} v={v} voters={voters:?}"));
        }
        if v > self.supported_max {
          // Halt AT the gate: the pre-gate prefix is exactly what this replica applied.
          self.halted_at = Some((op, v));
          return self.reply(&format!(
            "HALT op={op} gate=v{v} supports=v{}",
            self.supported_max
          ));
        }
        self.cluster_version = v;
        self.absorb(op, body);
        self.reply(&format!("GATE-COMMITTED op={op} v={v} voters={voters:?}"))
      }
      _ => {
        self.absorb(op, body);
        self.reply(&format!("REJECTED-UNKNOWN-KIND op={op} kind={kind}"))
      }
    }
  }

  fn checkpoint_image(&self) -> Self::Image {
    self.encode_image()
  }

  fn materialize(image: &Self::Image, store: &mut dyn BlockStore) -> BlockAddress {
    store.put(image.clone())
  }

  fn restore_seed(&self) -> Self {
    Self::new(self.supported_max)
  }

  fn restore(&mut self, root: BlockAddress, store: &VerifiedView<'_>) -> Result<(), RestoreError> {
    let block = store.read_block(root).ok_or(RestoreError::new(root))?;
    self.decode_image_into(block);
    // DELIBERATELY NAIVE resume policy (what an implementer would write first): an upgraded binary
    // that now supports the version it halted on simply un-halts. Phase 3a of the probe tests
    // whether that is safe on viewstamp's restart-in-place path.
    if let Some((_, need)) = self.halted_at {
      if need <= self.supported_max {
        self.halted_at = None;
      }
    }
    Ok(())
  }
}
