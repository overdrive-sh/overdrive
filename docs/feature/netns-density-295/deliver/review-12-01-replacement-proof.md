# STEP 12-01 replacement-proof review

## Review metadata

| Field | Value |
|---|---|
| Feature / step | `netns-density-295` / `12-01` |
| Review date | 2026-10-27 |
| Reviewer | Fresh independent replacement-proof reviewer, `/root/review_12_01_replacement_proof` |
| Scope | Approved canonical DESIGN/DISTILL folds, current Quint inputs and recorded evidence, required exact-seed and unchanged conformance receipts, and the bounded roadmap validation gate |
| Formal record | `guest-flow-owner` run `2796366-1793121937.068`, exactly three jobs |
| Durable Metal run | `5ecc8ff499eb39ad2afdf3a4` |
| Evidence verification run | `065645e97a94e29acafe8cde` |
| Final verdict | **APPROVED — REPLACEMENT PROOF COMPLETE** |
| Open findings | 0 |

This is the independent replacement proof required before roadmap validation,
DES GREEN, or a commit. It is not the post-commit DELIVER implementation
review. A fresh DELIVER reviewer must still inspect the production diff after
the step commit. This review created no DES event and no commit.

The governing contract is `feature-delta.md`, the STEP 12-01 criteria and
pending validation gate in `deliver/roadmap.json`, the independently reviewed
`design-12-01-*` amendments and their `review-design-12-01-*` records, and the
complete `review-distill-12-01-alignment.md` source-review history through
iteration 98. `AGENTS.md`, `.claude/rules/design.md`,
`.claude/rules/testing.md`, `.claude/rules/development.md`, and
`.claude/rules/verification.md` govern correctness, API/error exactness,
native falsification, and evidence honesty. The formal review used the
installed `nw-formal-verification-tlaplus`, `nw-tlaplus-verification`,
`nw-roadmap-review-checks`, and `nw-sar-critique-dimensions` guidance subject
to those repository rules.

## Authority and canonical-contract reconciliation

The approved folds preserve their exact decision boundaries:

| Decision | Current canonical result | Replacement-proof assessment |
|---|---|---|
| `BFIFO1` | Only the bounded `owner_flows` beacon queue is FIFO; D-UP-19 remains unchanged elsewhere. | Exact; no action, run, check, API, or wire addition. |
| `PABORTERR1` | A matching payload-free pre-`Paired` guest `Abort` resolves as `GuestDialError::Unreachable { errno: ECONNRESET }`. | Exact typed representation; no new variant, payload, string carrier, or wire field. |
| `ACTPRIORITY1` | Quiesced precedes missing-session activation; the existing action inventory and retry budget remain. | Exact; current required seed is green. |
| `ADMISSIONREFUSAL1` | The global non-admitted-reader cap is 65,536, with accepted baseline 311,296 and exact required fd bound 376,832; backlog input `B` is unchanged. | Exact; no hidden per-guest expansion or outcome weakening. |
| `QUIESCENCEOPEN1` | `OpenFlow` excludes damaged C1 FlowListeners. | Exact enabledness correction; current `0x8703c2cd` replay is green. |
| `CHCGROUP55RUNNER1` | The existing GuestKit path runs exactly 55 unchanged cgroup bodies in three disposable guests. | Exact; 55 passed, zero failed, with no physical-host module/cgroup effects. |
| `RELEASEPROJ1` | Lease presence/generation is authoritative for QPROJ-2; only the exact serving-state-empty cleanup row is retained for QPROJ-3. | Exact; same-CID transfer and late finish remain generation-safe; no model input, API, wire, or production semantic expansion. |
| `CONTROLOPENRESET1` | Existing `HostReadOpen` composes the queued reset and existing `NoSession` cleanup only for the approved writer-down branch. | Exact; invalid-token, claimed, previously-live, and already-reading branches remain unchanged. |
| FwdPair handoff corrections | `strmHeld`, pending/held abort split, holder-free recovery close, and Trace-47 `TakeEnd` cleanup use the reviewed selectors and effects. | Exact; R5-53, R5-63, WATCH-ABORT, K-A4, U-5, and U-R1 remain distinct. |
| `RECROUND1` | Model-only `roundState` has exactly `outside`, `pending`, `keep`, and `restore`, and applies multi-member effects only at the round barrier. | Exact; QPROJ-6, production APIs, typed errors, driver handlers, actions, runs, hazards, and checks remain unchanged. |

The normative feature-delta diff was checked for removed or relaxed `zero`,
`never`, `must`, `exactly`, `fail-closed`, `forbidden`, and `required`
clauses. Each changed clause is replaced by the independently approved more
exact ownership, projection, or enabledness contract. No user-approved outcome
or invariant is weakened. The typed error gate is closed: every amended error
path uses an already approved public representation, and no unresolved
`SPECIFICATION_AMBIGUITY` remains.

## Formal evidence authenticity and integrity

The current record is the completed durable run, not an earlier partial or
failed attempt:

| Evidence fact | Verified result |
|---|---|
| Outer command | `cargo xtask quint check --subsystem guest-flow-owner --jobs 3 --record` under Metal lease/run `5ecc8ff499eb39ad2afdf3a4`. |
| Inner Quint run | `2796366-1793121937.068`, 2026-10-27T17:25:37.06758912Z through 2026-10-27T18:35:32.834023936Z. |
| Terminal | 279/279 `PASS`; outer terminal `exit_code=0 cause=command_returned`; adjacent `.exit` is `0`. |
| Durable collection | Terminal replay of `5ecc8ff499eb39ad2afdf3a4`; `exit_code=0 cause=command_returned`; `.collect.exit` is `0`. |
| Evidence verification | Metal run `065645e97a94e29acafe8cde`; one subsystem verified; `exit_code=0 cause=command_returned`; `.exit` is `0`. |
| Expected outcomes | 209 `holds`, 70 expected `violation`; all 279 have `pass = true` and `outcome == expect`; no cause or failure log. |
| Check identity | 279 unique check names. The current and previous record have identical names, mains, properties, property kinds, backends, bounds, expectations, and timeouts. |
| Input integrity | All 28 recorded input hashes match the current local bytes. |
| Trace integrity | 70 unique referenced counterexample traces and 140 retained trace artifacts; every reference resolves. |

Current authoritative hashes are:

| Artifact | SHA-256 |
|---|---|
| `evidence/summary.json` | `2123ffda061d93b1782284cba73e8cdb95a2cf9bd8f7f4cea8e6bd6b327f021c` |
| `evidence/summary.md` | `a01d90d3f52f786546e7c7b553fe5d38d823b0b440d074707cdc5cf1908cdca5` |
| `checks.toml` | `0712216a6a3318f7f9e4107c5f54a8ad497602fa16a1774f3a5ad816025e912b` |
| `quiescence.qnt` | `f4a86d4ad99d9da352c39caa1bca5a4406e981ae70101a97a23e008ab6c6dde7` |
| `fwd_pair.qnt` | `47e88a79a9c7ec75b6b0dca55f59c489dfcdc46ec8bc450c1422b172a8c20449` |

`quint-guest-flow-owner-record-jobs3-durable98-20261027.log`, its collection
log, and `quint-verify-evidence-durable98-20261027.log` are the raw receipts.
The earlier run `2580067-1793100798.247` remains preserved as a failed
271/279 receipt with eight compile-server OOM tool errors. The disconnected
and incomplete pre-durable runs and `packaging98` SSH exit 255 are likewise
historical diagnostics. None is cited as current proof.

## Hazard coverage and heap-packaging review

Iterations 97 and 98 split four two-instance FwdPair wrapper files into
one-instance compilation units. The complete `checks.toml` semantic diff is
only four existing `spec` paths. The check inventory stays 279, and every
name, main, invariant, TLC exhaustive bound, expected violation, timeout,
retry, CI flag, heap, and worker setting stays unchanged.

The eight checks that previously ended in compile-server OOM all now complete
in one attempt and find their unchanged expected violations:

| Check | Current wrapper | Property |
|---|---|---|
| `fwd-pair-hazard-consume-on-short-send` | `fwd_pair_hazards.qnt` | `UnitsConserved` |
| `fwd-pair-hazard-eof-as-fin-peer-loss` | `fwd_pair_hazards_10.qnt` | `NoEofOnPeerLoss` |
| `fwd-pair-hazard-check-then-hook` | `fwd_pair_hazards_2.qnt` | `AttachTimeEndClassifiedOnce` |
| `fwd-pair-hazard-ended-attach-graceful-close` | `fwd_pair_hazards_11.qnt` | `EndedHalfNeverEndsClean` |
| `fwd-pair-hazard-release-keeps-pairs` | `fwd_pair_hazards_3.qnt` | `ReleasedContextHoldsNoPair` |
| `fwd-pair-hazard-lossy-end-record` | `fwd_pair_hazards_12.qnt` | `OneEndRecordPerPair` |
| `fwd-pair-hazard-watch-abort-shuts-vh` | `fwd_pair_hazards_5.qnt` | `WatchAbortLeavesHolderSocket` |
| `fwd-pair-hazard-drop-without-abort` | `fwd_pair_hazards_13.qnt` | `NoLiveWatchAfterDrop` |

All FwdPair hazard wrappers now contain exactly one imported instance. No
hazard, witness, counterexample, or rejected behavior was filtered, removed,
relabelled, or converted to `holds`. The unchanged expected-violation count is
70, so the packaging repair retains the formal teeth rather than fabricating
a green record.

## Conformance, native, and mechanical receipts

| Gate | Current result | Receipt |
|---|---|---|
| Quint typecheck | 31/31 files, exit 0 | `quint-typecheck-after-fwd-hazard-packaging98-20261027.log/.exit` |
| Default Rust union | 2,124/2,124, exit 0, including all 46 owner-flow cells | `default-union2124-after-recround1-lint96-20261027.log/.exit` |
| FwdPair | 40/40 at `0xee26ec37`; refused-attach and watch/read corrections included | `fwd-pair-full40-seed-ee26ec37-after-watch-fixes-20261026.log/.receipt` |
| Owner flow | 46/46 | `owner-flows-full46-ab174c14-physical-launch-approved-20261026.log` and the later 2,124-test union |
| UDP slots | 13/13, exit 0 | `udp-slots-full13-after-exact-ticket-20261026-receipted.log/.exit` |
| Quiescence named set | 10/10 at `0x2355dc58`, exit 0 | `quiescence-full10-seed2355dc58-recround1-20261027.log/.exit` |
| Retained quiescence seeds | `0x82af4af4` and `0x94db98ed`, 1/1 each, exit 0 | `quiescence-seed82af4af4-recround1-20261027.*`; `quiescence-seed94db98ed-recround1-20261027.*` |
| Shared-network recovery / R5-58 | 13/13, exit 0 | `shared-network-recovery-full-module-after-raii83-20261026.log/.exit` |
| Flow quota | 8/8; randomized full-capacity sample 145.81 s, inside the approved 240 s exception | `quint-flow-quota-suite-final-current.log` |
| cgroup default | Exactly 55 passed, zero failed, three disposable guests, exit 0 | `cgroup55-final-exit-receipt-20261026.json/.log` |
| mTLS guest relay | 7/7, exit 0 | `mtls-relay-7-final-20261026.json/.log` |
| Build and lint | Workspace check, workspace Clippy, format, DST lint, and conformance lint all exit 0 | `workspace-check-final-after-recround1-20261027.*`; `workspace-clippy-final-after-recround1-20261027.*`; `fmt-check-after-recround1-20261027.*`; `dst-lint-after-recround1-20261027.*`; `conformance-lint-after-fwd-hazard-packaging98-20261027.*` |

No mutation testing was run; the repository requires it only at the final
DELIVER-wave gate. No new test, API, seam, lane, model action, scenario,
hazard, or architectural mechanism was added for this proof.

## Review iteration 1 — stale exact-seed receipts

The preserved receipt set initially left one blocking proof gap. The required
`0xea0ef1a4`, `0x3c09cb16`, `0xafd56990`, `0xf23a4188`, `0x6539f0dd`, and
`0x8703c2cd` passes predated later approved model/driver corrections. The
newer broad suites used other random seeds and therefore did not replay those
exact schedules. The only preserved `0x8703c2cd` records were explicit
failures before `QUIESCENCEOPEN1` and `RECROUND1`.

**Iteration 1 verdict: CHANGES_REQUESTED.** The original STEP 12-01 crafter
was asked to run only those six missing current-source replays through the
coordinated Metal lease, without editing source, tests, configuration, design,
roadmap, or evidence inputs.

## Review iteration 2 — exact-seed remediation

All six bounded replays used the current source and completed with one test
passed, zero failed, exit 0, and durable Metal terminal
`exit_code=0 cause=command_returned`:

| Required seed / target | Metal run | Receipt / SHA-256 |
|---|---|---|
| `0xea0ef1a4` / `pair_conf_randomized_traces` | `c88584c82038dd7f449bf386` | `replacement-proof-fwd-pair-ea0ef1a4-current-20261027.log/.exit`; `b3337130149e646e2544ff1a867df1536407277cd6dfc048ccae0eda649f5c98` |
| `0x3c09cb16` / accepted V_g abort | `d59ce57d7a55c9272d984466` | `replacement-proof-owner-flows-3c09cb16-current-20261027.log/.exit`; `ed494e91f282ce56cc372133ec0c338c2755b2ac9d6afe34fb8df9e735786dce` |
| `0xafd56990` / quiesce/admission | `cb8b8dbcf8ea2a291f84057a` | `replacement-proof-owner-flows-afd56990-current-20261027.log/.exit`; `afa1c136be077a03451500c74d26fdf7405ed4b581b5cecddf6feeeb4c8840bc` |
| `0xf23a4188` / owner randomized | `26280dd121f84b2cd6d1aa71` | `replacement-proof-owner-flows-f23a4188-current-20261027.log/.exit`; `a3e6a64cc8006f528ad667963265b995f67d3d8e96feff2318d56f9c617dfde9` |
| `0x6539f0dd` / owner randomized | `bad9acc597f63accc649370d` | `replacement-proof-owner-flows-6539f0dd-current-20261027.log/.exit`; `4c03940d5ed3007d4f6d9da8301202cef978fbd03b9c0cd246585557f3d32379` |
| `0x8703c2cd` / quiescence randomized | `0fba2d576e489e032967733b` | `replacement-proof-quiescence-8703c2cd-current-20261027.log/.exit`; `59ded27baaefac119287e40a9917debd432876ac3aa1a04b64f0dbbce6076609` |

The current formal input and evidence hashes remained byte-identical after
these read-only replays. The exact-seed finding is **CLOSED**.

## Native falsification and final disposition

No current `holds` check produced a counterexample. Every current conformance
target and every required retained failure seed passed, including the exact
schedule that previously exposed the C1 `OpenFlow` mismatch. The 70 current
violations are the configured hazard/witness counterexamples and retain the
expected rejected behaviors; they are not counterexamples against the
approved design. No native or reproduced evidence falsifies a DESIGN premise,
so the native falsification gate does not return the design or roadmap to
`pending`.

The replacement proof is complete. Roadmap validation may change only from
`pending` to `approved`, with this artifact as its reviewer basis and an
approval timestamp. No criterion, API/error contract, implementation scope,
DES event, or source file is authorized by that metadata update.

**APPROVED — REPLACEMENT PROOF COMPLETE.** STEP 12-01 is eligible for the
strict fresh-crafter RED → GREEN → COMMIT sequence. The mandatory fresh
post-commit DELIVER review remains outstanding.
