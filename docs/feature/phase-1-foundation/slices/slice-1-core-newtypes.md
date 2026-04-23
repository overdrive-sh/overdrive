# Slice 1 — Core Identifier Newtypes

**Story**: US-01
**Estimated effort**: ≤ 1 day
**Walking-skeleton member**: yes (backbone row 1)

## Hypothesis

If `FromStr → Display → FromStr` is not lossless for *any* identifier, our newtype contract is broken and every downstream content-hash and DST reproduction inherits the bug.

## What ships

- `JobId`, `NodeId`, `AllocationId` newtypes in `overdrive-core`
- Each with: validating constructor returning `Result`, `FromStr` (case-insensitive where applicable), `Display`, `Serialize`, `Deserialize`, `PartialEq`, `Eq`, `Hash`, `Debug`, `Clone`
- proptest round-trip per newtype
- Zero `normalize_*` helpers; validation lives in the constructor

## Demonstrable end-to-end value

`cargo test -p overdrive-core --test newtypes` runs green. Any downstream crate can depend on `overdrive-core` and use typed identifiers instead of `String`.

## Carpaccio taste tests

- **Real data, not synthetic**: identifiers derived from actual whitepaper examples (`job/payments`, `spiffe://overdrive.local/...`).
- **Ships end-to-end**: a library crate other crates can depend on today, not just a trait.
- **No promise deferred**: newtype contract is complete on delivery; nothing bolted on later.

## Definition of Done (slice level)

- [ ] Three newtypes implemented with the full contract.
- [ ] Proptest round-trip passing for each.
- [ ] No `String` identifiers in `overdrive-core` public API.
- [ ] Static inspection test enforces the above.
