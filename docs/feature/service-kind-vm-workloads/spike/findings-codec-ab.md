# Codec A/B Spike Findings — service-kind-vm-workloads

**Date:** 2026-09-06

**Scope:** One assumption only; isolated PROBE, not production implementation

**Authoritative run:** `increment-d-codec-ab-corrected/runs/0003`

**Verdict:** **WORKS — the measured priority order favors `rkyv`, with an explicit evolution cost**

## Question tested

For the same bounded, versioned VM-Exec control message algebra and the actual
`overdrive-init` release/link environment, does length-prefixed JSON or
length-prefixed `rkyv` have a binary-size, encoded-size,
validation-complexity, or schema-evolution advantage sufficient to choose the
codec?

There was no timing budget and no materiality threshold. The predeclared
decision order was:

1. stripped incremental init-representative binary size;
2. safe bounded decoding and evolution complexity;
3. encoded bytes and decode timing, because traffic is small and bounded.

Under that order, `rkyv` wins the primary measured criterion: **+24,568 B**
over the representative baseline versus JSON's **+77,816 B**. Both candidates
passed the same bounded rejection cases. JSON has the simpler additive-schema
story, so this is not an across-the-board dominance claim.

## Prediction and falsification recorded before execution

```text
PREDICTION json_incremental_binary_smaller_or_equal=UNKNOWN rkyv_wire_smaller=EXPECTED decode_speed_rkyv_faster=EXPECTED
FALSIFICATION no_material_binary_or_validation_advantage_means_codec_choice_remains_design_tradeoff
```

Actual: `rkyv` was smaller on wire and faster to decode as expected, but it
also produced the smaller incremental stripped representative binary despite
adding a new runtime codec dependency. The assumption that existing JSON use
would necessarily make an additional typed JSON protocol cheaper to link was
wrong.

## Grounding in the current init binary

The real crate targets static `x86_64-unknown-linux-musl`. Its workspace
release profile uses thin LTO, one codegen unit, line-table debug information,
`panic = "abort"`, and `strip = "symbols"`. The standalone A/B manifest copies
those settings exactly; its `release-unstripped` profile changes only stripping
so the diagnostic size is also observable.

Beacon's current `EXEC` path actively calls JSON rather than merely declaring
it as a dependency:

- `crates/overdrive-core/src/vm/beacon.rs:217` calls
  `serde_json::to_string(argv)`;
- `crates/overdrive-core/src/vm/beacon.rs:335` calls
  `serde_json::from_str(json)`;
- current `overdrive-init` sends/parses `BeaconMessage::Exec` through that
  production module;
- the unstripped actual binary contained 34 defined `serde_json` symbols.

Raw native-metal evidence:

```text
SUBSTRATE kernel=Linux 7.0.0-29-generic arch=x86_64 virtualization=none
TOOLCHAIN rustc 1.95.0 (59807616e 2026-04-14) | cargo 1.95.0 (f2d3ce0bd 2026-03-21)
ACTUAL_INIT_SOURCE serde_json_encode=crates/overdrive-core/src/vm/beacon.rs:217 serde_json_decode=crates/overdrive-core/src/vm/beacon.rs:335
ACTUAL_INIT stripped_bytes=635632 format=ELF 64-bit LSB pie executable, x86-64, version 1 (SYSV), static-pie linked, BuildID[sha1]=b84d563ca4324c6a2c60f77983381323531e9321, stripped
ACTUAL_INIT_LINK_EVIDENCE unstripped_bytes=3321576 defined_serde_json_symbols=34
```

The real stripped binary's 635,632 B is a baseline fact, not a denominator for
blindly projecting the standalone deltas. Only embedding a candidate in the
full production init would produce an exact production delta, and this spike
was forbidden from editing production.

## Provisional logical algebra held constant

Both codec variants compile the same provisional, non-accepted algebra:

- handshake with protocol version and session generation;
- execute with session generation, request ID, and direct argv;
- cancel with session generation and request ID;
- overload with session generation and request ID;
- typed result with seven categories: success, nonzero exit, signal, timeout,
  cancellation, unavailable, and protocol failure.

Both use the same four-byte big-endian length prefix, four-byte schema-version
prefix inside the payload, 16 KiB declared-length cap checked before codec
decoding, and exact-length check. Framing and cap behavior are therefore
credited to neither codec.

Every candidate executable first executes the representative of Beacon's
already-active `serde_json` `Vec<String>` encode/decode path. Increment C's
first runtime attempt exposed that failing to call this baseline in every
variant let thin LTO remove JSON from the `rkyv` binary; increment D corrects
that experimental defect and is authoritative.

## Binary size

| Variant | Unstripped bytes | Delta vs baseline | Stripped bytes | Delta vs baseline |
|---|---:|---:|---:|---:|
| Existing-JSON representative baseline | 2,668,584 | — | 516,848 | — |
| Baseline + typed JSON VM-Exec codec | 3,015,560 | +346,976 (+13.00%) | 594,664 | +77,816 (+15.06%) |
| Baseline + validated `rkyv` VM-Exec codec | 2,814,576 | +145,992 (+5.47%) | 541,416 | +24,568 (+4.75%) |

`rkyv` was **53,248 B smaller** than JSON after production-style symbol
stripping (8.95% of the JSON representative binary), and 200,984 B smaller
unstripped. The exact sizes repeated across all three successful increment-D
runs.

This measurement does not invent a threshold for “material.” It establishes a
strict ranking under the already-declared primary criterion. The 53,248 B gap
must not be described as an exact future `overdrive-init` saving because the
full binary can deduplicate and inline differently.

Raw output:

```text
BINARY name=baseline unstripped_bytes=2668584 stripped_bytes=516848
BINARY name=json unstripped_bytes=3015560 stripped_bytes=594664
BINARY name=rkyv unstripped_bytes=2814576 stripped_bytes=541416
```

## Encoded size

Frame sizes include the common four-byte length prefix and four-byte schema
version.

| Identical logical value | JSON | `rkyv` | `rkyv` difference |
|---|---:|---:|---:|
| Canonical execute request | 112 B | 88 B | −24 B (−21.43%) |
| Canonical success result | 97 B | 40 B | −57 B (−58.76%) |
| Maximum common representative request | 16,388 B | 16,328 B | −60 B (−0.37%) |

The maximum common input contains a 16,274-byte argv string. JSON is the
limiting encoding and lands exactly at the 16,384-byte payload cap (16,388 B
including the outer length); `rkyv` leaves 60 bytes unused for the same value.
The savings are immaterial to throughput at the expected probe volume, but
they are directionally consistent.

Raw output:

```text
ENCODED request_frame=112 result_frame=97 max_representative_frame=16388 cap=16388
ENCODED request_frame=88 result_frame=40 max_representative_frame=16328 cap=16388
```

## Decode and validation timing

Each successful run decoded and validated the canonical request 100,000 times
inside the stripped native-metal binary. There is no performance budget, so
these are descriptive—not pass/fail—numbers.

| Run | JSON ns/op | `rkyv` ns/op | Ratio |
|---|---:|---:|---:|
| increment D / 0001 | 797.58 | 160.63 | 4.97× |
| increment D / 0002 | 797.55 | 160.44 | 4.97× |
| increment D / 0003 | 948.89 | 162.68 | 5.83× |

The JSON variance in run 0003 shows why timing is secondary here. Even the
largest measured time is under one microsecond per small frame, and actual
probe execution will dominate it.

## Malformed, truncated, oversized, and version behavior

Both runtime candidates passed:

- malformed codec payload rejection;
- truncated-frame rejection through an exact declared-length check;
- oversized declared length rejection before codec decoding;
- unsupported schema rejection before codec access.

The `rkyv` path uses
`rkyv::from_bytes::<MessageV1, rkyv::rancor::Error>` only after framing and
schema checks, so bytecheck validates the archive before deserializing or
accessing an archived value.

```text
REJECTION malformed=PASS truncated=PASS oversized_preallocation=PASS unsupported_schema=PASS
REJECTION malformed=PASS truncated=PASS oversized_preallocation=PASS unsupported_schema=PASS bytecheck_before_access=PASS
```

The raw `oversized_preallocation=PASS` label overstates what this isolated
harness proves: `unframe` receives an already-complete byte slice. It verifies
the ordering of the declared-length check ahead of codec decoding, but a future
stream reader must separately read the length, enforce the cap, and only then
allocate/read the payload.

## Schema evolution

JSON's V1 reader accepted an additive V2-only field while the common major
schema remained V1, because serde ignores unknown fields by default:

```text
EVOLUTION v1_reader_additive_same_schema=ACCEPT
```

That tolerance is policy, not an intrinsic guarantee. Adding
`#[serde(deny_unknown_fields)]` would reverse the result, while silently
ignoring a misspelled field is the corresponding risk of the tolerant policy.
Breaking changes still require a version transition and old/new decoder
policy.

The `rkyv` V1 reader rejected the additive V2 archived layout through the
explicit schema prefix before archived access:

```text
EVOLUTION v1_reader_additive_new_layout=REJECT
```

That rejection is the safe result. `rkyv` layouts are positional; adding a
field requires a new versioned payload and retaining a decoder/up-conversion
path for every supported peer version. The wire must never try a new archive
as the old type and hope bytecheck distinguishes every semantic mismatch.

## Source and dependency complexity

- One 309-line source compiles all three variants, with common framing and
  baseline code shared. The JSON-specific module is 107 lines; the
  `rkyv`-specific module is 122 lines, including the explicit V2 archive used
  to demonstrate evolution.
- Unique normal dependency-tree lines measured by Cargo: baseline 6, JSON 14,
  `rkyv` 29. JSON adds serde derives to a graph that already contains
  `serde_json`; `rkyv` adds bytecheck, rancor, pointer-metadata, archive
  derives, and related support crates.
- Procedural-macro dependencies affect builds but do not themselves link into
  the runtime binary. The binary-size table—not dependency count—is the linked
  result.
- JSON remains human-readable with ordinary inspection tools. An `rkyv` wire
  needs version-aware decoding tooling and couples peers to the chosen archive
  configuration and Rust data layout.

The raw run's `SOURCE_FACTS ... json_cfg_lines=232` label is an instrumentation
label defect: its `sed` range matched both `mod selected` blocks and summed
them. Direct line boundaries establish the 107/122 split above. This did not
affect compilation, size, wire, rejection, or timing measurements.

## Attempts and corrections

All attempts are preserved; nothing was overwritten.

| Attempt | Result | Evidence |
|---|---|---|
| increment C / 0001 | setup failure | rsync encountered a prior root-owned ignored `.context` target; no probe executed |
| increment C / 0002 | preflight refusal | required kernel/rootfs selectors were not supplied; fail-closed as intended |
| increment C / 0003 | experimental-design failure | runtime revealed malformed JSON fixture and that the `rkyv` feature build had eliminated the existing-JSON baseline path |
| increment D / 0001 | PASS | corrected all variants to execute the existing JSON path |
| increment D / 0002 | PASS | added actual-init link evidence: 34 defined `serde_json` symbols |
| increment D / 0003 | PASS, authoritative | used the maximum identical logical request admitted by both codecs |

Increment C was not rewritten after the material experimental correction; the
corrected source is a new increment as required by `.claude/rules/spike.md`.

## Reproduction and evidence integrity

Canonical command from repository root:

```sh
bash spike-scratch/service-kind-vm-workloads/increment-d-codec-ab-corrected/capture.sh 0004
```

Expanded sanitized metal command recorded in metadata:

```text
RSYNC_BIN=spike-scratch/service-kind-vm-workloads/increment-d-codec-ab-corrected/rsync-without-local-env.sh OVERDRIVE_METAL_KERNEL=/srv/vm/overdrive-testing/kernel OVERDRIVE_METAL_ROOTFS=/srv/vm/overdrive-testing/rootfs.ext4 OVERDRIVE_METAL_SCENARIO=service-vm-codec-ab cargo xtask metal run -- bash spike-scratch/service-kind-vm-workloads/increment-d-codec-ab-corrected/run.sh
```

Tracked-candidate paths:

- `spike-scratch/service-kind-vm-workloads/increment-c-codec-ab/` — failed
  attempts 0001–0003 and the superseded harness;
- `spike-scratch/service-kind-vm-workloads/increment-d-codec-ab-corrected/`
  — corrected manifest, lockfile, source, runner, capture/redaction wrapper,
  README, and append-only runs 0001–0003;
- `docs/feature/service-kind-vm-workloads/spike/findings-codec-ab.md` — this
  distilled result.

Authoritative artifact hashes:

```text
fe996dbeac3c9cebe921832ddaad11a1747b87268eb16be6ba607dd86a4fb84c  Cargo.toml
c331351034666fe44401032e6669710f863c2c46ef8f34a5dcc58d333987cb07  Cargo.lock
027a988916325345cf0e12597088b07731fcceb344ffdcbca0dba0846d91a613  capture.sh
5d52e6329e124bad04c2185a6d283294009844a31f59913bb020ea26fd680d60  run.sh
a5f4b71f3956807603a19a5f0b65b60fa13ae6e1733f1f0f60a3ae4c22618eca  rsync-without-local-env.sh
651ea4c2c06e45e1b8c2a8c59700c14ea72bd756c9224cde22c684a2a9e337b4  src/main.rs
419e0ed2de9ae8b29d9ca4538256a2ffbcaaaf677eab429a177bf154538dc445  runs/0003.meta
e0001f57a318f0b0d4e2db16f641f81d04ef9974ba49f49512bc42dd34538ff7  runs/0003.stdout
3befed09e1d26d58fc721c54f6fc65104c808dd7ff3bcd4451c678dd232d9e79  runs/0003.stderr
```

The `.meta` file independently records the stdout/stderr hashes. A scan for the
configured metal target and hostname across both increments and this feature's
spike documents passed; `.env` was excluded from rsync and is not stored.

## Uncertainty and limits

- Thin LTO, rustc/LLVM, codegen units, strip policy, dependency versions, and
  exact message shapes can change linked deltas. Repeat after any such change.
- The representative binary isolates incremental codec cost; it is not the
  production binary. Exact full-init deltas remain unknown until an accepted
  DESIGN permits a production-shaped integration experiment.
- The decode loop includes validation and owned deserialization/allocation. It
  does not claim zero-copy use and is not an end-to-end probe latency model.
- JSON evolution depends on an explicit unknown-field policy. `rkyv` evolution
  depends on an explicit version envelope plus retained version decoders.
- Only the production-relevant x86_64 musl target was measured. Aarch64 may
  produce different linked sizes even though both supported architectures are
  64-bit little-endian.
- No probe port, wire type, schema, limit, or public API in this harness is an
  accepted DESIGN decision.

## DESIGN gate recommendation

**Choose `rkyv` at the guided DESIGN gate if the declared priority remains
stripped init footprint first; require a pre-access version check plus
bytecheck and retain per-version decoders. Choose JSON only if the user values
additive schema tolerance and operational readability more than the measured
53,248-byte representative gap.**

## Promotion gate result

**DISCARD from production; retain as evidence.** The user selected `rkyv` for
DESIGN and required the same versioning approach used elsewhere in the
project. Accordingly, the accepted direction is the existing per-type rkyv
envelope discipline (historical `V<N>` payload variants, latest-only writes,
validated reads with up-conversion, and preserved golden bytes), not the
spike's standalone integer schema prefix. The harness remains throwaway
evidence and defines no production API or wire layout.
