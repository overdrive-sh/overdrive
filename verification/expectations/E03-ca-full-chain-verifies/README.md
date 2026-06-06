# E03 — The full Root → Intermediate → SVID chain verifies under `openssl verify`

**Surface:** E (end-to-end) · **KPI:** K1 · **Status:** `pending`

## Expectation

A workload SVID minted by the platform's built-in CA chain-verifies through
the full three-tier hierarchy with a **standard external tool**, independent
of the platform's own word:

```
openssl verify -CAfile root.pem -untrusted intermediate.pem svid.pem   →  exit 0  ("svid.pem: OK")
```

The root is a self-signed P-256 CA (`CA:TRUE`, keyCertSign|cRLSign); the node
intermediate is signed by the root with `pathLenConstraint=0`; the workload
SVID is a leaf carrying exactly one `spiffe://overdrive.local/job/<name>/alloc/<id>`
URI SAN, `CA:FALSE`, keyUsage=digitalSignature (critical), ~1h validity. This
is the headline walking-skeleton proof — the genuine user-observable outcome
for Sam the security engineer, who verifies chains with `openssl` rather than
trusting the platform.

**No operator CLI verb mints an SVID this phase** (feature-delta D-CA-4): SVID
issuance is an internal platform mechanism triggered when the platform runs a
workload. `openssl verify` over the minted material is the honest external
entry point.

- Anchor: S-04-07 (`rcgen_full_svid_chain_verifies_root_intermediate_svid`, the `@walking_skeleton` scenario)
- Anchor: ADR-0063 D1 (`Ca` trait 3-tier hierarchy)
- Anchor: docs/feature/built-in-ca/feature-delta.md § Outcome KPIs — K1 (North Star: % of issued SVIDs that chain-verify to the root)

## Verification

Precondition: the host CA adapter (`RcgenCa`, real `ring`/rcgen crypto) can
generate a root, issue an intermediate, and mint an SVID. In DELIVER this is
exercised by the gated `integration-tests` test
`rcgen_ca_chain_verify.rs::rcgen_full_svid_chain_verifies_root_intermediate_svid`
(run via Lima). This expectation captures the **operator/reviewer-observable**
proof: the three PEMs exported and verified by `openssl` as an external tool.

Sub-claims:

1. `openssl verify -CAfile root.pem -untrusted intermediate.pem svid.pem`
   exits 0 with `svid.pem: OK`.
2. (Profile) `openssl x509 -in svid.pem -noout -text` shows exactly one
   `URI:spiffe://overdrive.local/job/.../alloc/...` SAN, `CA:FALSE`, and
   `Digital Signature` keyUsage marked critical.
3. (Negative anchor, from S-03-05) a chain in which the pathLen=0 intermediate
   signs a *further CA* fails `openssl verify` (pathLen enforced, not merely
   set).

`satisfied` requires sub-claims 1–3 on a Lima run, reviewed adversarially for
"did `openssl` actually exit 0, or did the runner narrate it?" (the different-fox
audit reads only the captured `evidence/`).

## Evidence

Captured under `evidence/` by `harness/run-expectation.sh E03`. Not yet run —
the `Ca`/`RcgenCa` production surface lands in DELIVER. Until then the runner
records `pending` and prints the manual capture steps.
