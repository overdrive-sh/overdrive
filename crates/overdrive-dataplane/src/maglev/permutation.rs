//! Maglev permutation generator (Eisenbud, NSDI 2016, weighted
//! variant per Cilium / Katran).
//!
//! Pure synchronous function — `BTreeMap` order is the canonical
//! input ordering per `.claude/rules/development.md` § Ordered-
//! collection choice. The produced permutation is bit-identical
//! across runs and across nodes given identical inputs (DST
//! invariant `MaglevDeterministic`; S-2.2-12).
//!
//! # Algorithm (Eisenbud NSDI 2016 § 5.3, weighted via multiplicity)
//!
//! Given backends `B_0, …, B_{N-1}` with weights `w_i` and table size
//! `M` (prime, from [`MaglevTableSize`]):
//!
//! 1. **Multiplicity expansion** — flatten `(B_i, w_i)` into a
//!    `Vec<BackendId>` of length `Σ w_i`, where `B_i` appears `w_i`
//!    times. `BTreeMap` order is preserved across the expansion, so
//!    the round-robin step below is deterministic across runs.
//! 2. **Per-entry permutation** — for each entry `j` in the expanded
//!    vector, derive `(offset_j, skip_j)` from a deterministic hash of
//!    the entry's identity (`(BackendId, replica_index)`):
//!
//!    ```text
//!    offset_j = hash(BackendId, replica_index, "offset") mod M
//!    skip_j   = hash(BackendId, replica_index, "skip"  ) mod (M - 1) + 1
//!    perm_j[k] = (offset_j + k * skip_j) mod M
//!    ```
//!
//!    `M` is prime ⇒ `gcd(skip_j, M) = 1` ⇒ `perm_j` is a permutation
//!    of `0..M`.
//! 3. **Population** — round-robin across the expanded vector. Each
//!    iteration picks the next slot from the entry's permutation; if
//!    the slot is taken, walk the permutation until an empty slot is
//!    found. Continues until all `M` slots are filled.
//!
//! # Determinism
//!
//! The hash function is FNV-1a with a fixed 64-bit offset basis. No
//! `std::collections::DefaultHasher` (per-process random seed; would
//! violate K3 reproducibility from whitepaper § 21).
//!
//! # Saturating arithmetic
//!
//! Multiplicity expansion saturates `Σ w_i` against `usize::MAX`. The
//! `Weight` type is `u16` (max 65_535); even at `N = 65_535` backends
//! each at `u16::MAX`, the expanded vector size is bounded by
//! `4_294_836_225` — well within `usize::MAX` on 64-bit. The saturating
//! discipline is structural: a future widening of `Weight` to `u32`
//! would not introduce a panic surface.

use std::collections::BTreeMap;

use overdrive_core::dataplane::MaglevTableSize;
use overdrive_core::id::BackendId;

/// Per-backend weight. `u16` matches the `BACKEND_MAP` value-shape
/// `weight: u16` per architecture.md § 10.
pub type Weight = u16;

/// FNV-1a 64-bit offset basis (FNV-1a spec).
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
/// FNV-1a 64-bit prime (FNV-1a spec).
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Deterministic 64-bit FNV-1a hash over a sequence of byte slices.
/// Inlined rather than pulled in as a dep — the algorithm is ~5 LoC
/// and avoiding a transitive dep keeps the dataplane crate's graph
/// trim. The hash is used only for permutation seeding; not a
/// security-sensitive primitive.
#[inline]
fn fnv1a_64(parts: &[&[u8]]) -> u64 {
    let mut h = FNV_OFFSET;
    for part in parts {
        for &b in *part {
            h ^= u64::from(b);
            h = h.wrapping_mul(FNV_PRIME);
        }
    }
    h
}

/// Generate the Maglev permutation table for the given weighted
/// backend set and table size. Pure synchronous; deterministic.
///
/// Inputs iterated in `BTreeMap` order so the produced permutation
/// is bit-identical across runs.
///
/// # Panics
///
/// Does not panic. An empty `backends` map is the caller's
/// responsibility to short-circuit before invoking `generate`; the
/// result for empty inputs is an empty vector (no backends ⇒ no
/// permutation), which is structurally harmless but useless. The
/// `xdp_service_map_lookup` hot path checks for empty backends
/// upstream (returns `XDP_DROP` with `DropClass::NoBackends`).
///
/// # Determinism
///
/// Two successive calls with identical inputs return bit-identical
/// `Vec<BackendId>`. This is the S-2.2-12 invariant.
pub fn generate(backends: &BTreeMap<BackendId, Weight>, m: MaglevTableSize) -> Vec<BackendId> {
    let m_u32 = m.get();
    let m_usize = m_u32 as usize;

    if backends.is_empty() || m_u32 == 0 {
        return Vec::new();
    }

    // Step 1: multiplicity expansion. Each backend `B_i` appears `w_i`
    // times in deterministic `BTreeMap` order. `replica_index` tracks
    // the per-backend position so each entry's permutation is unique.
    //
    // Saturating accumulation: while `u16` weights cannot overflow
    // `usize` on 64-bit, the discipline is structural — a future
    // widening of `Weight` would not introduce a panic surface.
    let total_replicas: usize =
        backends.values().copied().map(usize::from).fold(0usize, usize::saturating_add);

    let mut entries: Vec<(BackendId, u16)> = Vec::with_capacity(total_replicas);
    for (id, weight) in backends {
        for replica in 0..*weight {
            entries.push((*id, replica));
        }
    }

    // Step 2: per-entry `(offset, skip)` derived from a fixed-seed
    // FNV-1a hash. `m_u32 >= 251` (smallest prime in `ALLOWED_PRIMES`)
    // so `m_u32 - 1 >= 250 > 0` and the modular arithmetic is safe.
    //
    // The hash key includes the BackendId's u32 form, the replica
    // index, and a fixed seed-tag distinguishing `offset` from `skip`.
    // BackendId iteration order is the BTreeMap order; replica index
    // is local; both are stable across runs.
    let table_minus_one = u64::from(m_u32 - 1);
    let m_u64 = u64::from(m_u32);

    let mut perms: Vec<(u32, u32)> = Vec::with_capacity(entries.len());
    for (id, replica) in &entries {
        let id_bytes = id.get().to_le_bytes();
        let rep_bytes = replica.to_le_bytes();
        let offset_seed = b"overdrive-maglev-offset";
        let skip_seed = b"overdrive-maglev-skip";

        let h_offset = fnv1a_64(&[offset_seed, &id_bytes, &rep_bytes]);
        let h_skip = fnv1a_64(&[skip_seed, &id_bytes, &rep_bytes]);

        // SAFETY of the cast: `h_offset % m_u64 < m_u64 ≤ u32::MAX`
        // because `m: MaglevTableSize` is constrained to ALLOWED_PRIMES
        // (max 131_071 < 2^32). Same reasoning for the skip cast — the
        // modular bound proves the value fits in u32 without truncation.
        #[allow(clippy::cast_possible_truncation)]
        let offset = (h_offset % m_u64) as u32;
        #[allow(clippy::cast_possible_truncation)]
        let skip = ((h_skip % table_minus_one) + 1) as u32;

        perms.push((offset, skip));
    }

    // Step 3: round-robin population. Each iteration of the outer
    // loop assigns one slot per entry in turn; entries compete for
    // slots and walk their permutation when their preferred slot is
    // taken. Loop terminates when all `M` slots are filled (Maglev's
    // termination guarantee — the `(offset, skip)` permutation is a
    // bijection on `0..M` so each entry has at most `M` candidates).
    let n = entries.len();
    let mut next_idx = vec![0u32; n];
    let mut result: Vec<Option<BackendId>> = vec![None; m_usize];
    let mut filled = 0usize;

    'outer: while filled < m_usize {
        for entry_idx in 0..n {
            let (offset, skip) = perms[entry_idx];
            // Walk the permutation until we find an empty slot.
            // Bounded loop: at most `M` candidates per entry per
            // outer-loop iteration; the verifier-style proof is that
            // each entry's permutation is a bijection on `0..M`, and
            // the outer loop's progress invariant (`filled` is
            // monotonically increasing) bounds total work at `M * M`.
            loop {
                let probe = next_idx[entry_idx];
                let slot = (offset.wrapping_add(probe.wrapping_mul(skip)) % m_u32) as usize;
                next_idx[entry_idx] = probe.wrapping_add(1);
                if result[slot].is_none() {
                    result[slot] = Some(entries[entry_idx].0);
                    filled += 1;
                    break;
                }
                // Defensive bound. The invariant guarantees we find a
                // slot within `M` probes per entry, but we cap probe
                // count at `M` to avoid pathological loops if a
                // future refactor weakens the bijection invariant.
                if u64::from(next_idx[entry_idx]) >= m_u64 {
                    // Should be unreachable given the bijection
                    // invariant; structurally fall through to the
                    // next entry to keep the loop well-defined.
                    break;
                }
            }
            if filled == m_usize {
                break 'outer;
            }
        }
    }

    // Every slot must be filled — the Maglev population loop is
    // total over `0..M`. Unwrap is safe; if it ever fires, the
    // bijection invariant has been broken and the algorithm itself
    // is wrong.
    // Every slot is `Some` by the population-loop termination
    // condition (`filled == m_usize`); `unwrap_or` provides a
    // structurally-sound default (the first backend) for the
    // verifier-impossible case where the bijection invariant has been
    // broken. Returning the first backend instead of panicking keeps
    // `generate` total over its input space.
    let fallback = entries[0].0;
    result.into_iter().map(|s| s.unwrap_or(fallback)).collect()
}
