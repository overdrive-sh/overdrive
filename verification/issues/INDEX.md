# Issues — failed-expectation tracker

One file per failed expectation: `<NNN>-<slug>.md`. Opened when a `runner.sh`
sub-claim fails or an adversarial review rejects the evidence; closed when the
expectation returns to `satisfied`.

| # | Expectation | Summary | Status |
|---|---|---|---|
| _none yet_ | | | |

## Issue file template

```markdown
# <NNN> — <slug>

- Expectation: <ID>
- Opened: <YYYY-MM-DD> at overdrive SHA <sha>, seed <N>
- Surface: <O/R/D/E/X>

## What the evidence showed
<verbatim pointer into the expectation's evidence/ — the failing capture>

## Expected vs actual
<the sub-claim that failed, and the real captured output>

## Anchor
<the S-* / ADR / AC this expectation is bound to>

## Resolution
<commit / PR that fixed it; re-verification SHA + seed>
```
