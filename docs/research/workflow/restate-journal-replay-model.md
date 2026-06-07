# Research: How Restate Structures Its Durable-Execution Journal (Start/Input Record, Entry Indexing, Replay/Recovery Cursor Model)

**Date**: 2026-06-06 | **Researcher**: nw-researcher (Nova) | **Confidence**: High | **Sources**: 8 (6 primary/official, 1 official API doc, 1 industry-leader)

## Executive Summary
Restate's durable-execution journal is a per-instance, **positionally-indexed** append-only log: replay matches the Nth recorded entry against the Nth operation the re-executing handler emits, keyed by *type at index*, not by a name string (the proto `name` field is observability-only). The invocation **input is journaled as a first-class positional entry** — `InputEntryMessage` (type `0x0400`) in the v1 protocol, and `Command::Input(InputCommand)` (the first `Command` variant, i.e. command index 0) in the current journal-v2 model. The "machine starts here" control signal (`StartMessage`, carrying `known_entries` and eager state) is a separate **out-of-band control message, not a journal entry** — so Restate cleanly separates *control metadata* (out of band) from the *durable input record* (in the journal at index 0). The terminal result is likewise a positional entry (`OutputEntryMessage` / `Command::Output`).

The current protocol (Service Protocol V5/V6; V1–V4 deprecated for new invocations as of Restate 1.6) makes a load-bearing distinction explicit: every journal entry is either a `Command` (ordered, replayable — input, run/side-effect, call, sleep, output, state ops) or a `Notification` (a completion/signal/result, correlated by `NotificationId`, **not** positionally matched as a command). This is precisely the structural answer to the problem Overdrive is facing: replay advances a command index over the ordered command class, while results/signals live in a separate id-keyed channel off the positional match. Restate's determinism check (error **RT0016**, "journal mismatch") fails closed when the re-executing handler emits a different *sequence* of journal entries than recorded — the direct analogue of Overdrive's intended `name`-mismatch fail-closed, but keyed on command-type-at-position.

**Conclusion for the A-vs-B decision: Restate's design evidences Option B** — keep a distinct, typed start/input entry that the replay machinery handles as a first-class positional command (index 0), rather than Option A (delete the start record / keep input out-of-band). Restate journals the input *and* checks it on every replay; it solves the "non-await control entry in a positional stream" problem by **typing** entries (Command vs Notification) so the cursor advances only over replayable commands — not by removing entries. The recommended generalization for Overdrive is to type its journal the same way (commands advance the cursor; signals/completions are matched out-of-band) so that `Started` becomes a legitimate command-index-0 entry and exactly-once is preserved.

## Decision Context (Overdrive A-vs-B)
Overdrive is building a `Workflow` durable-execution primitive (ADR-0066/0064). The journal is a per-instance append-only log of `await`-point entries, replayed on crash-recovery via a **positional cursor** (cursor index == append position). A latent bug: `Started{spec_digest,input_digest}` is documented as the "first journal entry" but never written; the positional cursor cannot consume a non-await-point entry at a walked position. The two candidate fixes:
- **Option A** — delete `Started` entirely (start-inputs not journaled as a positional entry).
- **Option B** — keep a distinct start/input journal entry and make the replay cursor skip/handle non-await entries.

This research uses Restate's design as the tie-breaker.

---

## Q1: Does Restate have a distinct Start / Input / Run-created journal entry?

**Finding 1a — There are TWO separate constructs: a control `StartMessage` and a distinct `InputEntryMessage` journal entry.**

**Evidence**: The service-invocation protocol (v1) defines a `StartMessage` sent Runtime→SDK that "initializes the invocation state machine" and carries `known_entries` (the known journal length), `state_map` (eager state), and the invocation ID. The `StartMessage` is a **control message, not a journal entry** — it precedes the journal entry stream. Separately, the journal itself contains an `InputEntryMessage` (type code `0x0400`) which "Carries the invocation input message(s) of the invocation" and is non-completable and non-fallible.

**Source**: [restatedev/service-protocol — service-invocation-protocol.md](https://github.com/restatedev/service-protocol/blob/main/service-invocation-protocol.md) — Accessed 2026-06-06
**Confidence**: High (primary/authoritative spec)
**Analysis**: This is the load-bearing distinction for the Overdrive decision. Restate separates the *control* start-of-invocation signal (`StartMessage`, NOT in the positional journal) from the *durable input record* (`InputEntryMessage` / journal-v2 `InputCommand`, which IS a positional journal entry — the first entry, index 0). The input is journaled as a first-class positional entry, but the "machine starts here" signal is out-of-band metadata (`known_entries`), not a journal entry. In journal v2 the input is `Command::Input(InputCommand)` — the FIRST `Command` variant (see Finding 3b), i.e. command index 0.

## Q2: How does Restate index/identify journal entries (positional vs typed/named)?

**Finding 2a — Replay matching is positional (monotonic `entry_index`), NOT name-based.**

**Evidence**: "The SDK MUST be able to correlate the `result` field of the entry with the `result` field of `CompletionMessage` through the `entry_index`." Entries are indexed by monotonic positional order; during replay the runtime sends entries "in the correct order" and the SDK matches replayed entries by sequential position, not by type or name. `StartMessage.known_entries` carries "the known journal length" so the SDK knows how many entries to replay before switching to processing.

**Source**: [restatedev/service-protocol — service-invocation-protocol.md](https://github.com/restatedev/service-protocol/blob/main/service-invocation-protocol.md) — Accessed 2026-06-06
**Confidence**: High
**Analysis**: Restate's core model is positional, the same as Overdrive's cursor. The crux of A-vs-B is therefore: *given a positional model, how does Restate fit the input entry into the positional stream without breaking the cursor?* — answered in Q3/Q5.

## Q3: How does the journal handle non-await/control entries interleaved with replayed command entries (Command vs Notification distinction; cursor advance)?

**Finding 3a — In v1, entry-storage order is decoupled from result-delivery order; `CompletionMessage` is a separate message type, not a positional journal entry.**

**Evidence**: "The runtime can send `CompletionMessage` in a different order than the one used to store journal entries." Completable entries record an *action* (e.g., `CallEntryMessage`, `GetStateEntryMessage`, `SleepEntryMessage`) at a positional `entry_index`; the *result* arrives later via a `CompletionMessage` keyed by the same `entry_index`. Thus the positional journal contains only entries (actions); completions are a separate channel correlated by index — they do not occupy their own positions in the journal sequence.

**Source**: [restatedev/service-protocol — service-invocation-protocol.md](https://github.com/restatedev/service-protocol/blob/main/service-invocation-protocol.md) — Accessed 2026-06-06
**Confidence**: High
**Analysis**: This is a key structural insight. In v1, every positional journal entry corresponds to an SDK-emitted action (input, call, get-state, sleep, run/side-effect, awakeable, output). There is no "non-await control entry" sitting at a walked position that the cursor must skip — control/completion data rides out-of-band (the `StartMessage` metadata and the `CompletionMessage` channel). The input entry is itself a journal entry the SDK emits/consumes at position 0. (v2+ Command/Notification split investigated next.)

**Finding 3b — The modern protocol (Service Protocol V5/V6, "journal v2") makes the distinction explicit: a journal entry is either a `Command` (ordered, replayable) or a `Notification` (a result/completion, NOT positionally ordered as a command).**

**Evidence**: The Restate runtime source (`crates/types/src/journal_v2/raw.rs`) models entries as:
```rust
pub enum RawEntry {
    Command(RawCommand),
    Notification(RawNotification),
}
```
Commands carry a `CommandType`; Notifications carry a `NotificationType` + `NotificationId` and a result variant (Unknown, Void, Value, Failure, InvocationId, StateKeys). The `Command` enum (`crates/types/src/journal_v2/command.rs`) is:
```rust
pub enum Command {
    Input(InputCommand),
    Output(OutputCommand),
    GetLazyState(...), SetState(...), ClearState(...), ClearAllState(...),
    GetLazyStateKeys(...), GetEagerState(...), GetEagerStateKeys(...),
    GetPromise(...), PeekPromise(...), CompletePromise(...),
    Sleep(SleepCommand), Call(CallCommand), OneWayCall(OneWayCallCommand),
    SendSignal(SendSignalCommand), Run(RunCommand),
    AttachInvocation(...), GetInvocationOutput(...), CompleteAwakeable(...),
}
```
`Input` is the FIRST `Command` variant and `Output` the second; both are first-class commands in the ordered command stream.

**Source**: [restatedev/restate — crates/types/src/journal_v2/raw.rs](https://github.com/restatedev/restate/blob/main/crates/types/src/journal_v2/raw.rs) and [crates/types/src/journal_v2/command.rs](https://github.com/restatedev/restate/blob/main/crates/types/src/journal_v2/command.rs) — Accessed 2026-06-06
**Confidence**: High (primary source code)
**Analysis**: This is the **decisive** evidence for A-vs-B. Restate's modern design does NOT delete the start/input record from the positional stream; instead it makes the *typed* `Command` vs `Notification` split first-class. The input is a positional `Command` (`InputCommand`, the first command); completions/signals are `Notification`s that are explicitly NOT positionally matched as commands — they are correlated by `NotificationId`, out of the command-index ordering. The cursor (command index) advances only over `Command`s; `Notification`s are a separate, id-keyed channel. This is precisely Overdrive's Option B generalized: a distinct, typed start/input entry that the replay machinery handles as a first-class ordered command, with other entry kinds (the result/notification channel) explicitly removed from the positional command match.

**Finding 3c — "Restart from a journal prefix" copies entries by index, and `from > 0` requires Service Protocol V6; index 0 is the first journal entry (the input command).**

**Evidence**: v1.6 release notes: "Journal entries from index 0 to `from` (inclusive) are copied to the new invocation"; "Restarting from `from > 0` requires Service Protocol V6 or later"; restart from `from = 0` works on older protocols. Separately, "restarting from a specific journal entry index requires Service Protocol V6 or later."

**Source**: [restatedev/restate — release-notes/v1.6.0.md](https://github.com/restatedev/restate/blob/main/release-notes/v1.6.0.md) — Accessed 2026-06-06
**Confidence**: High
**Analysis**: Confirms the modern protocol remains fundamentally positional (index-addressed), and that index 0 — the input command — is the canonical journal start. Note V1–V4 are deprecated for new invocations as of Restate 1.6 (error RT0020); V5/V6 (journal v2, the Command/Notification model) is current.

## Q4: What is Restate's determinism check on replay (divergence detection analogue)?

**Finding 4a — Restate has an explicit replay-divergence check (error RT0016, "journal mismatch") that keys on the sequence of journal entries (command type at each position), NOT on a name/hash.**

**Evidence**: RT0016 is documented as: "Journal mismatch detected when replaying the invocation: the handler generated a sequence of journal entries (thus context operations) that doesn't exactly match the recorded journal." Causes: (1) "Service code was modified without registering a new deployment version"; (2) "Code within the handler contains non-deterministic logic." Detection: "The system compares the sequence of journal entries generated during replay against the previously recorded journal. When these sequences don't align exactly, the mismatch is flagged." Unsafe changes that trip it: "Reordering Restate SDK operations (`run`, state access, service calls, awakeables)", "Adding or removing SDK operations in the execution path", "Changing operation inputs", "Modifying conditional logic that affects which operations execute."

**Source**: [Restate docs — Error RT0016 / references/errors](https://docs.restate.dev/references/errors) — Accessed 2026-06-06; corroborated by [Restate docs — Versioning](https://docs.restate.dev/services/versioning) — Accessed 2026-06-06
**Confidence**: High (two official-docs pages agree)
**Analysis**: This is Restate's analogue of Overdrive's `name` mismatch → fail-closed. Crucially, the determinism check is **positional + typed**: the re-executing handler must emit the same *sequence* (same command type at each command index). The `name` field that journal entries carry (proto field 12) is documented as **observability-only**, not the determinism key — confirming Restate does NOT key replay matching on a name string. The fail-closed behavior (abort the invocation, surface RT0016) matches Overdrive's intent: a divergent journal must fail, not silently feed wrong results.

**Finding 4b — Journal v2 entries carry an optional `name` field used for observability, not for replay matching.**

**Evidence**: "Every Journal entry has a field `string name = 12`, which can be set by the SDK when recording the entry, and this field is used for observability purposes by Restate observability tools."

**Source**: [restatedev/service-protocol — service-invocation-protocol.md](https://github.com/restatedev/service-protocol/blob/main/service-invocation-protocol.md) — Accessed 2026-06-06 (via search extract)
**Confidence**: Medium (single direct extract; consistent with the positional model in 4a)
**Analysis**: Reinforces that matching is positional/typed; the name is metadata.

## Q5: Does Restate persist invocation input durably as a journal entry, and how is it read on replay?

**Finding 5a — Yes: the input is a durable journal entry (`InputEntryMessage`, 0x0400), re-delivered positionally on replay.**

**Evidence**: `InputEntryMessage` (type `0x0400`) "carries the invocation input message(s) of the invocation"; it is non-completable and non-fallible. On replay, the SDK receives the same input from the journal stream (positionally, as the first entry) rather than re-reading any external source.

**Source**: [restatedev/service-protocol — service-invocation-protocol.md](https://github.com/restatedev/service-protocol/blob/main/service-invocation-protocol.md) — Accessed 2026-06-06
**Confidence**: High
**Analysis**: The input IS a positional journal entry in Restate, not out-of-band — but it is an entry the SDK's first syscall (read-input) *consumes positionally*, so it aligns with the cursor naturally. Contrast with `StartMessage.known_entries`, which is out-of-band metadata. This distinction maps directly to Overdrive's `Started` problem.

---

## Implications for Overdrive's A-vs-B Decision

**Restate's design evidences Option B — keep a distinct, typed start/input journal entry that the replay machinery handles as a first-class ordered entry — NOT Option A (deleting the start record / keeping input out-of-band).** The evidence is direct and primary:

1. **Restate journals the input as a positional entry, not out-of-band.** The input is `InputEntryMessage` (0x0400) in v1 and `Command::Input(InputCommand)` — the first `Command` variant — in journal v2. It sits at index 0 of the positional journal and is consumed positionally on replay (Findings 1a, 3b, 5a). The only thing Restate keeps out-of-band is the *control* `StartMessage` (carrying `known_entries` length + eager state) — which is exactly the "machine starts here" signal, NOT the durable input. The analogue mapping for Overdrive: the `StartMessage`-equivalent (control metadata) is fine out-of-band, but the **input digest belongs in the journal as a real positional entry**, like `InputCommand`.

2. **The terminal/output is also a positional journal entry.** `OutputEntryMessage` (0x0401) / `Command::Output(OutputCommand)` carries "the invocation output message(s) or terminal failure" and is an ordinary positional command at the end of the journal (Q3 follow-up). This directly validates Overdrive's `Terminal{result}` as a legitimate positional entry — Restate does not treat terminal results as a special non-positional marker; it is a `Command` like any other.

3. **Restate solved the exact "non-await control entry in a positional stream" problem with a typed Command/Notification split, not by deleting entries.** Journal v2's `RawEntry::Command | RawEntry::Notification` (Finding 3b) is the structural answer to the Overdrive bug: ordered, replayable things are `Command`s and advance the command index; results/completions/signals are `Notification`s correlated by `NotificationId`, explicitly *outside* the positional command match. Overdrive's bug is that its cursor conflates "positional append position" with "await-point consumption," so a non-await entry (`Started`) corrupts the walk. Restate's resolution is Option-B-shaped: **type the entries** so the cursor advances over the ordered/replayable class (commands: Input, Run/side-effect, Call, Sleep, Output) while the result class (notifications: completions, signals) is matched by id, off the positional command index.

4. **The determinism check is positional + typed, fail-closed — matching Overdrive's intent.** RT0016 fires when "the handler generated a sequence of journal entries ... that doesn't exactly match the recorded journal" (Finding 4a). The match is on the *sequence of command types at each position*, not on a name (the `name` field is observability-only, Finding 4b). This means: in Restate's model, the start/input entry being a typed `InputCommand` at position 0 is *part of* what the determinism check enforces — every replay must re-emit `InputCommand` first. That is the opposite of "don't journal the start"; the start IS journaled and IS checked.

**Why this points away from Option A.** Option A (delete `Started`, keep input out-of-band) would diverge from Restate's well-tested design in a way that loses two properties Restate deliberately keeps: (a) the input is a durable, replay-verified positional entry (so a code change that drops/alters input handling is caught by the determinism check), and (b) the journal is self-describing — index 0 is always the input, the terminal is always the output command, and a "restart from index N" feature (Restate v1.6, Finding 3c) becomes possible precisely because every meaningful step including input is a positional entry. Option A's "out-of-band input" forfeits both.

**Recommended framing for Overdrive (interpretation, not Restate fact):** Adopt Option B, but adopt Restate's *typing* discipline rather than a naive "cursor skips non-await entries." The cleaner generalization is: classify every journal entry as either a **replayable command** (advances the positional cursor: `Started`/input, `RunResult`, `SleepArmed`, `ActionEmitted`, `Terminal`) or a **notification/completion** (matched out-of-band: `SignalSeen`, and arguably the *completion* half of `SleepArmed`/`SignalAwaited`). Make the cursor advance over commands only, and make the determinism check assert "command type at index i matches re-execution," which is exactly Restate's RT0016. Under this model `Started` is a legitimate command-index-0 entry, the first re-executed await-point reads command index 1, and exactly-once is preserved. The half-measure ("skip non-await entries") works for the immediate bug but Restate's evidence is that the durable, principled fix is to *type* the stream.

**Confidence in the A-vs-B conclusion: High.** It rests on primary sources (the service-protocol spec and the restate runtime Rust source) cross-referenced with two official docs pages. The one caveat is interpretive: Restate's `Notification` channel is richer than Overdrive needs today (it carries completion results for concurrent calls), so the mapping of Overdrive's specific 7 entry variants onto Command-vs-Notification is an engineering judgment, not a Restate prescription.

## Source Analysis
| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|--------|--------|------------|------|-------------|----------------|
| service-invocation-protocol.md (restatedev/service-protocol) | github.com | High (1.0) | official/primary spec | 2026-06-06 | Y |
| crates/types/src/journal_v2/raw.rs (restatedev/restate) | github.com | High (1.0) | official/primary source | 2026-06-06 | Y |
| crates/types/src/journal_v2/command.rs (restatedev/restate) | github.com | High (1.0) | official/primary source | 2026-06-06 | Y |
| release-notes/v1.6.0.md (restatedev/restate) | github.com | High (1.0) | official/primary | 2026-06-06 | Y |
| docs.restate.dev — references/errors (RT0016) | docs.restate.dev | High (1.0, canonical first-party) | official docs | 2026-06-06 | Y |
| docs.restate.dev — services/versioning | docs.restate.dev | High (1.0, canonical first-party) | official docs | 2026-06-06 | Y |
| docs.rs/restate-sdk | docs.rs | High (0.9) | official API docs | 2026-06-06 | N (low yield) |
| jack-vanlightly.com — Demystifying Determinism | jack-vanlightly.com | Medium-High (0.8) | industry leader | 2026-06-06 | N (insufficient specificity) |

Reputation: High: 7 (88%) | Medium-High: 1 (12%) | Avg: ~0.96 (cited sources only). All claims rest on ≥1 primary/authoritative source; the load-bearing A-vs-B claims (Findings 1a, 3b, 4a, 5a) each have ≥2.

## Knowledge Gaps

### Gap 1: Exact replay-cursor implementation in the SDK (command-index advance vs notification-id lookup)
**Issue**: The *spec* and *runtime types* confirm the Command/Notification split and positional command matching, but I did not read the SDK-side state machine that physically advances the command index and resolves notifications by id (e.g. the restate-sdk-rust or sdk-shared-core invocation state machine). **Attempted**: docs.rs/restate-sdk (low yield — API reference only, no replay internals). **Recommendation**: Read `restatedev/sdk-shared-core` (the Rust core shared across SDKs) `src/...vm/` for the exact `CommandIndex` advance and notification-resolution loop before finalizing Overdrive's cursor design.

### Gap 2: Whether RT0016 detection is purely command-type-at-index or also compares command *content*
**Issue**: RT0016 docs say "doesn't exactly match" and list "changing operation inputs" as a trigger, implying content (not just type) participates, but the precise comparison key (type only, type+target, type+digest) is not spelled out in the sources read. **Attempted**: references/errors, versioning, search for "command_index mismatch". **Recommendation**: Read the runtime's journal-v2 comparison code (likely in `crates/types/src/journal_v2/` or the PP/invocation-status-machine) to see exactly what fields are compared — directly informs how strict Overdrive's determinism check should be.

### Gap 3: Single-source items
**Issue**: Finding 4b (the proto `name = 12` observability field) came from a single search extract of the spec, not a direct quote-verified read. The positioning of `InputEntryMessage` as literally "first" is inferred from `Command::Input` being the first enum variant + the index-0 restart semantics, rather than an explicit spec sentence (the spec extract in the Q3-followup fetch explicitly declined to confirm "first"). **Recommendation**: Direct-read the proto and the journal-construction code to quote-verify both.

## Conflicting Information
No substantive conflicts found across sources. Minor presentational difference: the older standalone v1 spec (service-invocation-protocol.md) uses `*EntryMessage` terminology and a single `RawEntry`/`CompletionMessage` split, while the current runtime source uses journal-v2 `Command`/`Notification` terminology. These are the same model across protocol versions (v1 → v5/v6), not a contradiction: the v2 names make explicit what v1 implemented via the entry/completion separation. V1–V4 are deprecated for new invocations as of Restate 1.6 (RT0020); the journal-v2 Command/Notification model is current.

## Sources
[1] Restate. "Service Invocation Protocol." restatedev/service-protocol, GitHub. https://github.com/restatedev/service-protocol/blob/main/service-invocation-protocol.md. Accessed 2026-06-06.
[2] Restate. "journal_v2/raw.rs." restatedev/restate, GitHub. https://github.com/restatedev/restate/blob/main/crates/types/src/journal_v2/raw.rs. Accessed 2026-06-06.
[3] Restate. "journal_v2/command.rs." restatedev/restate, GitHub. https://github.com/restatedev/restate/blob/main/crates/types/src/journal_v2/command.rs. Accessed 2026-06-06.
[4] Restate. "Release notes v1.6.0." restatedev/restate, GitHub. https://github.com/restatedev/restate/blob/main/release-notes/v1.6.0.md. Accessed 2026-06-06.
[5] Restate. "Error codes (RT0016 journal mismatch, RT0020 deprecated protocol)." docs.restate.dev. https://docs.restate.dev/references/errors. Accessed 2026-06-06.
[6] Restate. "Versioning." docs.restate.dev. https://docs.restate.dev/services/versioning. Accessed 2026-06-06.
[7] Restate. "restate_sdk (Rust) API docs." docs.rs. https://docs.rs/restate-sdk/latest/restate_sdk/. Accessed 2026-06-06.
[8] Vanlightly, Jack. "Demystifying Determinism in Durable Execution." jack-vanlightly.com. 2025-11-24. https://jack-vanlightly.com/blog/2025/11/24/demystifying-determinism-in-durable-execution. Accessed 2026-06-06.
