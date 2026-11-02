# Document features before finalization

After a feature's implementation and review are approved, update the website
documentation and write a blog post before finalizing the feature. Both are
required completion gates, not follow-up tasks.

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
  evidence, completeness of the decision narrative and reader context below,
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

## Introduce context before relying on it

Write for a reader who has not followed the feature's development or read its
other posts, ADRs, or internal discussions. Define the main subject briefly at
the opening, then introduce any additional unfamiliar mechanism before using
it in an example or argument. An introductory definition does not supply the
context for every later section.

Before discussing an experiment, failure, or replacement, establish which
system and component are involved, what the mechanism does, and what behavior
it was intended to provide. Name the design or approach before referring to
"the design," "the initial approach," or "the replacement." Explain terms
needed to understand the failure when they first appear.

Links provide supporting detail. They do not replace the minimum explanation
needed to understand the passage without reading another article. Keep that
explanation brief and concrete rather than inserting an unrelated tutorial.

The reviewer must check this throughout the post, not only at the opening.
For each new example or mechanism, verify that the preceding text identifies
its subject, purpose, and relevant behavior. Unexplained references block
approval even when the technical claims are accurate and their sources exist.
