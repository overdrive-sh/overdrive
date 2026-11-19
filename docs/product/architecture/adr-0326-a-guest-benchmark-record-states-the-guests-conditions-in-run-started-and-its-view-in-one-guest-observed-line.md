# ADR-0326 — A guest benchmark's record states the guest's conditions in `run_started` and its view in one `guest_observed` line

## Status

**Accepted (2026-11-19).** Approved by the user as PD-2 (artifact digests recorded, not refused on; the guest view comes from the replicates, PD-4). Extends [ADR-0301](adr-0301-a-benchmark-result-is-an-append-only-record-with-a-derived-report.md) (operative: records exist). DDD-2 in the [feature delta](../../feature/guest-benchmark-substrate/feature-delta.md).

## Context

A guest cannot see turbo, SMT or frequency, so its settings cannot stand in for the host's. The record must show the real host's settings and the guest's view beside them (D6), the guest's vCPU count and scheduling (D7) and that the guest was not nested (D5), without a second schema and without invalidating any committed native record. The guest also runs artifacts the benchmark form does not rebuild: `cargo xtask guest-kit-prepare` stages `overdrive-init`, `overdrive-guest-peer` and the `ovd_fwd` module, so the commit a record states is not by itself the identity of what ran. What the guest saw is known only after a replicate's guest has booted (ADR-0328), and `run_started` is the first line of an append-only record, written before any replicate. ADR-0301's `host` block refuses unknown keys and `run_started` refuses unknown fields; the native benchmark program's record contract (its feature delta, DDD-4, `docs/feature/native-benchmark-program/feature-delta.md:735`) states that no field is ever omitted to mean `null`, with `run_refused`'s `found`, `pinned` and `reason` as the only optional fields.

## Decision

`host` is always the real Metal host's block. Two things carry the guest:

1. `run_started` carries one optional object, `guest`, present if and only if `substrate` is `guest`. It holds what is true before any guest boots: the launch, the boot and the vCPU scheduling, as closed vocabularies with one value each, and the guest's settings, each stated as not visible from a guest.
2. One `guest_observed` line holds the guest's view: the kernel and clocksource (read inside the replicate's guest), the vCPU count and memory size, and the digest of each staged artifact the kit copies into the guest image. The vCPU count and memory size are configuration, not observation: they are the shape the guest kit booted the guest in, and the family reports them with the view because the benchmark core has no kit dependency to read them from (#369 tracks declaring another shape). The digests are computed once per run by the family's host half from the staged files, not in the guest. Each observed value takes the `host` block's three forms (text, absent with reason, unreadable with error). The exact keys and values are DDD-2. The line appears at most once, after the start canary, and only in a record whose `run_started` carries `guest`. The runner writes it when the first replicate reports a view; later reports are not compared (ADR-0328). A guest record in which no replicate reported a view has none, and so states no vCPU count or shape (feature delta CA-4).

`schema` stays 1: the record only widens what is accepted, and the writer and validator ship together. A native record has no `guest` key and no `guest_observed` line and is byte-identical to a record written before this ADR. A reader decides from the digests whether the staged artifacts match a fresh `guest-kit-prepare` at the record's commit; the run does not refuse on them.

## Alternatives

- **`guest: null` on native records**: every committed native record would fail validation or need rewriting (K4). Rejected.
- **A second record schema or a `guest_record.jsonl`**: contradicts D4/K1 (no guest-only schema or validator). Rejected.
- **The guest's readings under new keys of `host`**: the `host` block means the real host; mixing the two is the misreading D6 forbids. Rejected.
- **Free-form `launch`, `boot`, `vcpu_scheduling` strings**: an unvalidated string cannot be read by a machine; the single-value vocabularies grow additively ([#368](https://github.com/overdrive-sh/overdrive/issues/368) adds `pinned`). Rejected.
- **The whole guest block in `run_started`, filled by a probe guest before it is written**: needs a guest boot before the record starts (ADR-0328). Rejected.
- **No artifact identity (the commit alone)**: a stale staged artifact is then recorded under the current commit and no reader can tell. Rejected.
- **Refuse when the staged artifacts differ from the commit**: needs a stamp `guest-kit-prepare` writes and a rule for a dirty checkout, and still misses an edit after the stamp; it changes the setup step for a check the digests make unnecessary for a reader. Rejected.

## Consequences

The native record contract's "no field is ever omitted" sentence gains `run_started.guest` as a second optional place (feature delta CA-2), and the record gains one line kind that only guest records carry. A validator built before this ADR refuses a guest record (it names the unknown substrate, field or line kind). The report adds guest lines only for a record that has a `guest` block, so every native report is byte-identical; a guest report without a `guest_observed` line prints each observed value as `absent: no replicate returned a guest view`. Computing the digests uses `sha2` (a workspace dependency) in the family's host half.

## Links

- Exact shape: [feature delta, DDD-2](../../feature/guest-benchmark-substrate/feature-delta.md)
- [ADR-0328](adr-0328-a-guest-benchmark-refuses-the-host-before-it-boots-a-guest-and-takes-the-guests-view-from-its-replicates.md)
