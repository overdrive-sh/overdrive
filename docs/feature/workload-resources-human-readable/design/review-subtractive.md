# Subtractive review — DESIGN, workload-resources-human-readable (GH #372)

Scope: feature-delta.md, ADR-0339..0342, the ADR-0310 correction, spike findings. R-1..R-3 and decisions 1a, 2, 3a, 4a, 5a are binding and not
challenged. Nothing below removes a mechanism the rulings require. The design is lean in mechanism (two newtypes, no new dependency, no envelope
bump, no new aggregate error variant); the excess is mostly stale or duplicated artifact text and a few over-specified details.

## SHOULD-REMOVE

1. **Stale DISCUSS text that contradicts the approved design (feature-delta, DISCUSS sections).** D-5 ("envelope bumped"), AC-1.6, AC-2.6,
   DoD item 3 ("Envelope bumped with a new golden fixture"), the DISCUSS OQ-3/OQ-4 prose and the Out-of-scope bullet "(Moved in scope by R-2 ...)"
   still assert what DD-5 and R-2 reversed. A DISTILL reader gets two contracts for the same fact; DISTILL would write a bump test that DD-5
   forbids. Why unreal: DD-5 is approved and V-1 held; the DISCUSS lines are superseded, not additional. Fix: delete or rewrite those lines to the
   present contract (AC-1.6 becomes "untouched golden fixtures still decode"). Risk if removed: none; risk if kept: a DISTILL test for a non-existent
   V2.
2. **V-1 row says "NOT RUN ... reported as a blocker" (feature-delta, Validation items).** Later sections say HELD with a run id. The table is
   self-contradictory and the "No contract-shaping item is Open" line sits under a row that says Open. Replace the row's status with HELD plus the
   findings link. Risk: none.
3. **V-3 as a validation item (feature-delta, Validation items).** It is not contract-shaping ("falsification changes the schema text only"), has
   a precedent, and is discharged by the openapi check that DELIVER already runs. A validation item that nothing can falsify beyond an existing gate is
   padding (design.md: "every validation item ... validation cost"). Drop the row. Risk: none; the openapi check remains the gate.
4. **V-2 as a separate validation item plus a standalone "Cloud Hypervisor memory constraint" section.** DD-8 is already approved and its
   rationale is stated three times: ADR-0339 Consequences, § Workload memory refinement, and the V-2 section (with the same citations). Keep one
   statement (the V-2 section, which has the evidence) and have the refinement section and ADR-0339 point at it. Risk: none.
5. **Approval ledger repeated four times (feature-delta).** "Decisions" table (Status column), "Open questions resolved" table, "Decisions recorded
   as approved", "Validation results and user approvals", "User approvals (continued)". Each restates DD-1..DD-8 and decisions 1-5. Keep the Decisions
   table (id, ADR, approval date) and delete the other four restatements, including the re-listing of the grammar pins under R-1/R-3, which
   duplicates § Interface contracts. Why unreal: design.md requires the approval recorded once in the canonical artifact with id and date. Risk: none.
6. **ADR text that embeds interface and test detail (ADR-0339, ADR-0340, ADR-0341).** design.md says an ADR holds one decision and no
   signatures, test matrices or implementation instructions. Specific items:
   - ADR-0341 Decision names `vcpus_for`, `cpu_weight_for`, `MemoryPlan::derive`, `.bytes()` and the "about 242 files" blast radius. These are
     interface/delivery content and already live in the feature delta. Narrow the decision to "the typed quantities are carried end to end; the
     primitive is unwrapped only at foreign-protocol boundaries; the archived layout is unchanged".
   - ADR-0339 Consequences carries the whole-MiB refinement with CH source citations and the rejected examples `1000B`, `6KiB`, `1536KiB`. That is a
     second independently decidable decision (decision 5a; it could be reversed without touching the grammar) living inside the grammar ADR. Either
     give it one sentence plus a link to the feature delta, or its own short ADR. Do not keep the examples in the ADR.
   - ADR-0339/0340 Decision bullet lists restate the grammar rules (units, case, canonical form, bound) that § Interface contracts pins exactly.
     Two copies can drift. Keep the one-sentence decision and the alternatives; move rules to the delta only.
   - ADR-0339 "Negative" bullet about GH #348's scaffold and DISTILL owner is delivery coordination, already in § Corrections. Remove.
   Risk: none; the contract content survives in the feature delta.
7. **ADR-0310 status paragraph narrates the change.** It adds "The size grammar was changed by the user's ruling ... the change is stated here
   and not narrated, because this decision is not yet implemented." design.md § "Do not manufacture amendment history before implementation" says
   incorporate the correction into the present-tense text and add no history. Reduce the Status edit to nothing beyond keeping "Approved 2026-11-17"
   (the Links entry to ADR-0339 is enough provenance). Risk: provenance of the user ruling lives in the feature delta and ADR-0339.
8. **Duplicate driving-ports sections.** DISCUSS "Driving ports" and DESIGN "Driving ports" list the same four ports. Keep the DESIGN one (or neither:
   "unchanged" in one line). Risk: none.

## CONSIDER

9. **`ByteSizeParseError` has six variants; collapse to four.** `Malformed` and `UnknownUnit` carry the same operator remedy (the grammar), and
   `Fractional` is a sub-case of malformed input whose message can still say "write `1536MiB`". No code branches on the variant (grep: the only
   consumers are `Display` into `AggregateError`/CLI text). Keep `MissingUnit` (D-4 names the expected form), `DecimalUnit` (names the binary unit to
   write), `Overflow`, and one `Malformed`-style variant whose message is chosen at construction. Counter: the rust.md rule "distinct failure modes get
   distinct variants" favours keeping `Fractional`; if the DISTILL tests assert per-cause text, variants are the cheap way to pin it. Judgement call;
   not blocking. Same shape for `CpuQuantityParseError`: four variants is fine, no change.
10. **Decimal-unit detection of `Mb`/`kb` spellings with a `suggestion` field.** Mapping every case variant (`128Mb`, `1kb`) to a suggestion is more
   mechanism than "decimal units are refused". `UnknownUnit` with a message naming the binary units would cover it. Keep only if the user wants the
   targeted suggestion (the ADR says the silent 2.4% difference is the error class, which supports a targeted message). Low value either way.
11. **Nearest-valid-values message for the whole-MiB refusal.** Computing `{lower}`/`{upper}`, omitting `{lower}` under 1 MiB, adds branching and
   six table rows for text the operator can read from "must be a whole number of MiB; got X". The rule itself is approved (5a); the message shape is
   not. Simplify to the rule plus the received canonical value. Risk: slightly less actionable error.
12. **`Hash` derive on `ByteSize` and `CpuQuantity`.** `Resources` derives no `Hash` (`traits/driver.rs:309-322`) and nothing in the design keys a
   map by these types. `Ord` is consumed (scheduler `max_free` and `covers` comparisons) and stays. Drop `Hash` unless a consumer appears. Risk: a
   later need adds one derive.
13. **Feature-delta inventory duplicated across three tables.** The component decomposition, the consumer trace table and the Reuse Analysis list
   the same structs and functions (scheduler appears in all three; `MemoryPlan` in two). Merge the consumer trace into the decomposition and shrink
   Reuse Analysis to its two CREATE NEW rows plus the two KEEP rows (`ByteCount`, `MemoryPlan`) that carry a reason. Also drop file:line anchors
   that rot (`:259,446,898,961,1058,1119-1141`); the DISCUSS premise table already cites them.
14. **DDD list.** "No new aggregate, entity or event" followed by a five-row glossary; `canonical form` is a rule, not a DDD term, and `VolumeSize`
   and `Resources` already exist. Reduce to one line naming the two new value objects. The `brief.md` and `vm_lifecycle.rs` rows in the decomposition
   are tail items for the crafter's grep gate and can fold into "about 242 files".
15. **Grammar example tables as pinned contract.** The ByteSize/CpuQuantity tables are 25+ rows; they are the DISTILL scenario matrix (design.md
   routes scenario matrices to DISTILL). The grammar plus canonical rule is the contract. Keep the rows that pin a decision (decimal refused, uppercase
   `M` refused, canonical `1024MiB` -> `1GiB`, `0B`, the two bounds, bare number); the Malformed row variants (`1_000MiB`, `.5`, `1.`, `1e3`) are
   DISTILL's. Risk: DISTILL re-derives them.
16. **ADR-0310 "separate byte-granule rule for sub-GiB sizes" alternative.** Added by the correction; it restates that the refinement refuses
   sub-GiB sizes, which the Decision already says. Drop it. The reworded "volume-only GiB/TiB grammar" alternative is the one that carries the
   reason.
17. **Garbled alternative in ADR-0339** ("A published crate ... they accept decimal units by default"). It misstates the ruling and the
   `bytesize` crates; state only "rejected by the user's ruling: a dependency for a five-unit grammar". Wording fix, no design effect.

## Checked and kept (reason real)

- Both newtypes, `rkyv` derives on them (needed: `Resources` is archived), `Serialize`/`Deserialize` (TOML, JSON wire), `from_millis`/`millis`,
  `new`/`bytes` (consumed by scheduler, cgroup, VM driver).
- Keeping `ResourceEnvelope` and `ByteCount` untouched (already minimal; deleting either was correctly declined).
- No envelope bump and no `ByteSize` non-zero type; the whole-MiB rule as a check, not a type; `Node::new` non-zero only.
- ADR-0342 separate from ADR-0341 (wire shape is independently decidable, decision 3a); ADR-0339 and ADR-0340 separate.
- V-1 spike: contract-shaping, correctly run before approval; the untouched golden fixtures are a separate regression guard, not a substitute.
- Lifecycle Gate Ownership and Formal protocol model "Not applicable" lines (design.md requires the sections).
- `describe` `CPU:` line (decision 4a), zero CPU accepted, lowercase `m` only (R-3 pins).
