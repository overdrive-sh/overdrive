# E09-v2 overall timeout delta review

## Review metadata

- **Review:** authorized overall E09-v2 remote timeout increase
- **Reviewer:** Codex, GPT-5.6 Luna, maximum thinking
- **Scope:** the `runner.sh` budget constant, its existing host-safe timeout
  fixture/assertions, and the corresponding timeout descriptions in the two
  E09-v2 READMEs
- **Boundary:** no re-audit of the previously approved production,
  predicate, scheduling, capture, or disposal changes
- **Native execution:** none performed. Native outcome and expectation README
  status remain owned by the separate evidence audit; this review makes no
  satisfaction claim.

## Verdict

**APPROVED.** The authorized remote setup/trial budget is consistently raised
from 600 to 1200 seconds, the 60-second cleanup grace remains unchanged, and
the derived transport timeout is 1320 seconds. The host-safe harness test and
focused static checks pass. No in-scope defect remains.

## Delta audit

`verification/expectations/E09-v2-vm-service-tcp-truthfulness-20/runner.sh:7`
now sets `REMOTE_RUN_BUDGET_SECONDS=1200`. The existing
`REMOTE_CLEANUP_GRACE_SECONDS=60` remains at line 8, and the existing formula
at line 11 derives `TRANSPORT_TIMEOUT_SECONDS=1320` from the budget, cleanup
grace, and unchanged 60-second transport margin. The remote timeout command
uses the 1200-second budget with `--kill-after=60s`; no scheduling or product
path is involved in this delta.

The focused fixture in
`verification/harness/test-e09-v2-runner.sh` recognizes `1200s` as its remote
budget and asserts `1200s` plus `--kill-after=60s` in the timeout-mode
transcript. Its fixture acceleration remains test-local; it does not change
the production runner's timeout semantics.

Both README descriptions now say 1200 seconds (20 minutes), retain the
separate 60-second cleanup grace, and show the 1200-second `timeout` command.
The reviewed runner, fixture, and README paths contain no stale 600-second
budget reference.

## Verification

The complete existing host-safe harness suite passed:

```text
bash verification/harness/test-e09-v2-runner.sh all
E09 v2 HOST-SAFE runner tests PASS (native product outcomes unverified)
```

The individual fixture modes (`valid`, `short`, `long`, `historical`,
`failed-row`, `split-identity`, and `timeout`) also passed. The focused static
checks passed:

```text
bash -n verification/expectations/E09-v2-vm-service-tcp-truthfulness-20/runner.sh \
  verification/harness/test-e09-v2-runner.sh                                      # exit 0
shellcheck verification/expectations/E09-v2-vm-service-tcp-truthfulness-20/runner.sh \
  verification/harness/test-e09-v2-runner.sh                                      # exit 0
git diff --check -- verification/expectations/E09-v2-vm-service-tcp-truthfulness-20/runner.sh \
  verification/harness/test-e09-v2-runner.sh \
  verification/expectations/E09-v2-vm-service-tcp-truthfulness-20/README.md \
  examples/service-kind-vm-workloads-v2/README.md                                 # exit 0
```

Mechanical evaluation of the arithmetic confirms
`1200 + 60 + 60 = 1320`. No native run, mutation test, commit, or source edit
was performed during this review.

## Findings and disposition

No reachable, in-scope defect remains. The constant, derived transport bound,
fixture expectation, cleanup grace, and user-facing timeout descriptions are
consistent. Native pass/fail status and any expectation `satisfied` transition
remain outside this review's authority.

**Final verdict: APPROVED.**
