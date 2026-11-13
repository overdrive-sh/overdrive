# Independent DISTILL review — final D11 conformance mapping alignment

## Metadata and verdict

| Item | Value |
|---|---|
| Feature | `service-allocation-readiness-and-ending` |
| Reviewed commit | `a793389a4f7e967fbc12ba4c568509d3f46a35db` |
| Authority | Accepted D11 and D11-E1 in `../feature-delta.md`; ADR-0263; D11 conformance and S-40 obligations; repository testing and interface-ownership rules |
| Remediation record | `final-d11-mapping-alignment.md` |
| Review iteration | 1 |
| Verdict | **APPROVED** for the bounded D11 test/model mapping correction |

## Production boundary and original failure

`build_service_stream` yields `Accepted` at `streaming.rs:1088-1096`, creates the lifecycle-bus receiver on the next poll at `:1098-1101`, then awaits `ObservationStore::alloc_status_rows` at `:1103-1110`. Its wait loop continues past nonprojecting probe stops at `:1125-1173`; one external poll can therefore consume the queued restartable stop and a later projecting give-up. The old conformance driver treated the `Accepted` yield as the subscription and one `Deliver` as one consumed event. The retained `0x43d4164d` mismatch at `Deliver(0)` follows from those two false test mappings. The accepted D11 order is still `Accepted` → subscribe → read → wait if no verdict; receipt of `Accepted` was never a public promise that the receiver already existed. The feature-delta's conformance-projection table at `:1634` still says that `Accepted` marks the subscribe point. That parenthetical describes the superseded private test mapping; it does not alter D11 item 1 or authorize a product-order change. The model and README now describe the actual test boundary. The prior review's R-4 publication-time verdict remains intact.

The new `SnapshotReadGate` is a test-local implementation of the existing `ObservationStore` port. Its first-read mode waits *before* delegating the scan (`stream_snapshot.rs:200-211`). `subscribe_at_snapshot` first consumes `Accepted`, then polls the real `build_service_stream` until that port is entered (`:417-443`). Because the production call to `bus.subscribe()` precedes the port call, the model's `Subscribe` action now reaches the actual subscribed-before-read interval. `Read` releases the gate and observes the same production stream (`:446-453`). No production source, public type, port method, or wire surface changed.

## Model and oracle audit

| Contract or risk | Review evidence and disposition |
|---|---|
| FIFO within one logical poll | `stream_snapshot.qnt:95-120,145-152` drains the queued FIFO through the first projecting terminal on `Read` or `Deliver`. The model permits only attempts 0 and 1 and one `Publish` per attempt (`:63,137-143`), so the two-event drain covers its entire possible queue. The driver starts at the declared oldest event and polls boundedly through Tokio cooperative `Pending` (`stream_snapshot.rs:364-407,524-537`); a missing expected verdict still produces a state mismatch. **Original seed fixed without skipping it.** |
| Subscribe-before-read counterfactual | The focused S-40 test uses the gate's after-snapshot mode: `SimObservationStore::alloc_status_rows` has returned the empty snapshot before it pauses (`stream_snapshot.rs:200-211`), then the test commits and publishes `Stable`, releases the read and requires the live `Stable` line and stream end (`:734-757`). With read-before-subscribe, this publication would be absent from both the captured snapshot and the receiver. This is an independent outcome assertion through the real stream and bus, not a copy of the stream algorithm. |
| Recorded and live verdicts | The driver still projects the yielded line against the closing `AllocStatusRow` read through `ObservationStore::alloc_status_row`, and rejects a mismatched kind or unknown closing row (`stream_snapshot.rs:660-716`). `ObservationRead` is accepted only with the injected `Unreachable { peer: "peer-b" }` cause (`:373-385`); S-42 retains the other approved typed-cause cases. No oracle or typed error was generalized into a string or generic carrier. |
| Normative model diff | The four invariant definitions and `Safety` are unchanged (`stream_snapshot.qnt:193-217`); the read-before-subscribe hazard still expects a `NoMissedTerminal` violation (`evidence/summary.md:70-80`). The complete model diff changes poll granularity and moves attempt 1's `Commit`/`Publish` after the relist in the named `publish_after_relist_reaches_channel` run (`stream_snapshot.qnt:257-268`). It removes no `zero`, `never`, `must`, `exactly`, `fail-closed`, `forbidden`, or `required` outcome and relaxes no operator contract. The run still observes no verdict for two restartable stops. |
| Schedule and scope | The new test scripts rows and events through the existing observation port and lifecycle bus at a real stream poll boundary. The named run uses an event published after the relist it claims to test. No production API or seam was added, no new owner path was invented, and no D11 test identity or seed was removed. The gate is test-local support. The new test has `/// CONTRACT_SHAPE: bounded-change.` (`stream_snapshot.rs:734-739`) and `host-pure` placement; all seven named conformance tests and the randomized test retain their per-test declarations (`:759-858`). |

The current production stream and the reviewed D11 driver, model, and placement paths are byte-identical to the reviewed commit. `git diff` also confirms `streaming.rs` was unchanged between the isolated verification base `e4635dd3fb536ba71d3034ce180ee7e4f3475f5b` and the reviewed commit. Thus the isolated focused results apply to this D11 source; they do not establish a result for unrelated later source or the broad affected selection.

## Verification and limits

| Evidence | Result and limit |
|---|---|
| Focused Metal conformance selection, `QUINT_SEED=0x43d4164d` | Recorded run `6b6bdfa02d0cf43163830368`: **9/9 passed**, including seven named runs, 100 randomized traces, and the focused subscription counterfactual. This is the original failing seed. |
| Same selection, `QUINT_SEED=0xd1acbdab` | Recorded run `2d7a4fe728f861e7b7012442`: **9/9 passed**. |
| Randomized selection, `QUINT_SEED=0x94ef88cb` | Recorded run `2582db1ad456c062611651a1`: **1/1 passed** after the bounded physical-poll correction. |
| Canonical formal and static gates | Recorded `quint check --record` **84/84**, `verify-evidence` pass, all **53** Quint files typecheck, conformance lint pass, strict control-plane Clippy pass, doctests pass, and formatting pass. The checked-in summary lists all 12 `ss-` checks as passing, including the read-before-subscribe violation. I independently checked the committed diff for whitespace errors and audited the complete model diff; I did not rerun Metal in this review. |
| Broad affected-test hook | **Not green and not claimed green.** The ordinary `kmod-load --busy-ok` command was rejected before Nextest; a direct broad attempt without the prepared guest lane returned `lane_not_prepared`. The final integration owner is preparing that lane and owns its scoped acceptance receipts. These environmental outcomes do not change this bounded D11 conformance verdict. |

The evidence was captured from an isolated sparse worktree based on `e4635dd3fb536ba71d3034ce180ee7e4f3475f5b`, with the D11 edits and the already committed cfg fix needed for lint. The formal summary was regenerated for all 84 checks because the previous record lacked per-check input keys; this is a metadata and evidence refresh, not a claim that all other models changed. The user waived mutation testing; no mutation or D17 witness gate applies. The reviewed commit retains Marcus as author and exactly the required Codex co-author trailer.

## Findings and disposition

| Finding | Evidence | Disposition |
|---|---|---|
| None blocking | The reproduced failure was a driver/model mapping mismatch; the remedy reaches the real subscription and read boundaries, retains the accepted D11 invariants and typed error oracle, and passes the retained seeds. | No code or model remediation requested. The separate final integration gate remains with its owner. |
| Informational documentation alignment | `feature-delta.md:1634` retains the obsolete `Accepted`-as-subscribe parenthetical in its conformance-projection table, while D11 item 1 and production establish the real ordering. | Recorded here as a superseded private mapping note; it is not a public-interface ambiguity or a blocker for this bounded DISTILL correction. A later documentation reconciliation can remove that stale parenthetical without changing D11 behavior. |

**Final verdict: APPROVED — iteration 1.** This reviewer changed only this Markdown review artifact and did not edit code, model, evidence, or commit it.
