# Documentation review — Metal run isolation

## Review metadata

- Feature: `metal-run-isolation`
- Review date: 2026-11-08
- Reviewer: independent documentation reviewer
- Iteration: 1
- Verdict: **NEEDS_REVISION**
- Required corrections: **1 bounded factual correction**
- Native feature acceptance: **PENDING**, independently of this documentation verdict
- Reviewed repository HEAD: `66669d66ec66` (abbreviated)
- Reviewed implementation: `ee0bbd65805524afa4a6348b5019ce12a7390e8b` and `2ba2232208ce9a5e91666f61b6b9845d9797c62a`

The review covers the two uncommitted website pages and their how-to navigation entry. It does not reopen the approved implementation, authorize native execution, or authorize publication. The only required change is to distinguish a locally printed run identifier from durable registration.

## Scope and authority

Reviewed content:

- [`run-verification-on-metal.mdx`](../../../../website/content/docs/how-to/run-verification-on-metal.mdx)
- [`sharing-a-metal-box-without-sharing-test-state.mdx`](../../../../website/content/blog/sharing-a-metal-box-without-sharing-test-state.mdx)
- [`how-to/meta.json`](../../../../website/content/docs/how-to/meta.json)

The reviewer read `AGENTS.md`, `CLAUDE.md`, `.claude/rules/finalize.md`, `.claude/rules/domain-naming.md`, `.claude/rules/verification.md`, `website/CLAUDE.md`, and the repository [technical-writer skill](../../../../.claude/skills/technical-writer/SKILL.md). The technical-writer skill governs the writing review throughout both pages. The finalization rule permits the blog's decision narrative while the operator guide describes implemented commands and current limits.

Factual authority includes the accepted P1–P5 contracts in [feature-delta.md](../feature-delta.md), ADR-0277 through ADR-0281, [source-identity implementation review](review-01-01.md), [integrated implementation review](review-01-02.md), [construction findings](../spike/findings.md), [resource alternatives](../design/resource-alternatives.md), [native socket-collision disposition](../design/f1-disposition.md), [codec compatibility evidence](../design/codec-compatibility/findings.md), the retained formal summary, and both retained ordinary-concurrency attempts. Narrow implementation reads checked the documented CLI, run markers, registration order, and existing source-transfer behavior.

## Iteration 1 — factual finding

### D1 — A printed run ID does not establish durable registration

**Priority:** required factual correction.

**Location:** `website/content/docs/how-to/run-verification-on-metal.mdx:152-155`.

The guide says the runner prints `OVERDRIVE_METAL_RUN_ID=<run-id>` “when it registers the invocation.” These are different lifecycle events in the implemented interface.

The actual public entry point at `xtask/src/main.rs:990-1011` invokes `infra/metal/bootstrap.sh --run`. The bootstrap chooses and prints the identifier locally at `infra/metal/bootstrap.sh:436-441`. It then performs remote path and login/sudo discovery, including fallible SSH operations at `:641` and `:651-659`. The durable `systemd-run` owner is not launched until `:813-848`. That owner creates its run directory and initial records at `infra/metal/lease-holder.sh:811-821`, then emits the separate `OVERDRIVE_METAL_RUN_REGISTERED` marker at `:864-867`. The accepted lifecycle table in `feature-delta.md` expressly states that a locally printed ID proves neither registration nor start.

This is a documentation mismatch with an intentionally distinct implemented state, not a proposed runner defect. The distinction determines whether a retained run exists for collection after an early failure. The existing warning to wait for `OVERDRIVE_METAL_RUN_STARTED` correctly handles command start but does not make the registration claim accurate.

**Required correction:** state that the runner prints the identifier before durable registration. Retain the separate registration-before-transfer/preflight and START-before-command claims. A short correction to the opening sentence is sufficient. No implementation, test, CLI, or lifecycle change is requested.

**Disposition:** open at iteration 1.

## Factual and decision-story assessment

| Area | Evidence and assessment |
| --- | --- |
| Motivating constraint | The blog opens with unrelated verification serialized by the physical-maintenance lease and explains why source and resource owners must also change. It does not claim measured queue savings or throughput. |
| Worktree identity | Both pages match canonical worktree-specific Git metadata identity, attached ordinary-clone fallback, detached ordinary-clone refusal, complete binding comparison, explicit override ownership, cold caches, and non-destructive legacy-cache handling. |
| Shared admission and administration | Default ordinary commands, shared maintenance admission, same-checkout exclusion, and explicit `metal run --exclusive -- COMMAND` agree with the reviewed contract. Shell/provisioning and older exclusive callers retain their authority. `--no-sudo` does not change admission class. |
| Serious resource alternatives | The blog distinguishes an insufficient hash from a CID-window allocator that was superseded without a completed experiment proving it impossible. Kernel namespace ownership addresses both CID and listener scope without adding a userspace allocator or claim service. |
| Namespace and kernel scope | The source-backed local-mode vsock boundary, fresh anonymous parent/local child, unchanged physical initial namespace mode, existing CID bands, and actual held listener ports are accurately scoped. Guest-kernel mutations remain separate from host VMM launch resources. |
| Native construction | The paired real GuestKit construction, identical CID, overlapping readiness, distinct boot IDs, survivor control path, and observed physical-kernel complement are explicitly limited to the retained construction. Neither page treats them as proof of the completed runner. |
| Test placement and attempt ownership | The pages retain nextest as selection/retry/profile/report/result owner, exact binary/test placement, fail-closed unknown identity, prepared dispatcher, actual selected executable/helpers/environment, fresh existing-group instance per attempt, and the explicit suite's batching. The body-255/nextest-100 distinction is preserved. |
| Unselected execution alternatives | The whole-run outer guest, persistent guest broker, and batch-result interpreter are described by their actual boundary or ownership costs. Viable alternatives are not called nonfunctional. |
| Reproduced placement correction | The six real-socket Rust identities and actual `AddrInUse` reproduction are separated from the unreachable current CLI selector. The story neither attributes this failure to Quint nor expands the correction into unrelated tests or CLI activation. |
| Native qualification | The native fixture's initial/local namespace contrast, actual parent-permission failure, minimal group-4200/mode-0710 construction correction, original confinement/data assertions, and 128 MiB declaration match the evidence. Named nested correctness cells receive no direct-native launch or benchmark credit. |
| Completion ownership | The actual supported nextest timeout explains why Drop is insufficient. The existing unit's `ExecStopPost`, three invocation roots, distinct command terminal/completion receipt, typed cleanup failure, read-only repeated collection, v1 compatibility, and absence of reboot recovery or pruning promises match P5. |
| Formal evidence | The model is introduced before its results. The retained 18 expected outcomes are correctly divided into 16 holds and two intentional hazard violations. Fairness and termination assumptions are visible. The self-introduced recording-codec incompatibility remains modeling infrastructure, and the post expressly disclaims production bug discovery and kernel/implementation-conformance proof. |
| Typed outcomes | The operator guide's runner-diagnostic table preserves the documented 75/78/70/74 distinctions. It does not turn a numerical command status into a runner failure or invent a public error representation. D1 concerns the earlier identifier/registration distinction. |
| Final native gate | Both pages state that final acceptance remains pending, commands did not start in the retained attempt, admission/placement journeys remain unrun, and no speedup was measured. The later retained attempt also stopped before START and supplies no countervailing proof. No completed K1–K3 overlap, physical-kernel zero delta, or survivor-complement outcome is claimed. |

The blog may link the first retained attempt without recounting the second attempt's repeated admission timeout. Its explicit pending status remains accurate against both captures. Requiring another chronological account would add length without changing the conclusion.

## Technical writing and equal-weight subtractive review

Both pages establish the subject before examples rely on it. The blog introduces the runner, vsock/CIDs, nextest's ownership, the transient service's role, and Quint at their relevant points. Source links support the explanations without replacing them. The operator guide puts the pending native gate near the opening, then follows preparation, execution, administration, source ownership, and collection.

The writing uses concrete owners and active verbs, names the costs of the selected approach, and separates observations from unmeasured outcomes. It makes no broad product isolation or performance promise. The guide avoids delivery-history narration. The blog uses historical contrasts for the engineering journey required by the finalization rule. No roadmap coordinates appear in maintained page names, headings, or command examples. A mechanical check found no authored semicolons in either page.

The subtractive review used the same weight as the factual review. No additional mechanism, example, troubleshooting matrix, historical task list, or speculative warning is necessary. The alternatives and failed native-fixture premise explain consequential design choices. The repeated native limitation at the opening and ending serves readers who enter through either summary or technical detail. The guide's compact tables replace longer enumerations. D1 can be fixed in place without growing either document. No optional style preference is promoted into a blocking finding.

## Validation evidence and limits

The documentation author reported passing `bun run typecheck`, `bun run lint`, `bun run build`, `bunx opennextjs-cloudflare build`, `bun run assert:one-index`, authored-link/anchor validation, and whitespace checks. The author also reported a passing local Worker check of both routes, how-to navigation, blog listing, HTTP and MCP search discovery, `llms.txt`, reciprocal links, and byte-identical MCP/Markdown exports. These commands were not rerun wholesale by the reviewer.

The author reported that `test:blog`, `test:mcp`, and `test:mcp:analytics` stop at the same existing search-ranking assertion: query `deterministic simulation` ranks `/docs/concepts/workload-identity` above `/docs/concepts/deterministic-simulation-testing`. The unchanged `git archive HEAD:website` baseline reproduces that failure without either new page or navigation entry. Later analytics assertions remain unexecuted. The review does not claim those three suites passed and does not require an unrelated search-ranking change.

The installed `bunx` proto shim refers to a missing binary. The author used a temporary PATH shim for local script launches, without editing global configuration. This is recorded as an environmental limit, not a content defect.

The reviewer independently performed these bounded read-only checks:

- All nine authored links resolve to existing repository routes or exact Git objects, including both GitHub-style heading anchors. This establishes local target/anchor correctness, not that unpushed GitHub commits are already publicly reachable.
- The how-to entry occurs exactly once in `meta.json`.
- The current generated `website/.next/server/app/one-index-check.body` reports **30 published pages, including 7 blog posts**, present in all three consumers, with one draft excluded.
- The untouched baseline gate reports **28 published pages, including 6 blog posts**, present in all three consumers, with one draft excluded.
- All **87 tracked `HEAD:website` files** match the retained baseline directory byte for byte at `/private/var/folders/11/l6dd1mtj007b7wkb4280qxdm0000gn/T/metal-docs-website-baseline-6sc5echb`.
- Current generated HTML contains the guide's pending-native callout, the blog's pending-paired-outcome section, the guide navigation entry, and the blog listing entry.

No standalone author validation logs were retained on disk. The author identifies its tool transcript as the record for command exits, Worker assertions, and the current/baseline ranking failures. The generated gate artifacts and page HTML were independently inspected as described above. This review distinguishes those direct observations from the author's reported command results.

No Metal, SSH, native, mutation, Rust test, deployment, publication, push, or commit command was run. No production code, test body, model, acceptance evidence, DES event, or unrelated dirty file was edited. The reviewer owns only this review artifact.

## Remediation disposition and verdict

| Finding | Required action | Status |
| --- | --- | --- |
| D1 | Correct the run-ID sentence so identifier emission does not imply durable registration. Preserve the existing start and collection semantics. | **OPEN** |

**NEEDS_REVISION.** Correct D1 and return the same bounded page change for re-review. No other prose, navigation, design, implementation, or validation expansion is required by this review. The feature's native acceptance remains pending regardless of the later documentation verdict.

## Iteration 2 — run identifier and durable registration

**Scope:** D1 only. The original author corrected the operator guide at `website/content/docs/how-to/run-verification-on-metal.mdx:152-156`. The reviewer read the corrected paragraph and compared its lifecycle claims with the implementation evidence retained under D1.

The paragraph states that the runner prints `OVERDRIVE_METAL_RUN_ID` after choosing the invocation ID locally, before durable registration. It separately states that registration precedes transfer and preflight, and that `OVERDRIVE_METAL_RUN_STARTED` establishes command start. The controller-loss guarantee still begins after START. This matches the existing local identifier, durable record, and started-command boundaries.

The correction is sufficient and adds no runtime promise or API. The original author's scoped whitespace check passed. No broad website check, Metal, SSH, native, mutation, or Rust test command was needed or run for this bounded wording correction. Iteration 1's validation evidence and disclosed baseline failures retain their original limits.

| Finding | Remediation | Disposition |
| --- | --- | --- |
| D1 | Describe local identifier selection before durable registration, with separate registration and START meanings. | **CLOSED — verified in iteration 2** |

## Final documentation verdict

**APPROVED.** The operator guide, engineering blog, and how-to navigation satisfy this documentation review. No required documentation finding remains open.

This approval covers documentation accuracy, writing, decision narrative, discovery, and the recorded website validation disposition. It does not establish final native acceptance, complete the feature, authorize another native run, or authorize publication. Both retained ordinary-concurrency attempts ended before START, and the required implemented-path native proof remains pending.

## Iteration 3 — final native evidence reconciliation

### Metadata, scope and authority

- Date: 2026-11-09.
- Reviewer: Codex, independently dispatched for the final documentation and blog review.
- Reviewed content commit: `33654e76715810c6fe172cc5a60f4e5833c94279`.
- Reviewed implementation authority: [the approved final slice review](review-slice.md), including the bounded runtime correction through `060359bc1cf88c78252e9d789396821413b41d7e`.
- Verdict: **APPROVED**, with no open documentation finding.

This iteration reviews the complete current [operator guide](../../../../website/content/docs/how-to/run-verification-on-metal.mdx), [engineering post](../../../../website/content/blog/sharing-a-metal-box-without-sharing-test-state.mdx), the non-normative DELIVER `[REF]` disposition in [feature-delta.md](../feature-delta.md), and [the final documentation validation record](documentation-validation-final.md). It supersedes the earlier documentation verdict's pending-native statements for this content version. Iterations 1 and 2 remain unchanged as historical reviews of their recorded sources. D1 remains closed.

The reviewer read the applicable `AGENTS.md`, `.claude/rules/finalize.md`, `.claude/rules/domain-naming.md`, `website/CLAUDE.md`, and the repository [technical-writer skill](../../../../.claude/skills/technical-writer/SKILL.md). The skill governed the factual, context, writing and subtractive review throughout both pages and this review artifact. The finalization rule expressly permits the blog's engineering history while requiring the guide to describe the current system.

Factual checks used the approved final slice review and its source/error mapping, the source-qualified native receipts below, the original documentation review, the resource-alternative and construction findings, the socket-collision disposition, and the retained formal summary. A bounded read of the current durable helper checked the queued-EOF mechanism. This documentation review does not duplicate the completed full implementation audit or execute another native campaign.

### Findings and factual assessment

**No new blocking or non-blocking documentation finding.** The updated claims match the recorded implementation and evidence boundaries.

| Area | Evidence and assessment |
| --- | --- |
| Operator commands and ownership | The guide retains ordinary shared maintenance admission, checkout-specific exclusion, exact nextest placement, explicit `metal run --exclusive`, exclusive shell/provisioning, and the Unix-identity meaning of `--no-sudo`. Preparation, sync/no-sync, collection and pull remain distinct operations. Commands do not imply arbitrary ordinary shell authority or a host fallback. |
| Identifier, registration and START | The guide preserves D1's correction. A locally emitted run ID precedes durable registration. Registration precedes transfer/preflight. START establishes command start and the subsequent controller-loss guarantee. |
| Typed runner outcomes | The added `controller_eof_before_start`/75 entry describes loss before START, including queued admission. Current `lease-holder.sh` observes the control pipe while its real flock waiter is queued, cancels and reaps that invocation's waiter on EOF, and retains the existing error mapping. The remaining 75/78/70/74 diagnostics preserve their approved semantic distinctions. Actual command results remain separate. No new error representation or runtime behavior is proposed. |
| Initial formal results versus later discovery | The retained formal summary contains 16 holds and two intentional hazard violations, matching the post. The post introduces Quint, scopes fairness and termination, and expressly says those early checks found no unknown production bug. It distinguishes the native socket collision and recording-codec infrastructure from later actual-owner conformance. |
| Queued-controller defect and correction | [The native owner receipt](native-evidence/owner-queued-eof-verification-2026-11-09-090213Z/README.md) records the actual independent exclusive maintainer, pre-START ordinary EOF, real queued lock wait, exact failing seed `0x8e8e19a4`, and correction `8b1dd117`. The exact seed, both existing controller-loss controls and fresh randomized seed `0x83bfe04` passed. The post attributes this defect to real-owner conformance without changing the model or claiming a fresh all-fifteen campaign. |
| Worktree and survivor proof | [The November 9 receipt](native-evidence/feature-native-closeout-2026-11-09-104319Z/README.md) reports worktree/admission drivers 0 at root `8914e5ee` and peer `06f25ce2`, with matching recorded feature hashes. The post accurately describes actual shared maintenance locks, distinct checkout owners, same-CID VMM overlap in distinct namespaces, the first supervisor's retained loss result and cleanup, the unchanged survivor, its 86 passing guest tests, and exact physical snapshots. It does not move these executions to the later preparation-correction source. |
| Admission proof | The same receipt separates the real public guest timeout from the approved E4 controlled-prior-owner witness. The post retains that controlled-owner boundary and describes maintenance progress only after the prior owner's finalizer, while the ordinary waiter remains unadmitted. It does not claim a full public guest stop window or a different barrier. |
| Composite placement and execution modes | [The consolidated receipt](native-evidence/placement-final-complement-2026-11-09-114229Z/README.md) separately attributes the original prefix, corrected archive-marker assertions, invalid-mode result, [four final mode executions](native-evidence/login-mode-cache-argument-correction-2026-11-09-113245Z/README.md), and final observer. The post and DELIVER disposition preserve these sources, the approved `artifact_scope_mismatch`/64 cause, the retained historical failures, and all four privileged/login and sync/no-sync passes at frozen runtime source `060359bc`. Neither claims one uninterrupted full driver pass at one SHA. |
| Final complement | The final observer returned 0 with no forbidden physical deltas, no owned processes and no owned roots. The post states that only the two paired finite IPv6 RA expiry countdowns changed in raw snapshots, under the existing approved comparison. It does not convert raw snapshots into exact byte equality or weaken a zero-state invariant. |
| Known unrelated owner-burst failure | [The preimplementation receipt](native-evidence/owner-burst-preimplementation-baseline-2026-11-09/README.md) records the same unchanged one-second/1,000-flow assertion failing with 78 consumed ends after successful preparation. [The current original-fixture comparison](native-evidence/owner-burst-current-branch-comparison-2026-11-09/README.md) also fails independently of the new dispatcher. The post and disposition retain that failure and the single omitted body without manufacturing owner-cell GREEN. The user's instruction, “stop focusing on failures that are not related to this feature !!!!!”, excludes investigation or remediation of this known baseline failure. It is no documentation blocker and no waiver of P1–P5. |
| Normative contract and completion status | Independent byte comparison confirms that every byte before the DELIVER `[REF]` disposition matches the content commit's parent. P1–P5 are unchanged. The disposition accurately records final slice approval and pending documentation approval at its authored time. The validation record explicitly says the author does not approve or finalize the feature. |

### Decision narrative and technical writing

The post explains the concrete lease constraint before discussing mechanisms. It covers the serious alternatives and their actual dispositions: branch ownership and automatic cache adoption, an insufficient numeric hash, the superseded 224-window allocator, a whole-run outer guest, a persistent guest broker and a batch-result interpreter. It does not claim a successful or uncompleted alternative experiment proved the alternative impossible. The selected kernel boundary is connected to CID/listener ownership and lifetime. Cold caches, per-attempt guest boots and the finite ordinary-command grammar have explicit costs.

The consequential discoveries remain distinct: exact Linux local-mode behavior, actual socket collisions in six Rust identities, the native fixture's parent-traversal failure in both namespace contexts, nextest timeout bypassing Drop, later queued-controller EOF, the archive assertion's wrong typed marker and login-mode preparation permissions. Each passage identifies the component and intended behavior before explaining its result. The post supplies enough context to understand these examples without opening their supporting links. It does not become a chronological account of every delivery task.

The guide describes implemented usage and current limitations. The post uses the historical account allowed by the finalization rule. Both preserve the exact checkout, invocation, transient owner, maintainer, selected attempt, guest kernel, host-launch resource and collector boundaries. Both keep named nested correctness credit separate from direct-native launch evidence. No speedup or throughput gain is claimed. Lack of reboot recovery and automatic pruning remains explicit.

The writing uses concrete owners and active verbs, introduces vsock/CIDs, nextest ownership, the transient service and Quint before relying on them, and names real tradeoffs. Source links support the explanations instead of replacing them. No authored semicolons appear in either MDX file. Equal-weight subtractive review found no necessary additional mechanism, warning, example, historical task list or troubleshooting matrix. No style preference is promoted into a required change.

### Independent bounded checks and validation limits

The reviewer independently checked:

- The current guide, post and feature-delta bytes match commit `33654e767` and the three SHA-256 values retained by the author.
- All **12** authored page links resolve to local routes or the exact retained Git objects. Both GitHub heading anchors exist at their pinned source. This proves local target correctness, not public availability of an unpushed commit.
- The guide occurs exactly once in how-to navigation. Regenerated Fumadocs server imports and browser collection maps contain both pages. Blog frontmatter contains its required metadata, November 9 date and no `draft: true` override.
- The normative feature-delta prefix is byte-identical to the reviewed commit's parent. All local links in the final disposition and validation record resolve.
- Scoped committed-diff whitespace checks pass. All seven unrelated dirty file hashes still match the author's before/after preservation snapshots.

The initial ad hoc discovery check also looked for literal page maps in generated `dynamic.ts`. That file loads configuration dynamically and contains no such maps. The reviewer corrected that checker assumption and verified the actual generated `server.ts` imports and `browser.ts` maps. It was not a website failure or a content change.

The reviewer read the retained final typecheck and lint logs and the scoped validation log under `.context/metal-documentation-final-20261109/`. Their contents support the author's recorded final collection-generation/typecheck, lint, links, discovery and scope results. This reviewer did not independently rerun typecheck or lint. The short stdout logs do not independently encode process exit values, so exit 0 remains attributed to the author's explicit execution record.

The original review's Next/OpenNext build, one-index and focused Worker observations retain their original source and attribution. No final rendered-HTTP, Worker, full build or broad acceptance pass is claimed. The existing search-ranking failure in `test:blog`, `test:mcp` and `test:mcp:analytics` remains recorded as a reproduced baseline limit. Later analytics assertions remain unexecuted. The final validation record accurately distinguishes these earlier observations from its current scoped checks. No unrelated ranking investigation or repeat failure campaign is required.

No production source, test, model, native evidence, website prose, navigation, validation record, normative design or DES event was changed. No Metal, SSH, native, model, mutation, Rust-test, deployment, publication, push or commit command was run. This reviewer owns only the appended review iteration in this file.

### Remediation disposition and current final documentation verdict

| Finding | Disposition |
| --- | --- |
| D1 — local identifier versus durable registration | **CLOSED**, unchanged from iteration 2 and preserved in the reviewed guide. |
| Final evidence reconciliation | **APPROVED**, no new documentation finding or required remediation. |

**APPROVED — final operator documentation, engineering post and recorded validation disposition at `33654e767`.** Factual accuracy, consequential decision story, technical-writer compliance throughout, limitations, links, navigation, blog discovery and validation honesty satisfy the final documentation gate. The source-scoped composite evidence and single pre-existing burst-body exception remain explicit. This verdict completes the documentation review. The orchestrator still owns finalization bookkeeping. It does not authorize publication or deployment.
