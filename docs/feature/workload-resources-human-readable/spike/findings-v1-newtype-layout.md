# V-1 — single-field newtypes keep the rkyv archived layout

**Verdict: WORKS.** Wrapping `cpu_milli: u32` / `memory_bytes: u64` in single-field
newtypes (`CpuQuantity(u32)`, `ByteSize(u64)`) under rkyv 0.8 (`bytecheck`) produces
byte-identical archives, flat and embedded in a `V1` enum envelope, and bytes
written with the old shape validate and decode through the new type.

- Probe: `spike-scratch/resources-newtype-layout/` (`run.sh`, `src/main.rs`)
- Run: `cargo xtask metal run -- bash spike-scratch/resources-newtype-layout/run.sh`,
  run id `2290cbebcd6604db77be3067`, kernel `7.0.0-38-generic`, cargo 1.95.0.
- Inputs: `(0,1)`, `(500,134217728)`, `(u32::MAX,u64::MAX)`, `(1500,1<<40)` —
  the production values (`500`, `128MiB`) plus both bounds.

```
flat c=500 m=134217728: old=f4010000000000000000000800000000 new=f4010000000000000000000800000000 equal=true
envelope c=500 m=134217728: len old=56 new=56 equal=true cross_decode=true
flat c=4294967295 m=18446744073709551615: old=ffffffff00000000ffffffffffffffff new=ffffffff00000000ffffffffffffffff equal=true
envelope c=4294967295 m=18446744073709551615: len old=56 new=56 equal=true cross_decode=true
VERDICT: WORKS (byte-identical, cross-decodes)
```

Design implication: field renames do not enter the archive (rkyv is positional), and a
`#[repr(transparent)]`-shaped single-field struct archives as its inner primitive. The
existing golden fixtures stay valid, so DD-5 (no envelope bump) holds. The probe models
the aggregate shape, not the real `WorkloadIntent`; DELIVER's untouched golden fixtures
are the regression check on the real types.
