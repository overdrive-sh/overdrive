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

Executed through `harness/run-expectation.sh E03` at SHA `c5702a13` (working
tree dirty — the Slice ③ env-gated export + 3-check runner are captured in the
pinned `evidence/dirty-diff.patch`), `executed_in_lima: true`, **runner exit 0**.
The runner now enforces ALL THREE sub-claims over real exported PEM material;
the PEMs are produced as a side-effect of the gated
`rcgen_ca_chain_verify.rs` integration tests run in Lima with
`OD_E03_CA_DIR="$EVIDENCE_DIR/ca"` (the black-box producer step — `cargo
nextest` is invoked only to write the PEMs; the runner itself stays
bash + openssl + file-observation and links no `overdrive-*` crate).

What the captured `evidence/` shows:

- **Sub-claim 1 (S-OC-13) — positive chain verifies.**
  `openssl verify -CAfile root.pem -untrusted intermediate.pem svid.pem` →
  `evidence/chain_verify.out` = `…/positive/svid.pem: OK` (exit 0).
- **Sub-claim 2 (S-OC-14) — leaf profile.** `evidence/svid_text.out` shows
  exactly one `URI:spiffe://overdrive.local/job/payments/alloc/a1b2c3` SAN,
  `X509v3 Key Usage: critical` → `Digital Signature`, and NO basicConstraints
  extension (a cert with no basicConstraints is CA:FALSE by X.509 default —
  never a CA; the runner falsifies on `CA:TRUE`, which is absent).
- **Sub-claim 3 (S-OC-15, MANDATORY negative anchor) — pathLen=0 ENFORCED.**
  `evidence/pathlen_negative.out` = `error 25 at 2 depth lookup: path length
  constraint exceeded` / `…/negative/leaf.pem: verification failed` (exit 2).
  The chain is `root → intermediate(pathLen=0) → further-CA → leaf`; the
  end-entity leaf below the further-CA is load-bearing — `openssl` only counts
  the pathLen budget for CAs that sit as INTERMEDIATES on the path to an
  end-entity, so without a leaf below it the further-CA verifies directly and
  pathLen is never exercised. With the leaf present `openssl` rejects on
  `path length constraint exceeded`, the genuine proof pathLen is enforced and
  not merely set. (This corrected the pre-Slice-③ negative test, which failed
  on a *signature* mismatch — the wrong reason — because its rebuilt issuer DN
  omitted the per-node `CN=node-a` and never linked the chain.)

**Status candidate: evidence captured, all three sub-claims executed cleanly;
awaiting the different-fox Haiku audit** (the authoring agent does not
self-stamp `satisfied` per `.claude/rules/verification.md`). The headline
`Status:` line stays `pending` until the audit confirms — the auditor reads
only `evidence/` and MUST verify sub-claim 3's `path length constraint
exceeded` is present (E03 evidence missing the negative anchor is a mandatory
`refuted`).
