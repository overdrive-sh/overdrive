# DESIGN Decisions — transparent-mtls-host-socket (GH #26 folds #222)

**Agent**: Morgan (nw-solution-architect) · **Date**: 2026-06-12 · **Mode**:
formalize a user-LOCKED decision on complete empirical evidence · **Density**:
`lean` + `ask-intelligent` (Tier-1 `[REF]`) · **Rigor**: `.nwave/des-config.json`
inherit; `review_enabled: true` (see § Review below); mutation N/A (docs).

## The locked decision (designed, not relitigated)

**Fold #222 into #26. Build ONE universal "transparent mTLS via an agent-light L4
proxy" as THE enforcement mechanism for ALL workload kinds** (process/exec, WASM,
microVM, unikernel). Whitepaper §7's "one identity model, two enforcement
mechanisms" collapses to ONE. In-band kTLS-on-the-workload's-own-socket is
SUPERSEDED as v1 and retained as a tracked FUTURE OPTIMIZATION.

Recorded in **ADR-0069**. User-decided 2026-06-12 on 6 Tier-3 spikes + 3 research
docs (kernel 7.0, committed `353cdc52`). The mechanism is fully de-risked.

## Why (the evidence, one line each)

- **In-band lossless foreclosed 3 ways**: no `sk_msg` HOLD (`findings.md`);
  source-TX-bypass RST on redirecting the live socket (`findings-lossless-hybrid.md`
  + `sockmap-redirect-live-socket-liveness-research.md`); lossless capture
  structurally requires a proxy (`findings-userspace-relay.md`).
- **Proxy proven agent-light BOTH directions**: forward agent-IDLE sockmap-egress-
  redirect → kTLS-TX, 15/15 (`findings-egress-ktls-splice.md`); return agent-LIGHT
  zero-copy `splice` via `tls_sw_splice_read`, ~1/record (`findings-splice-return.md`).
- **Basic mechanism proven**: `sockops → rustls → kTLS`, `pidfd_getfd` handoff,
  SOCKMAP-before-`TCP_ULP` ordering, control records via `ktls::KtlsStream`
  (`findings.md`).

## What was produced

| Artifact | Path |
|---|---|
| Central ADR | `docs/product/architecture/adr-0069-transparent-mtls-universal-agent-light-l4-proxy.md` |
| Application Architecture section | `docs/product/architecture/brief.md` § "Transparent mTLS — universal agent-light L4 proxy extension" (+ ADR index row 0069 + changelog) |
| C4 diagrams (L1+L2+L3) | `docs/feature/transparent-mtls-host-socket/design/c4-diagrams.md` |
| Feature-delta DESIGN sections | `docs/feature/transparent-mtls-host-socket/feature-delta.md` § "Wave: DESIGN / [REF] …" |
| Whitepaper §7/§8 reshape | `docs/whitepaper.md` § 7 ("Transparent mTLS — one universal agent-light L4 proxy") |
| Upstream back-propagation | `docs/feature/transparent-mtls-host-socket/design/upstream-changes.md` |
| This summary | `docs/feature/transparent-mtls-host-socket/design/wave-decisions.md` |

## Key decisions (D-MTLS-1…11)

See the feature-delta § "Wave: DESIGN / [REF] Decisions Table" for the full table.
Highlights: D-MTLS-3 (NEW `MtlsEnforcement` port, `Dataplane` does not fit);
D-MTLS-4 (forward agent-idle sockmap-egress, return agent-light `splice`); D-MTLS-5
(leg B = plain kTLS-RX, NO psock); D-MTLS-10 (in-process agent — no separate
process, no gRPC/CSR; resolves the prior open item); D-MTLS-11 (Earned-Trust
`probe()` mandatory).

## Reuse Analysis verdict (hard gate)

3 REUSE-AS-IS · 4 EXTEND · 1 CREATE-NEW port (`MtlsEnforcement`) · 1 CREATE-NEW dep
(`ktls`) · 1 EXTEND-or-new-crate open (adapter home, OQ-2). Default-EXTEND honored.
Full table in `brief.md` § 6 / feature-delta § Reuse Analysis.

## Open questions / deferrals (blockers — architect created NO GH issues)

- **OQ-1** — pin the EXACT `MtlsEnforcement` signatures before the crafter dispatch
  (model fixed by ADR-0069; the connection-handle wire shape + error variants are
  NOT improvised). Orchestrator pins with the user.
- **OQ-2** — `HostMtlsEnforcement` home (`overdrive-host` EXTEND vs dedicated
  `overdrive-mtls-host` crate). Default EXTEND; DELIVER-pinnable.
- **DEFER-1** — in-band restart-survival future optimization → needs a
  product-owner-approved GH issue (none exists). Surfaced, NOT created.
- **DEFER-2** — fully agent-idle bidirectional kernel splice (kernel patch) → needs
  a GH issue if pursued. Surfaced, NOT created.
- **DEFER-3** — multi-node reachability → likely the existing #36; verify
  (`gh issue view 36 --comments`) before citing.

## J-SEC-003 back-propagation (flagged, NOT self-applied)

The DISCUSS job + slices 00–05 were authored on the in-band "agent fully out,
restart-survivable, kTLS on the workload's own socket" model. Those properties no
longer hold in v1. The enforcement topology is now proxy-shaped (2 sockets/conn;
agent-light return). Flagged for the product-owner in `design/upstream-changes.md`.
The architect does NOT edit `jobs.yaml` or the slice files.

## Density & triggers

`lean` + `ask-intelligent`. Tier-1 `[REF]` sections emitted. No Tier-2 auto-render.
This is a formalize-the-locked-decision dispatch — the heavy reasoning lives in the
6 spike findings + 3 research docs + ADR-0069; the wave records the decision and
the decomposition, not a fresh investigation.

## Review

`review_enabled: true`. A per-wave peer review (solution-architect-reviewer) is
**warranted but the value is bounded** here: the central decision is user-LOCKED on
exhaustive empirical evidence (not an architect bias-prone choice), and the primary
review risks the critique dimensions target (resume-driven dev, technology bias,
missing alternatives) are pre-empted — the ADR carries 4 alternatives with rejection
rationale, all OSS, all kernel-source-pinned. The HIGH-value review target is
**OQ-1** (the un-pinned `MtlsEnforcement` signature) — but that is precisely the
item deliberately deferred to the orchestrator+user (a signature must not be
architect-improvised). Recommendation: surface OQ-1/OQ-2/DEFER-1..3 to the user as
the gating step; a full reviewer pass is optional and lower-yield than pinning OQ-1.
