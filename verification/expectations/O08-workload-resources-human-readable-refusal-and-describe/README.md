# O08 — a mistyped workload size is refused in words, and `describe` reads the declared size back in the operator's own units

**Surface:** O (operator CLI) · **KPI:** Declared size readable in `describe` / Silent mis-size on mistyped value · **Status:** `pending`

<!-- Status rationale: authored at DISTILL, before any implementation evidence
exists (`.claude/rules/verification.md` § "DISTILL — author expectations").
Nothing has been captured. The status stays `pending` until a different agent
reviews evidence captured against the built `overdrive` binary. -->

## Expectation

Ana Ortiz sizes a workload in `[resources]` with `cpu = "500m"` and
`memory = "128MiB"`. Two things hold on the built product, and no `assert!`
holds the first one:

1. **A refusal tells her what to write, not what the parser tripped on.**
   Deliberately contrasting mistakes (`memory = 134217728`, `memory = "128MB"`,
   `cpu = "500M"`, `memory = "1000B"`, the removed `memory_bytes`) each make
   `overdrive deploy` exit non-zero before any allocation exists, and the
   message **names the key or the form that was expected** (for example the
   binary units, or the lowercase milli suffix `m`). A reader who has never
   seen the grammar can correct the spec from the message alone. "Names the
   remedy" is a judgement a person makes from the captured text.
2. **A size she writes is read back in her own words.** After
   `overdrive deploy dev-box.toml`, `overdrive workload describe dev-box` shows
   `Memory:` and `CPU:` lines in the canonical spelling: `128MiB` and `500m`,
   never a raw byte count or a bare milli-unit number. A size she spelled
   differently (`memory = "1024MiB"`, `cpu = "0.5"`) is shown as `1GiB` and
   `500m`, so she can see what the platform understood.

What would refute it: a refusal whose text only says "invalid TOML" or "unknown
field `job`", or a describe that prints `134217728`.

- Anchor: docs/feature/workload-resources-human-readable/feature-delta.md AC-1.3, AC-1.5, AC-2.3, AC-2.8 (DISCUSS stories US-1, US-2)
- Anchor: docs/feature/workload-resources-human-readable/feature-delta.md D-3, D-4, DD-7
- Anchor: docs/product/architecture/adr-0339-memory-and-volume-sizes-share-one-binary-byte-size-grammar.md
- Anchor: docs/product/architecture/adr-0343-a-workload-memory-is-a-whole-number-of-mib.md

## Verification

Evidence is captured at DELIVER or DEVOPS against the built `overdrive` binary
on the native-metal substrate (`.claude/rules/testing.md` § "Native-metal
expectation captures run with `--exclusive`"): one `overdrive serve`, then
`overdrive deploy` of the accepted and refused specs above, then
`overdrive workload describe`, recording verbatim stdout, stderr and exit codes.
A point-in-time capture with a handful of deliberately contrasting specs, not a
matrix; the parser and admission rules are covered exhaustively by the test
tiers (`crates/overdrive-core/tests/acceptance/`). A reviewer who did not write
the implementation judges "names the remedy" from the captured refusal text and
sets the status.

## Evidence

None captured. Not yet run.
