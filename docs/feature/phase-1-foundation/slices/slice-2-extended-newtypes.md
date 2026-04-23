# Slice 2 — Extended Identifier Newtypes

**Story**: US-02
**Estimated effort**: ≤ 1 day
**Walking-skeleton member**: yes (backbone row 1, remainder)

## Hypothesis

If `SpiffeId` is a `String` alias with a `normalize_spiffe_id` helper, the typed-identity claim in whitepaper §8 is not code, it's convention — and subsystem crates will each invent their own string-parsing, producing eight subtly-different bugs.

## What ships

- `SpiffeId`, `CorrelationKey`, `InvestigationId`, `PolicyId`, `CertSerial`, `Region`, `ContentHash`, `SchematicId` newtypes in `overdrive-core`
- Each meets the full US-01 completeness contract
- `SpiffeId` exposes structured accessors for trust domain and path — no string-splitting at call sites
- `Region` is case-insensitive on parse, lowercase on `Display`
- `ContentHash` is fixed-length (32 bytes) with hex `Display`
- `SchematicId`: content-hash over a *documented* canonical form (rkyv-archived or JCS JSON)

## Demonstrable end-to-end value

`cargo test -p overdrive-core --test newtypes_extended` green. `static_api_inspection` test confirms zero `String`-as-identifier.

## Carpaccio taste tests

- **Real data**: whitepaper §8 canonical SPIFFE URIs, realistic region codes (`eu-west-1`), real SHA-256 bytes.
- **Ships end-to-end**: downstream crates can stop inventing their own parsers today.
- **Independent of Slice 1 impl work**: once Slice 1 establishes the pattern, this runs in parallel.

## Definition of Done (slice level)

- [ ] Eight new newtypes implemented with the full contract.
- [ ] `SchematicId` canonicalisation documented in a module-level rustdoc.
- [ ] Static API inspection test passes (0 `String` identifiers).
- [ ] proptest round-trip per newtype.
