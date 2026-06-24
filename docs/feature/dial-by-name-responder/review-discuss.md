# DISCUSS Wave Review - dial-by-name-responder

**Reviewer**: Codex, applying `nw-product-owner-reviewer` / DISCUSS hard-gate criteria  
**Date**: 2026-06-24  
**Verdict**: **REVISIONS_NEEDED before full DESIGN handoff**  
**Scope**: `docs/feature/dial-by-name-responder/{intake.md,feature-delta.md,slices/*.md}`, `docs/product/journeys/dial-a-mesh-peer-by-name.yaml`, and `docs/product/jobs.yaml` product SSOT update for `J-MESH-001`.

The artifacts are strong enough to proceed with the **Slice 00 spike**. They are not yet clean enough for full responder DESIGN handoff because the DNS empty-answer contract is contradictory and the handoff gate currently says both "ready for DESIGN" and "do not design until the spike validates the core routing assumption."

---

## Verdict

**Not cleared for full `@nw-solution-architect` DESIGN handoff yet.**

**Cleared only for Slice 00 spike execution / spike design**: validate the one-listener-many-netns routing assumption, record `PROMOTE / DISCARD / PIVOT`, then rerun this DISCUSS review before designing the walking skeleton.

### Blocking Issues

#### 1. Empty-candidate DNS semantics are contradictory

**Severity**: blocking  
**Dimension**: clarity / testability / cross-artifact consistency

The artifacts correctly pin one DNS semantic at the top level: `NXDOMAIN` is reserved for an unknown name, while a known name with no IPv6 record gets `NODATA`.

Evidence:

- `docs/feature/dial-by-name-responder/feature-delta.md:120` says `AAAA` returns `NODATA` and "`NXDOMAIN` is reserved for an unknown name."
- `docs/feature/dial-by-name-responder/feature-delta.md:224` repeats that `AAAA` for a known IPv4-backed name is `NODATA`, not `NXDOMAIN`.

But the empty-backend story then allows a known service with no running backend to return "empty answer / NXDOMAIN" interchangeably:

- `docs/feature/dial-by-name-responder/feature-delta.md:274` titles US-DBN-4 as "honest empty/NXDOMAIN."
- `docs/feature/dial-by-name-responder/feature-delta.md:280`, `288`, `293`, `294`, `303`, `310`, and `315` all use "empty/NXDOMAIN" for no-running-backend cases.
- `docs/feature/dial-by-name-responder/slices/slice-03-empty-candidate-honesty.md:8`, `15`, `23`, `28`, and `41` repeat the same ambiguity.
- `docs/product/journeys/dial-a-mesh-peer-by-name.yaml:61` and `112` carry the ambiguity into the product-level journey.

This blocks acceptance design because `NOERROR/NODATA` and `NXDOMAIN` are different resolver contracts. They produce different cache behavior, different client error surfaces, and different executable acceptance tests.

**Recommendation**: choose one exact DNS contract and update every artifact to match. The cleanest shape appears to be:

- Known service name + A query + at least one running IPv4 backend: `NOERROR` with A records.
- Known service name + A query + zero running backends: `NOERROR/NODATA` or another explicitly named empty-answer contract.
- Known service name + AAAA query in v1: `NOERROR/NODATA`.
- Unknown service name: `NXDOMAIN`.

If the intended contract is instead `NXDOMAIN` for known-but-empty services, then line 120 must stop reserving `NXDOMAIN` only for unknown names, and the product journey must say why that resolver/cache behavior is acceptable.

#### 2. Handoff status conflicts with the spike-first gate

**Severity**: blocking  
**Dimension**: dependency tracking / priority validation

The feature correctly identifies a load-bearing unvalidated mechanism: one host-side listener must answer DNS sent to many per-netns gateway addresses, and there is no Tier-2 backstop.

Evidence:

- `docs/feature/dial-by-name-responder/feature-delta.md:130-139` says the mechanism must be validated before the walking skeleton.
- `docs/feature/dial-by-name-responder/feature-delta.md:366-368` says the skeleton cannot be designed until Slice 00 validates the assumption.
- `docs/feature/dial-by-name-responder/slices/slice-00-spike-one-listener-many-netns.md:3-5` says Slice 00 is blocking and runs before the walking skeleton.
- `docs/feature/dial-by-name-responder/slices/slice-01-walking-skeleton-one-name.md:49` depends on `Slice 00 PROMOTE`.

But the DoR section says the feature is substantially met for DESIGN handoff:

- `docs/feature/dial-by-name-responder/feature-delta.md:421-424` says "DoR substantially met for handoff to DESIGN."

That sends two incompatible instructions to the DESIGN wave: either design the responder now, or stop until the real-kernel spike validates the routing assumption.

**Recommendation**: rewrite the gate as: "DISCUSS approved to run/design Slice 00 only. Full responder DESIGN is blocked until Slice 00 records `PROMOTE`; if it records `PIVOT` or `DISCARD`, revise DISCUSS artifacts before continuing." Keep the spike as an in-feature dependency, but do not describe full DESIGN as ready until the spike result exists.

### High Issues

#### 3. Shared artifact tracking is summarized, not registry-grade

**Severity**: high  
**Dimension**: shared artifact tracking / horizontal integration

The product journey has a useful `shared_artifacts_summary`, but it is not a formal registry with source of truth, consumers, owner, risk, and validation per artifact.

Evidence:

- `docs/product/journeys/dial-a-mesh-peer-by-name.yaml:120-125` lists four shared artifact summaries.
- `docs/feature/dial-by-name-responder/feature-delta.md` has no dedicated shared-artifact registry section.
- There is no `docs/feature/dial-by-name-responder/discuss/shared-artifacts-registry.md`.

This matters because the feature depends on several high-risk values being single-source and byte-consistent: `ServiceBackendsResolve`, answered backend address, per-netns responder address, query name, demo command path, and EDD evidence capture.

**Recommendation**: add a registry-grade section or split file for at least:

- `service_backends_running_set`
- `answered_backend_addr`
- `responder_addr`
- `mesh_dns_name`
- `ping_pong_command_path`
- `edd_ping_pong_evidence`

Each should state source of truth, consumers, owner, integration risk, and validation.

#### 4. Conventional DISCUSS files are absent, so downstream traceability depends on compact-artifact tolerance

**Severity**: high if local tooling expects split files; otherwise medium  
**Dimension**: handoff completeness / traceability

Most existing DISCUSS waves in this repo use split artifacts under `docs/feature/<feature>/discuss/`: journey, user stories, story map, shared artifacts, outcome KPIs, DoR validation, and wave decisions. This feature instead uses one compact `feature-delta.md` plus slices.

Evidence:

- `docs/feature/dial-by-name-responder/feature-delta.md:3-6` explicitly declares a single DISCUSS narrative artifact.
- The feature has no `docs/feature/dial-by-name-responder/discuss/` directory.

The compact artifact is dense and mostly complete, but downstream agents or review commands may look for conventional paths and miss the requirements.

**Recommendation**: either document that `feature-delta.md` is the authoritative compact replacement for the split DISCUSS bundle, or materialize the standard split files before full handoff.

### Medium Issues

#### 5. "One source, two readers" contradicts the pinned "one source, three readers" contract

**Severity**: medium  
**Dimension**: terminology consistency / shared artifact consistency

`docs/feature/dial-by-name-responder/feature-delta.md:198` says the headless single-source example is "one source, two readers." The rest of the artifact correctly pins "one source, three readers" for outbound resolve, inbound install, and name answers, including `feature-delta.md:119`, `328`, `340`, and `395`.

**Recommendation**: change line 198 to "one source, three readers" or narrow the sentence explicitly to "two readers involved in this assertion."

#### 6. Draft labels should be removed or resolved before final handoff

**Severity**: medium  
**Dimension**: handoff readiness

The primary artifact and every slice brief still present themselves as drafts:

- `docs/feature/dial-by-name-responder/feature-delta.md:1-5` says `DRAFT`, "not committed, not final."
- Each slice brief begins with `DRAFT brief`.

This is acceptable during review, but final handoff should not force DESIGN to guess whether it is reading live requirements or reviewer-facing draft text.

**Recommendation**: after the blocking fixes, replace draft banners with reviewed status and link this review.

### Strengths

- The job split is well justified. `J-MESH-001` is distinct from `J-SEC-003` because reachability-by-name can fail before enforcement has a connection to protect.
- The artifacts preserve the pinned contracts well: headless, no VIP path, in-agent userspace responder, `ServiceBackendsResolve` as the single source, and IPv4-only v1.
- The spike-first shape is correct. It surfaces the real-kernel netns/routing risk before production responder design hardens around an unvalidated assumption.
- The ping-pong demo is a strong operator-surface proof: real `overdrive deploy` commands, bidirectional by-name calls, counters/dates, and mTLS evidence.
- Outcome KPIs are measurable and include useful baselines.

### Verification Performed

- Parsed YAML successfully:
  - `docs/product/jobs.yaml`
  - `docs/product/journeys/dial-a-mesh-peer-by-name.yaml`
- Verified GitHub issue references used as current dependencies/sources:
  - #243 `In-agent node-local name responder for dial-by-name (svc.overdrive.local)` - open
  - #227 `EDD harness: disposable full-system Lima VM...` - open
  - #75 `[5.9] Image Factory MVP...` - open
  - #61 `Private Service VIPs...` - open
  - #167 `VIP allocator...` - closed
  - #178 `Native east-west SPIFFE-ID resolution...` - closed
  - #241 `Path-A canonical workload address...` - closed
  - #242 `Intended-peer SVID pinning...` - open

### Handoff Conditions

Full DESIGN handoff is cleared when:

1. DNS empty-case semantics are made exact and consistent across feature delta, Slice 03, KPIs, and product journey.
2. The handoff gate is changed to approve only Slice 00 until the spike records `PROMOTE`.
3. Shared artifact tracking is made registry-grade, either in `feature-delta.md` or a conventional `shared-artifacts-registry.md`.
4. Draft labels are resolved after the above fixes.

---

## Resolution log — author response (2026-06-24, post-review revisions)

> Appended by the feature author after revising the artifacts. The reviewer's
> findings above are preserved **verbatim**; this section records how each was
> addressed. The artifacts now **postdate** this review — re-verify against
> current HEAD; the original line numbers have shifted.

| Finding | Status | Resolution |
|---|---|---|
| **Blocking #1** — empty-case DNS semantics contradictory | ✅ RESOLVED | Added one canonical contract — `feature-delta.md` § *The v1 DNS answer contract* (table): `A`+running → NOERROR/A; `AAAA`+running → NODATA; **0 running backends (declared-but-empty OR unknown — indistinguishable in v1, the responder reads only the running set) → NXDOMAIN**, short negative-TTL. Every `empty/NXDOMAIN` site (US-DBN-2/4 + KPIs, slice-03, journey, jobs.yaml J-MESH-001) now says NXDOMAIN; the line-120 "NXDOMAIN reserved for unknown name" clause was rewritten. |
| **Blocking #2** — handoff status vs spike-first gate | ✅ RESOLVED | Gate verdict rewritten: "DISCUSS approved to run/design **Slice 00 only**; full responder DESIGN BLOCKED until Slice 00 records PROMOTE (PIVOT/DISCARD → revise DISCUSS)." Header + all 4 slice banners flipped to "Reviewed — gated to Slice 00." No "ready for DESIGN" language remains. |
| **High #3** — shared-artifact tracking not registry-grade | ✅ RESOLVED | Added `feature-delta.md` § *Shared-artifact registry*: 6 artifacts (`service_backends_running_set`, `answered_backend_addr`, `responder_addr`, `mesh_dns_name`, `ping_pong_command_path`, `edd_ping_pong_evidence`) × source-of-truth / consumers / owner / integration-risk / validation. |
| **High #4** — conventional `discuss/` split files absent | ✅ RESOLVED (documented) | Header states `feature-delta.md` is the authoritative compact form mandated by the `nw-discuss` Outputs contract + `validate_feature_layout.py`; legacy split files are intentionally not produced (materializing them would violate the contract). |
| **Medium #5** — "two readers" vs "three readers" | ✅ RESOLVED | `feature-delta.md` headless-single-source example reworded to "two of the one-source / **three**-readers contract." The pre-existing `mtls_resolve.rs` code comment ("one source, two readers" — the shipped 2-reader state) is left untouched; out of DISCUSS scope. |
| **Medium #6** — draft banners | ✅ RESOLVED | feature-delta + all 4 slice banners now read "Reviewed (DISCUSS, 2026-06-24; gated to Slice 00)", linking this review. |

### Handoff conditions — status
1. DNS empty-case semantics exact + consistent — ✅ done (Blocking #1).
2. Gate approves Slice 00 only until spike PROMOTE — ✅ done (Blocking #2).
3. Shared-artifact tracking registry-grade — ✅ done (High #3).
4. Draft labels resolved — ✅ done (Medium #6).

**Net:** all four handoff conditions met. The artifacts remain gated to **Slice 00
(the spike)** *by design* — full responder DESIGN stays blocked until the spike
records PROMOTE (that is the intended state, not an open blocker). A different-fox
re-review of the revised artifacts is recommended before DESIGN.

**Author decision surfaced to the user:** the empty case was resolved to
**NXDOMAIN-for-any-0-running** (capability-honest — the v1 responder reads only the
running set, so declared-but-empty and unknown are indistinguishable).
**NODATA-for-declared-empty** would require DESIGN to add a declared-service view; it
is flagged as a future refinement, **not** v1. If the intended contract is the
latter, flip it before DESIGN.

