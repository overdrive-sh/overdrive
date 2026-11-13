# Document features before finalization

After a feature's implementation and review are approved, update the website
documentation and write a blog post before finalizing the feature. Both are
required completion gates, not follow-up tasks.

- Capture and review the feature's EDD expectations (`verification/expectations/`,
  `.claude/rules/verification.md`) before writing any docs page or blog post that
  mentions verification. Writing comes after the evidence. A post must never
  report a feature's own expectation as pending or uncaptured: capture it first.
  If it truly cannot be captured, stop and ask the user. Do not publish around it.
- Update the affected pages under `website/content/docs/` to describe the
  implemented behavior, operator-facing usage, and current limitations.
- Write a post under `website/content/blog/` explaining the feature, its
  rationale, and the engineering journey. Product documentation describes the
  current system; the blog may discuss the decisions and discoveries that led
  to it.
- Follow `website/CLAUDE.md` and use the repository's `technical-writer` skill
  for all website prose. Ground claims in the reviewed implementation and
  verified evidence. Distinguish measured results from goals and deferred work.
- Validate the content with the applicable existing website checks, including
  links, navigation, and blog discovery. Resolve failures caused by the change
  and record any pre-existing or environmental failures accurately.
- Dispatch an independent reviewer agent for the documentation and blog post.
  The reviewer checks factual accuracy against the implemented behavior and
  evidence, completeness of the decision narrative below, compliance with the
  technical-writer skill throughout the content, the voice rules below,
  current limitations, writing quality, links, and validation results.
  Resolve blocking findings and obtain reviewer approval before finalization.
- Check in the documentation and post, and include their paths and validation
  results, reviewer verdict, and review artifact path in the finalization record.

Do not mark the feature finalized while either deliverable is missing or its
required validation or reviewer approval remains unresolved. Deployment and
publication are separate from this gate; completing the content does not
authorize publishing the site.

## Tell the feature's decision story

Read the feature documents, accepted ADRs, spike findings, and verification
evidence before writing the blog post. Do not reconstruct the journey from
memory or describe only the final implementation.

The post must explain:

- The concrete problem that motivated the feature, including the observed
  failure or measured constraint and why it blocked the intended outcome.
- The serious alternatives investigated, what each experiment established,
  and why each alternative was rejected, deferred, or left unselected.
- Why the selected approach addresses the original constraint, its costs and
  tradeoffs, and the discoveries that materially changed the design.
- What the delivered implementation proves, what remains unmeasured, and its
  current limitations.

Keep evidence scoped precisely. A limit on one bridge is not a system-wide
interface limit. An alternative that passed its experiment must not be called
nonfunctional merely because it was not selected. Explain the actual selection
criteria. Lead with the motivating problem and decision, then use implementation
details to explain the journey. Select the consequential experiments rather than
recounting every delivery task.

## Write as the team, in the active voice

The blog and docs speak for Overdrive. The decisions in them are the team's
decisions, so the post says so.

- **Write "we".** "We chose Git-worktree identity", "we kept the existing
  suite", "we dropped the window design". The team decided; the post owns it.
- **Never write "the user".** "The user selected", "the user chose", "the user
  ruled" leak the agent workflow that produced the feature into public prose.
  The person approving a decision in a design session is part of the team, not
  a third party the reader has never met. The same goes for "the agent", "the
  reviewer", "the architect", "the crafter", wave names (DISCUSS, DESIGN,
  DISTILL, DELIVER), roadmap step IDs and review rounds: they describe how the
  work was organised, not what was decided or why.
- **Name who acted; no agentless passive for decisions.** "It was chosen",
  "it was superseded", "was left unselected", "were not credited", "has been
  measured" hide the decision-maker. Rewrite as "we chose", "we dropped it
  because", "we did not count", "we have not measured". The passive is fine
  for things no one decided ("the device is released when the VMM exits");
  it is wrong for choices.
- Translate design-artifact vocabulary rather than copying it. Feature
  documents record "the user approved D-3 on 2026-11-02"; the post says what
  we decided and why.

**Symptoms during review:** "the user" anywhere in a post or docs page;
"selected design", "was left unselected", "it was decided", "it was
superseded" for a choice the team made; wave names, step IDs or agent roles
in reader-facing prose.
