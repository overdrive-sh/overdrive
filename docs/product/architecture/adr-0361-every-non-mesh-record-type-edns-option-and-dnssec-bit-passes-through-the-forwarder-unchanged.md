# ADR-0361 — Every non-mesh record type, EDNS option and DNSSEC bit passes through the forwarder unchanged

## Status

**Accepted** (GH #382, DESIGN decision GPD-6; approved by the user 2026-11-25).
Evidence: `docs/feature/guest-public-dns-resolution/spike/findings-design-validation.md` (V-3). Interface contract: `docs/feature/guest-public-dns-resolution/feature-delta.md` § *[REF] Interface contracts*.

## Context

For mesh names the responder answers NODATA to AAAA because the mesh is IPv4 (ADR-0072). The guest's egress is IPv4 only (ADR-0162); guest IPv6 transport is #308. Public names may carry AAAA, CNAME chains, SVCB/HTTPS and DNSSEC records, and guests may set the DO or CD bits or advertise a UDP size.

## Decision

- The forwarder sends the guest's query bytes unchanged and relays the upstream reply unchanged, except for the owned-range replacement of ADR-0359. AAAA, CNAME chains, RRSIG and the AD and TC bits are not interpreted or rewritten. The responder neither sets nor clears AD.
- The reply size is whatever the upstream returns for the guest's advertised size, bounded by one fixed receive buffer of the guest transport's datagram payload maximum; a larger reply fails that attempt. The responder does not read or apply the guest's EDNS size itself.
- No AAAA-to-NODATA synthesis for non-mesh names.

## Alternatives considered

- **NODATA for AAAA on non-mesh names.** Deterministic and matches the mesh arm, but misreports names that have AAAA and breaks DNSSEC proofs. Not needed: an IPv4-only guest does not stall on AAAA (V-3).
- **Strip DNSSEC records.** Breaks validating guests. Rejected.
- **Enforce the guest's advertised UDP size in the responder.** Duplicates the truncation the upstream already applies to the same query bytes, and needs EDNS parsing for no reachable failure. Rejected.

## Consequences

- Positive: one code path for all types; no synthesised answers; validating guests work.
- Negative: AAAA answers reach a guest that cannot use them; the guest's resolver libraries try IPv4 first, so the unusable AAAA answers cost no connect delay (V-3, which did not measure an IPv6 connect alone).
