# Slice 08 — Transparent mTLS: plaintext baseline against composed path, both directions

Story: US-BENCH-08. Job: J-PLAT-008. Family: `mtls-path`, native substrate.

## Goal
For each direction, a plaintext-baseline arm and a composed-mTLS arm with handshake, first-byte latency, throughput and tail latency as separate rows, and the difference with its interval and noise floor.

## In scope
- Egress (workload dials a peer) and ingress (peer dials the workload) as separate result sets; the plaintext baseline defined in the report.
- Benchmark client follows the production model: the egress dialer is plaintext.
- Interval and A/A floor per direction; catalogue row.

## Out of scope
CPU, memory and sockets (slice 09); wire capture (Tier-3); comparison with the in-guest kernel path (#303 consumes this program).

## Learning hypothesis
Disproves: that the plaintext baseline and the composed path can be driven in-process over the same endpoints. If not, DESIGN defines the baseline differently and surfaces it.

## Acceptance
- [ ] Both directions, each with a baseline arm.
- [ ] Handshake, first byte, throughput and tail separate.
- [ ] Baseline defined in the report.
- [ ] Interval and A/A floor per direction.

## Dependencies
Slices 01 to 03.

## Estimate
3 days. Reference class: `spike-scratch/` mTLS probes and slice 07.
