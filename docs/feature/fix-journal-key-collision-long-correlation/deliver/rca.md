# RCA — Journal-key collision for long correlation keys

**Component:** `crates/overdrive-control-plane/src/journal/mod.rs`
(`WorkflowId::for_correlation`)
**Class:** identity collision via lossy truncation
**Severity:** high — corrupts a second workflow instance's durable execution

## Symptom

Two distinct `CorrelationKey`s that share their first 124 mapped
characters but differ only past that point derive the **same**
`WorkflowId`. The second instance's `start()` then opens the *first*
instance's journal and either (a) finds a `Terminal` row and silently
no-ops, or (b) replays the wrong command sequence — corrupting the
second instance's execution.

## Root cause chain (5 Whys)

1. **Why do two distinct instances share a journal?** Their
   `CorrelationKey`s derive the same `WorkflowId`.
2. **Why the same `WorkflowId`?** `for_correlation` truncates the mapped
   id at `WORKFLOW_ID_MAX = 127` (`wf-` prefix + 124 correlation chars);
   the two keys are identical across those 124 chars.
3. **Why is 124 chars enough to lose distinctness?** The canonical
   correlation form `target:purpose/<hex>` puts the content-addressed,
   discriminating `<hex>` at the **end** — exactly the region truncation
   drops.
4. **Why is the ceiling 127 when `CorrelationKey` allows 253?**
   `WorkflowId` invented a bespoke `WORKFLOW_ID_MAX = 127`, half the
   shared `LABEL_MAX = 253` used by every other label-shaped id. The
   discrepancy has no downstream justification — the redb journal key is
   a variable-length `&str` (`journal/redb.rs:56-61`); the sim store keys
   a `BTreeMap<(WorkflowId, u32), _>`. Nothing requires ≤127.
5. **Why was the truncation believed safe?** The doc comment called it
   "defensive" ("correlation keys are already short"). That is a typical-
   value argument, not a type-maximum argument — the latent collision was
   reachable for any key longer than 124 mapped chars.

**Root cause:** a derived id (`WorkflowId`) sized its length ceiling with
a bespoke smaller magic number instead of the shared `LABEL_MAX` it is
derived from, so the mapping truncated the discriminating suffix of long
keys.

## Contributing factor (out of scope, documented)

The char-fold step (every non-`[a-z0-9-]` char → `-`) is *independently*
lossy: `payments:register/x` and a hand-built `payments-register-x` fold
to the same id. This only collides hand-built `CorrelationKey::new()`
keys carrying `:`/`/`; every `derive()`-produced key keeps a distinct
24-hex-char hash suffix, so once truncation is gone all in-tree usage
(the action-shim `StartWorkflow` arm) is collision-free. Not fixed here.

## Fix

Unify on the single shared ceiling, sized for the prefix the mapping
prepends:

- `overdrive-core/src/id.rs`: promote `LABEL_MAX` to `pub` (the DNS-name
  ceiling shared by every label-shaped id).
- `journal/mod.rs`:
  `const WORKFLOW_ID_MAX = overdrive_core::id::LABEL_MAX + WF_PREFIX.len()`
  (256). A `CorrelationKey` ≤ `LABEL_MAX` maps 1:1 plus the 3-char
  prefix, so the result always fits — the loop's length guard becomes
  structurally unreachable and the end-of-string discriminant always
  survives. Grammar updated `{0,126}` → `{0,255}`.

This makes the collision *unrepresentable* rather than detecting it after
the fact. Codified as a reusable rule in `.claude/rules/development.md`
§ "One shared length ceiling for label-shaped ids".

## Files affected

- `crates/overdrive-core/src/id.rs` — `LABEL_MAX` → `pub` + doc
- `crates/overdrive-control-plane/src/journal/mod.rs` — ceiling, prefix
  const, grammar docs/error, `for_correlation` doc/body; regression tests
- `.claude/rules/development.md` — new rule section

## Risk

Low. Widening a validation ceiling and a grammar interior bound is
backward-compatible — every previously-valid `WorkflowId` remains valid;
no persisted id shrinks or changes shape. Derived ids for *short*
correlation keys (all current in-tree usage) are byte-identical to before
(the truncation path never fired for them).

## Regression tests

- `for_correlation_long_keys_sharing_truncated_prefix_do_not_collide` —
  two 134-char keys sharing the first 124 mapped chars derive distinct
  ids (fails against the old 127 ceiling).
- `for_correlation_does_not_truncate_a_maximum_length_key` — a 253-char
  key maps to `wf-` + 253 chars with no truncation.
