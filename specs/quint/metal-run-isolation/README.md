# Metal runner admission and completion protocol

Status: DESIGN proposal, P2 / ADR-0278 and P5 / ADR-0281; no user approval or implemented v2 runner is implied. [Exact owner/interface/projection contract](../../../docs/feature/metal-run-isolation/feature-delta.md#wave-design--ref-formal-protocol-model). This permanent model supplies the repository DESIGN gate, not a runtime subsystem.

`runner_protocol.qnt` contains two run slots, same/different checkout selection, one exclusive maintainer and the actual shared-barrier → checkout → prepare → one START → main exit → owner stop/finalizer → read-only collection ordering. Source transfer cannot change a locked checkout. Loss before START aborts; loss after START does not cancel the durable command. Missing supervisor terminal never becomes a command result. Cleanup failure or absent completion does not overwrite the command terminal and cannot produce successful collection. The owner operation `StopOwner` is the existing **blocking systemd stop boundary**, including manager drain/finalization before the unit becomes inactive. It does not model a new recovery daemon or detached cleanup task.

`runner_ok` disables model-only action recording for exhaustive checks; `runner_conf` records the last action for named-run conformance replay. Both instantiate the identical state machine with no disabled rule. `hazard/runner_hazards.qnt` removes one ADR-level rule each: P2's ordinary shared admission (shows an unrelated ordinary owner blocks admission), and P5's owner-completion collection gate (shows a command result escapes while descendants/roots remain). They are negative controls for these decisions, not extra witnesses or rejected-architecture experiments. P1's pure identity function, P3's kernel namespace construction and P4's static placement are assumed inputs, not additional shared-owner protocols or new modeled runtime mechanisms.

The finite representative statuses are command 0/255, runner preparation/admission 75, source/preflight 78, absent terminal -1 → collector 70, absent/failed completion → collector 74. The exact external 0–255 status domain and typed record validation remain the feature's schema obligations; the model's representative classes do not narrow that contract. Receipt 0 is absent, 1 complete, 2 cleanup failure; stop outcome 3 represents cleanup finishing but the final receipt not being published. One checkout revision bit distinguishes matching/mismatching caller input, without attempting to model source bytes or hashes.

Assumptions and validation boundaries:

| Assumption | Discharging evidence / required conformance |
|---|---|
| Kernel flock is shared-compatible, exclusive-incompatible, scoped to held descriptors | [Native increment A](../../../docs/feature/metal-run-isolation/spike/native-increment-a.log), run `459aeff4c46c7a3c302e60d3`; DISTILL replays admission against actual `lease-holder.sh` and kernel fdinfo, with same and distinct checkout inputs |
| systemd `KillMode=mixed` drains the unit descendants before the finalizer and stop returns | [Retained O actual-nextest-timeout construction](../../../docs/feature/metal-run-isolation/spike/native-increment-o.log), plus [B stop-order capture](../../../docs/feature/metal-run-isolation/spike/native-increment-b-retry.log), with VMM PIDs gone and scoped roots removed; native `StopOwner` conformance observes the same existing manager boundary |
| Correct full checkout binding, exact source marker and syntactically validated atomically published records | P1/P2/P5 exact schemas and owner paths in the feature; schema/conformance tests exercise refusal and no-sync mismatch. No hash collision or corrupt record is silently modeled as valid |
| Namespace/CID/listener isolation and correct test placement | [Exact pinned v7.0 sources](../../../docs/feature/metal-run-isolation/design/kernel-source-provenance.json), [paired real GuestKit G proof](../../../docs/feature/metal-run-isolation/spike/g-paired-result.json), [direct-native N fixture proof](../../../docs/feature/metal-run-isolation/spike/native-increment-n.log) and [F1 exact-test collision/placement correction](../../../docs/feature/metal-run-isolation/design/f1-disposition.md); these kernel facts are not established by Quint |
| Scheduler/storage/service operations eventually finish or return their specified failure, admission deadline eventually expires, and selected command eventually ends | Explicit weak fairness of enabled admission/prepare/start/main-exit/stop actions. There is no promise that an infinitely running command, hung kernel/storage or unfair scheduler terminates |
| No externally authorised physical maintenance or hostile record mutation bypasses canonical barrier/source locks | Existing administrative boundary, not an adversarial-host security model. Malformed record refusal is specified separately; no reboot/scavenger/recovery protocol is invented |

`runs.ctl`, `runs.checked`, `runs.ever`, `runs.starts`, `runs.desc`, `runs.roots`, `observed` and `conf_step` are explicitly model-only control-flow/history/resource-obligation facts. The driver must **not** project them from its own history or invent production accessors. `runs.roots` conservatively marks an obligation from checkout admission, not a claim that directories already exist before preparation; native conformance directly checks all three actual roots after stop. All actual projected fields and actions have exact external owner accessors/driving ports in the feature contract. In particular, stopped-unit process/root complements are directly inspected at the native owner boundary; no driver-owned ledger is accepted as evidence of cleanup.

## Model-only replay record

Named `quint test` traces omit `mbt::actionTaken` and `mbt::nondetPicks`. The existing `conf_step` therefore records the action that produced each state as the ordinary record `{ tag: str, value: { actor: int, value: int } }`. Installed `quint-connect` 0.1.2 accepts that record directly through its sum-type-shaped decoder: `tag` supplies `Step::action_taken`, and the inner `value` record supplies the two named picks. This encoding does not require another Quint module or a new action/type variant.

The actual ITF variable name is one qualified key. The driver uses:

```rust
Config {
    state: &[],
    nondet: &["runner_conf::runner_protocol::conf_step"],
}
```

The static `nondet` path must select the whole `conf_step` record. A handler-side rename cannot repair decoding because the runner constructs `Step` before it calls `Driver::step`. The driver's production-derived state projection continues to exclude this model-only metadata and the other model-only variables identified above.

| Recorded tag | `actor` pick | `value` pick |
|---|---|---|
| `init` | 0 | `initFor(c)` checkout choice `c` |
| `Register`, `Barrier`, `Checkout`, `Prepare`, `Start`, `LoseController`, `AdmissionTimeout`, `SupervisorLost`, `Collect`, `NoSyncRefusal` | invocation `r` | 0 |
| `AbortPreparation`, `MainExit` | invocation `r` | command's existing `code` argument |
| `StopOwner` | invocation `r` | existing `outcome` argument |
| `Sync` | checkout `c` | 1 |
| `Maintain` | 0 | existing `op` argument |
| `Idle` | 0 | 0 |

`REC = true` writes `Idle` explicitly, so an idle transition cannot replay its predecessor. Every `REC = false` instance keeps `conf_step` constant after initialization, preserving the exhaustive-check state space and fairness tuple. No protocol guard, state transition, invariant, temporal property, named run, manifest entry or hazard changes with this encoding correction. [Bounded actual-trace and public-runner codec evidence](../../../docs/feature/metal-run-isolation/design/codec-compatibility/findings.md) verifies the original decode failure, all thirteen existing named traces and seeded random traces including `Idle`. The diagnostic driver deliberately disables state comparison; it is codec evidence, not Metal owner conformance.

Run the installed repository commands with Quint on PATH:

```text
cargo xtask quint typecheck
cargo xtask quint check --subsystem metal-run-isolation --record
cargo xtask quint verify-evidence --subsystem metal-run-isolation
```

`checks.toml` owns the exhaustive safety, weak-fair progress, two expected hazard violations and one evaluator entry per named scenario. Tool-written `evidence/summary.md` and `summary.json` give actual versions, bounds, hashes and outcomes; do not hand-edit them. All native construction captures predate the future v2 implementation and are not conformance-pass claims for it. DISTILL authors the actual conformance driver against the pinned shell/systemd/kernel owner boundary; no product Rust public API or Sim seam is required.
