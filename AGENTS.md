YOU MUST READ ALL THE FOLLOWING FILES:
- CLAUDE.md
- .claude/rules/bpf.md
- .claude/rules/debugging.md
- .claude/rules/design.md
- .claude/rules/development.md
- .claude/rules/rust.md
- .claude/rules/testing.md
- .claude/rules/verification.md

nwave skills can be found at $HOME/.claude/skills/nw-*/SKILL.md
nwave agents can be found at $HOME/.claude/agents/nw/

## Implement to the design — never invent API surface

When implementing against an accepted design (an ADR, `brief.md`, a
feature-delta, a roadmap step), match the design's **exact public API
shape**. Do **not** invent new public surface — a new method, type, enum
variant, trait, or parameter — to make tests green or to fill a gap the
design left underspecified. The design is a contract, not a suggestion;
an implementation that adds API the ADR did not call for has *diverged*,
even if every test passes.

When the design specifies a *model* but not the exact *signature* (e.g.
"the transient is the step's `Err` re-driven by the engine" without the
function shape), the gap is **not** licence to improvise. **STOP and
surface the gap** to the user / orchestrator and get the shape pinned —
never reach for the nearest mechanism that compiles. A subagent that
grades itself on "tests green" will invent surface; that is the failure
mode this rule exists to prevent.

This binds three roles:

- **Crafters**: build only the API the design names. If you need a
  primitive the design doesn't specify, return a blocker — do not add a
  public method/type/variant on your own initiative.
- **Orchestrators dispatching crafters**: point the crafter at the
  authoritative design (the ADR / feature-delta / roadmap step) and
  forbid inventing API. Do **not** pre-explore the codebase or restate
  the signature yourself — the crafter reads the design and is bound by
  the crafter rule above; duplicating that read is wasted work. The
  orchestrator's job is to not *loosen* the contract: granting latitude
  ("pick the cleanest shape," "add a variant if needed") *causes*
  divergence — do not.
- **Reviewers / orchestrators accepting work**: verify the output
  against the design's API shape, not just "tests pass." A green suite
  over a divergent API is a rejection, not an approval.

**Precedent** (the `workflow-result-error-model` feature, ADR-0065):
crafters twice invented surface the ADR did not sanction — a
`TerminalErrorKind::Retryable` variant (a "terminal error" that wasn't
terminal, flatly contradicting the ADR's "retryable never reaches the
return type"), then a second `ctx.run_retryable` step method instead of
the ADR's single `ctx.run`. Both compiled and passed their tests; both
were design divergences caught only in adversarial review and by the
user, and both cost a rework cycle. The cost of surfacing a gap is one
message; the cost of inventing past it is a wrong contract that
propagates until someone notices.

## DELIVER orchestration rules

These rules are mandatory for `/nw-deliver` and `/nw-execute` work in this
repository.

- Every roadmap step gets its own fresh, isolated crafter agent. Spawn it
  without inherited conversational turns and give it the complete DES prompt.
  Never reuse a crafter from an earlier step for a later step.
- The only crafter-reuse exception is review remediation: findings for a step
  go back to the original crafter for that same step. They never go to a later
  step's crafter.
- Every step gets its own fresh, isolated reviewer. Do not reuse a reviewer
  across roadmap steps. The same reviewer may perform iteration 2 for its own
  step after remediation.
- Before reporting completion, each reviewer writes its full review artifact to
  `docs/feature/{feature-id}/deliver/review-{step-id}.md`. The artifact records
  every iteration, verdict, finding, and remediation disposition. A returned
  chat verdict alone is not a completed review, and the next step must not start
  until the on-disk review exists.
- Persist review artifacts as native Markdown following the repository's
  existing DELIVER review convention: headings, metadata, findings, evidence,
  verification, remediation dispositions, and verdicts rendered as prose,
  lists, and tables. Do not put a YAML response inside a `.md` file. Structured
  YAML may be returned to the orchestrator for machine parsing, but it does not
  replace the Markdown review artifact.
- Crafters and reviewers do not send progress updates, partial findings, or
  status checkpoints back to the orchestrator. They report only when their
  bounded work is complete, or when they have reached a genuine blocker that
  prevents further progress. The orchestrator must not poll or prompt them for
  intermediate status.
- After dispatching an agent, the orchestrator waits silently for that agent's
  completed report or genuine blocker. Do not repeatedly check for progress or
  emit recurring "still running" commentary while the agent works. A wait
  timeout is not a status event and must not be surfaced to the user.
- An incidental request made during an active DELIVER run, such as updating an
  orchestration rule or documentation, does not end or pause the run. Complete
  the incidental request, then immediately resume the pending DELIVER phase
  unless the user explicitly stops, pauses, or replaces the active workflow.
- Treat a user statement, observation, correction, or question as
  conversational by default: answer it, but do not infer authorization to
  take an action from it. Act only when the user explicitly requests an
  action. In particular, pointing out a model mismatch, defect, risk, or
  surprising behavior does not authorize stopping, interrupting, replacing,
  or modifying an active agent or workflow.
- The step sequence is strict: fresh crafter RED -> GREEN -> COMMIT, fresh
  reviewer, original-crafter remediation if required, reviewer re-review, then
  and only then the next roadmap step.
- Review/remediation has no iteration cap. Keep cycling a step's original
  crafter for remediation and that step's reviewer for re-review until the
  reviewer returns `APPROVED`. Agent-definition defaults such as "max 2 review
  iterations" do not apply to this repository's DELIVER workflow and must not
  trigger human escalation or advancement with unresolved findings. If an
  original agent is no longer addressable, dispatch a fresh isolated
  replacement for the same role and step, then continue the cycle.
- Do not run mutation testing during individual roadmap steps. Mutation testing
  is a single final DELIVER-wave gate, after all steps and their reviews are
  complete. Per-step crafters must not delay RED -> GREEN -> COMMIT for mutation
  testing or edit mutation exclusions as part of a step unless the user
  explicitly overrides this rule.
- Keep executable tests and verification expectations as independent evidence
  layers. Rust tests exercise the production composition root in-process and
  must not spawn the built Overdrive production binary, emit expectation
  evidence, or act as an expectation runner. A `verification/expectations/*`
  runner is black-box: it directly drives the built default-feature binary and
  observes operator/kernel/wire/cleanup surfaces with crate-independent
  external tools. It must not invoke `cargo test`, `nextest`, a Rust test
  binary, or import/link an `overdrive-*` crate. Legitimate external Tier-3
  fixtures such as Cloud Hypervisor or guest workload processes remain allowed
  when the test contract requires them; they do not make the test harness the
  system-under-test process boundary.
- Keep examples, expectations, and integration tests at their distinct
  boundaries. Repository-root `examples/` contains checked-in,
  operator-runnable product examples that express a user journey. An
  expectation drives one of those examples through the built product and
  verifies only the stakeholder-visible black-box outcome. An integration test
  uses the production crates in-process to prove the internal guarantees that
  make the outcome true, including private lifecycle state, protocol framing,
  decoder behavior, normalized kernel-program identity, loss detection,
  generation stability, exact counters, and cleanup complements. Expectations
  must not recreate specs or workload programs inline, absorb integration-test
  assertions, invoke the test harness, or duplicate the Rust implementation in
  Python, shell, or another helper.
- A green suite, complete DES log, clean commit scope, or 100% mutation score
  does not replace the reviewer. Step 01-01 passed all of those and the
  reviewer still caught a blocking Contract Shape declaration defect.
- The orchestrator checks mechanical evidence only: DES phase order, commit
  trailers/stat, expected file scope, command results, and reviewer verdict.
  Deep correctness, design compliance, test honesty, and diff scrutiny belong
  to the dedicated reviewer per `.claude/rules/development.md`.
- Implementation review remediation must stay within the architecture already
  approved by DESIGN. A reviewer may identify an architectural gap, but it
  must not invent or iteratively prescribe a new persistence subsystem,
  system-of-record boundary, ownership model, consistency protocol, recovery
  protocol, or other architectural mechanism inside a DELIVER step. If a
  finding cannot be closed without such a choice, record the finding as a
  blocking DESIGN gap and return it for a separate DESIGN remediation and
  independent design review. Resume the original DELIVER step only after that
  design is approved; do not evolve implementation review iterations into an
  unreviewed architecture-design process.
- Do exactly the work the user requested. Do not replace a bounded fix with
  adjacent hardening, generalized lifecycle correctness, speculative failure
  handling, architectural cleanup, or a mechanism an agent considers more
  complete. A reviewer finding does not expand the task. If a finding or
  proposed dependency is not necessary to satisfy the user's stated outcome
  and accepted design, reject it as out of scope. Technical plausibility,
  severity, or elegance is not authorization. When the requested fix is small,
  the design and implementation must remain small unless the user explicitly
  expands them.
- If a designer, reviewer, crafter, or orchestrator believes something outside
  the approved scope should be added, changed, hardened, generalized, or
  redesigned, it must surface that proposal to the user and obtain explicit
  approval before acting on it. Record the rationale and likely impact, but do
  not edit design artifacts, add findings that mandate the expansion, change
  production code, or dispatch remediation for it while approval is pending.
  Agent judgment that an addition is useful, safer, cleaner, or a good idea is
  not authorization to invent or implement it.
- Treat every review finding as a hypothesis until its failure is proven
  reachable through the current production code. Before accepting a finding
  for DESIGN or remediation, the reviewer must cite the concrete production
  entry point, complete caller/owner path, exact state and ordering that
  trigger it, and the current shutdown/cancellation/retry behavior with
  file-and-line evidence. A theoretically cancellable Rust future, a forced
  test-only abort, or an internally consistent hypothetical state is not a
  production defect when the real owner drains the operation or the state dies
  with its process. Findings without this reachability proof are rejected, not
  converted into design requirements.
- Before accepting that anything needs a fix, remediation, or DESIGN change,
  reproduce the claimed failure with a bounded spike or a regression test that
  fails against the current implementation through the real production entry
  point and owner path. Static suspicion, design prose, a theoretical trace,
  or a test-only state that production cannot reach is not proof. The spike or
  failing test must isolate the original claimed defect without depending on
  the proposed remedy. If the failure cannot be reproduced, keep it recorded
  only as an unproven hypothesis; do not change production code or expand the
  design to address it.
- For suspected control-plane defects whose correctness depends on ordering,
  timing, concurrency, crash/restart, retry, or convergence, the required
  failing regression is first a seeded `overdrive-sim` safety, liveness, or
  convergence invariant against the current implementation. Designers and
  reviewers must not promote an imagined schedule into a finding or design
  requirement until that invariant fails reproducibly and prints its seed. A
  real-production-binary spike may additionally prove that the triggering state
  is reachable and that the Sim model matches production composition; it does
  not replace the invariant. Real-kernel Tier-3 tests remain the evidence for
  host-adapter effects that simulation cannot observe, such as actual netns,
  veth, TAP, nftables, cgroup, process, and redb behavior. Do not add a Sim
  seam, production API, or architectural mechanism merely to manufacture the
  hypothesized state; surface any missing testability boundary to the user for
  approval first.
- Revalidate the premise before designing the remedy. Designers must read the
  affected production paths and distinguish observed code facts from proposed
  behavior; accepted DESIGN prose and a reviewer assertion are not substitutes
  for implementation evidence. If the proposed remedy reaches into subsystems
  outside the proven path--for example broker scheduling, hydration, probe
  persistence, replay, task ownership, or recovery protocols--stop and prove
  that dependency is unavoidable before adding it. Do not make an invented
  failure model internally consistent.
- Remediation reviews must test both the fix and its necessity. When successive
  findings concern machinery introduced by the previous remediation rather
  than the original reachable defect, reopen the mechanism choice and simplify
  or remove it instead of continuing a patch-review-patch loop. No-iteration-cap
  means genuine defects are resolved until approval; it is not permission for
  an unbounded architecture-growth loop.
- When an existing synchronous interface gains work that must be awaited, make
  that existing interface async and update its bounded implementations and call
  sites. Do not preserve a stale synchronous signature by discovering a Tokio
  runtime, spawning a detached future, or adding a second public method. Task
  submission is not effect completion: release, commit, install, cleanup, and
  other ordering-sensitive operations must return only after their promised
  async effect or typed failure handling has completed.
- Only the isolated crafter executing a step may write that step's DES phase
  events. An interrupted or replacement agent must not claim inherited work;
  it independently reruns RED and logs only phases it actually executes.
- Preserve all pre-existing dirty work. Never reset, discard, overwrite, or
  silently commit unrelated files. A replacement agent must audit partial
  step changes left by an interrupted agent before adopting any of them.
- Use the user-selected GPT 5.6 Luna model with maximum thinking for every
  crafter and reviewer. When the agent interface inherits the current
  Conductor session model/reasoning and exposes no per-agent selector, inherit
  it; never downgrade reviewers to their legacy Haiku frontmatter default.
- Keep orchestrator reads minimal: the mandatory project files above, the
  command skill being invoked, the selected roadmap, DES rigor/log state, and
  mechanical results. Specialized agents load their own role skills and the
  code/design context needed for their bounded task. Do not preload unrelated
  skill trees in the orchestrator.
- New or transitioned tests must carry their required per-test Contract Shape
  declaration. For source-local pure-function Rust properties, use the exact
  rustdoc line `/// CONTRACT_SHAPE: pure-function.` on every live property.
- Roadmap allowlists must account for compiler-required fallout. Adding a
  public `AllocationSpec` field requires neutral updates at every existing
  struct literal; enabling nix ioctl macros requires the workspace `nix`
  dependency's `ioctl` feature in `Cargo.toml`. Treat those as tightly bounded
  mechanical fallout, not as permission for unrelated behavior changes.
- Roadmap `implementation_scope`, `files_to_modify`, and similar file lists are
  guidance, not restrictive allowlists. The acceptance criteria define the
  required implementation boundary. Crafters may change additional production,
  API, renderer, test, harness, configuration, and compiler-fallout files when
  those changes are necessary to satisfy the step honestly. They must document
  why each expansion is required and keep it tightly related to the criterion;
  they do not stop merely because a necessary file was omitted from the roadmap
  list.
- Initialize `execution-log.json` with `des-init-log`; never hand-write phase
  events. In this environment the DES launchers require
  `PYTHONPATH=/Users/marcus/.claude/lib/python` so they can import the bundled
  `des` package.
- A roadmap with `validation.status = pending` is not executable unless it is
  reviewed or the user explicitly directs that it be marked approved. Do not
  silently self-approve it.

The installed nWave skill layout is
`$HOME/.claude/skills/nw-*/SKILL.md`; agent definitions are under the agent
directory listed above.


<claude-mem-context>
# Memory Context

# [helios/accra] recent context, 2026-09-06 3:12am GMT+2

Legend: 🎯session 🔴bugfix 🟣feature 🔄refactor ✅change 🔵discovery ⚖️decision 🚨security_alert 🔐security_note
Format: ID TIME TYPE TITLE
Fetch details: get_observations([IDs]) | Search: mem-search skill

Stats: 50 obs (23,233t read) | 879,622t work | 97% savings

### Jul 29, 2026
57778 5:51p 🔵 OAI-PMH Date Filtering Confirmed but Record Format Shows Person Names Not Paper Titles
57780 5:52p 🔵 All Alternative Access Methods for Woodcock-Larsen Paper Exhausted With Zero Success
57781 5:53p 🔵 Aarhus Pure Web Interface Returns HTTP 403 Cloudflare Protection for Woodcock Publication Listings
57782 5:54p 🔵 CrossRef Metadata Confirms Paper Existence But Lists Null License and Similarity-Checking PDF Only
57783 " 🔵 White Rose Repository Search by Author Name Accessible But Results Content Not Captured
57784 " 🔵 White Rose Repository Contains 7 Woodcock Publications from 2025-2026 But Target Paper Absent
57785 " 🔵 Browser Automation Infrastructure Available But Python Libraries Not Installed
57786 5:55p 🔵 Headless Chrome Blocked by Cloudflare Bot Detection on ACM DOI Page
57787 " 🔵 Python Virtual Environment Created With websocket-client for CDP Automation
57788 5:57p 🔵 Chrome DevTools Protocol Endpoint Successfully Accessible on Port 9333
57789 " 🔵 Chrome Headless Started Successfully With Anti-Bot-Detection Flags on Port 9335
57790 " 🔵 Chrome WebSocket Connection Rejected With HTTP 403 Due to Missing Origin Allowlist Flag
57791 " 🔵 CDP Automation Successfully Connected But Cloudflare Challenge Runs Indefinitely Without Resolving
### Sep 6, 2026
72652 12:12a 🔵 Research Request for Project Implementation Best Practices
72674 12:30a 🔵 nwave documentation density configuration discovered
72675 " 🔵 Overdrive platform product documentation structure mapped
72676 12:31a 🔴 DISCUSS wave initiated for service-kind VM workloads feature
S13069 Commit service-kind VM workloads SPIKE wave evidence after native-metal validation (Sep 6 at 12:33 AM)
72681 12:34a 🔵 VM Exec Health Probes Research Completed for GH #257
72682 " 🔵 GitHub Issue Dependency Chain Mapped for VM Service Support
72683 " 🔵 Feature-Delta Wave Structure and Probe Implementation Precedent Identified
72707 1:38a 🔄 Inlined process reaping logic in service-vm-spike
72708 1:40a 🔵 H4 test case timeout after reap_adopted removal
72709 " 🔴 Fixed socket line buffering in host_controller.py
72710 " 🔵 All H1-H5 test cases pass after socket buffering fix
72711 " 🟣 Added H6 integration test for Overdrive VM networking
72712 1:43a 🔴 Added missing BPF build step to H6 test script
72713 " 🔵 BPF build toolchain requires Lima VM unavailable on metal target
72714 " ✅ H6 test uses native BPF build to bypass Lima requirement
72716 " 🔵 H6 test metal run completed with empty output
72717 1:45a 🟣 H6 integration test passes end-to-end with native BPF build
72718 " ✅ Added elapsed time instrumentation to H3, H4, and H5 tests
72719 " 🔵 H1-H5 tests pass with timing telemetry showing millisecond-level performance
72720 1:46a 🔵 Metal test substrate hardware and software configuration documented
72721 " 🟣 Service-kind VM workloads spike findings documented and complete
72731 " 🔄 Service-VM spike code relocated to canonical spike-scratch directory structure
72732 2:02a ✅ Spike documentation completed with READMEs and wave-decisions disposition
72733 2:03a 🔵 Increment-b capture script execution completed with empty output
72736 2:04a 🔵 Increment-b host-network-probe capture succeeded with H6 evidence collected
72737 " ✅ Cargo lockfiles generated for both spike increments
72742 2:05a 🔵 Increment-b run 0002 evidence confirms H6 validation with complete resource cleanup
72743 " 🔵 Spike artifacts share common components across increments
72744 2:06a ✅ Findings documentation updated with evidence-integrity correction
72745 " 🔵 Comprehensive spike validation passed with minor whitespace issues
72746 " 🔵 Rsync scripts retain .context exclusion for backward compatibility
72750 2:07a 🔵 Service-kind VM workloads spike validated six hypotheses on native metal
S13070 Complete SPIKE wave commit and transition to DESIGN wave for service-kind VM workloads (Sep 6 at 2:07 AM)
**72747** 2:08a ✅ **Evidence preservation gitattributes added to exempt capture files from whitespace normalization**
A .gitattributes file was created under spike-scratch/service-kind-vm-workloads/ to preserve evidence files byte-for-byte as captured from the metal executions. The file sets -whitespace for all increment-*/runs/*.stdout and increment-*/runs/*.stderr files, exempting them from Git's whitespace normalization and pre-commit hooks. This ensures the SHA-256 hashes recorded in .meta files remain valid even if the raw process output contained trailing spaces or other whitespace that would normally be flagged. The trailing whitespace in findings.md was also fixed by removing spaces after the Date and Scope front-matter lines. With these changes, git diff --check passes cleanly, confirming all source files have clean whitespace while evidence captures are preserved exactly as produced by the metal substrate.
~367t 🛠️ 15,587

**72748** " 🔵 **All six hypotheses validated in authoritative tracked runs**
The authoritative tracked runs confirmed all six hypotheses passed on bare-metal hardware. H1-H5 from increment-a run 0003 validated that a distinct guest-initiated vsock control session can coexist with the Beacon lifecycle session (H1), concurrent probes execute with proper correlation (H2), timeout cleanup works without killing the VM (H3 in 510ms), concurrency can be bounded with prompt overload rejection (H4 in 1205ms total), and disconnect/reconnect works with proper ambiguous-request handling (H5 reconnected in 21ms). H6 from increment-b run 0002 validated that the production Overdrive VM network stack supports host-to-guest TCP probes through the complete topology (host-veth → netns ovd-ns-0000 → tap ovd-tp-0000 → guest workload_addr), with independent TAP-level SYN observation and complete resource cleanup. These results establish the feasibility constraints documented in findings.md and gate progression to the Application/component DESIGN wave.
~490t 🔍 15,587

**72749** " 🔵 **Spike artifacts ready for commit with complete validation**
The comprehensive validation confirmed all spike artifacts meet commit-readiness criteria. Evidence integrity is cryptographically verified (all SHA-256 hashes match), security is enforced (metal target fully redacted), spike isolation is maintained (crates/ unchanged), and code quality gates pass (scripts valid, whitespace clean). The untracked directories include the complete spike documentation under docs/feature/service-kind-vm-workloads/spike/, research notes under docs/research/virtualization/, and the two self-contained increments with their evidence under spike-scratch/service-kind-vm-workloads/. The modified product documentation files (AGENTS.md, jobs.yaml, journeys, personas) reflect updates from earlier work. All artifacts are ready for git add and commit to preserve the authoritative H1-H6 evidence for the service-kind VM workloads feature.
~455t 🔍 15,587

S13072 Complete and commit service-kind VM workloads SPIKE wave evidence after native-metal validation (Sep 6 at 2:08 AM)
**72751** 2:10a ✅ **Spike evidence committed to repository for service-kind-vm-workloads**
Session committed the complete spike evidence package for service-kind-vm-workloads to the repository. The commit preserves both the validated findings document (195 lines documenting H1-H6 passing results, measured substrate identity, reproduction commands, design constraints) and the throwaway probe implementations that generated the evidence. The evidence is structured as two tracked increments: increment-a validated guest-initiated vsock control with concurrent probe execution, timeout containment, bounded admission, and reconnect (H1-H5), while increment-b validated host-to-guest TCP reachability through the production network path (H6). Each increment includes its complete source code, capture scripts with security redaction of target credentials, and SHA-256 verified run outputs documenting both setup failures and final passing runs. The wave-decisions document explicitly records the DISCARD disposition - the spike code under spike-scratch remains isolated and establishes only feasibility constraints for the upcoming DESIGN wave, with no provisional mechanisms (ASCII framing, port numbers, capacity limits, process-group containment) promoted to product contracts. A gitattributes file exempts the raw stdout/stderr captures from whitespace normalization to maintain byte-exact evidence integrity. Pre-commit hooks validated the commit structure (no Rust/TOML/YAML linting needed as only documentation and isolated spike code changed). The commit advances the feature from feasibility validation to design-ready state with durable, reproducible evidence backing the 10 established design constraints.
~723t 🛠️ 20,815

S13074 DESIGN wave for service-kind VM workloads following SPIKE wave commit (Sep 6 at 2:10 AM)
S13075 DESIGN wave for service-kind VM workloads following SPIKE wave completion and commit (Sep 6 at 2:11 AM)
S13076 DESIGN wave for service-kind VM workloads following SPIKE wave completion - architecture decision validation against existing ADR boundaries (Sep 6 at 2:17 AM)
S13077 DESIGN wave for service-kind VM workloads - design validation correction and process improvement to prevent architecture divergence (Sep 6 at 2:23 AM)
S13080 DESIGN wave for service-kind VM workloads - process improvement completion and wire protocol design progression (Sep 6 at 2:25 AM)
S13081 DESIGN wave for service-kind VM workloads - rapid design iteration progression following wire protocol framing decision (Sep 6 at 2:26 AM)
S13089 Per-request guest cgroup v2 leaf isolation necessity evaluation: researching whether per-exec-request cgroup isolation is standard practice or over-engineering (Sep 6 at 2:32 AM)
**72789** 2:33a 🔵 **Codec A/B Spike Completed: rkyv 53KB Smaller Than JSON for VM-Exec Protocol**
Completed codec A/B spike for service-kind-vm-workloads feature comparing length-prefixed JSON vs rkyv for VM-Exec control message protocol. The spike tested one assumption only: whether binary size, wire size, validation complexity, or schema evolution would favor one codec under the declared priority order (stripped binary size first, safe bounded decoding second, encoded bytes/timing third). The experiment used an init-representative standalone binary mirroring overdrive-init's actual x86_64-unknown-linux-musl target, thin LTO, and strip profile. rkyv won the primary criterion (stripped incremental binary size) with +24,568 B versus JSON's +77,816 B over baseline, a 53,248 B advantage. rkyv also produced smaller wire frames and decoded 4.97× faster. JSON offers simpler additive schema tolerance; rkyv requires explicit version envelopes plus retained per-version decoders. Both passed identical bounded rejection tests. Increment C (runs 0001-0003) failed due to rsync setup, preflight refusal, and experimental-design flaw (rkyv build eliminated JSON baseline path). Increment D corrected the design and completed three successful runs with reproducible measurements. All evidence preserved under spike-scratch/service-kind-vm-workloads/ with SHA256 verification and .gitattributes exemption for raw stdout/stderr. Findings document recommends choosing rkyv at DESIGN gate if stripped footprint priority holds, or JSON if additive schema tolerance and operational readability outweigh the 53KB gap.
~693t 🔍 26,082


Access 880k tokens of past work via get_observations([IDs]) or mem-search skill.
</claude-mem-context>
