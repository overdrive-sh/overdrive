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
  evidence, the decision story below, compliance with the technical-writer skill
  throughout the content, current limitations, links, and validation results.
  Resolve blocking findings and obtain reviewer approval before finalization.
- Check in the documentation and post, and include their paths and validation
  results, reviewer verdict, and review artifact path in the finalization record.

Do not mark the feature finalized while either deliverable is missing or its
required validation or reviewer approval remains unresolved. Deployment and
publication are separate from this gate; completing the content does not
authorize publishing the site.

## The feature post tells its decision story

Before writing the blog post, read the feature documents, accepted ADRs, spike
findings and verification evidence. The post is an engineering blog post as the
`technical-writer` skill defines it (principle 12): the motivating problem, the
serious alternatives and why each lost, why the chosen approach fits and what it
costs, and what the delivered work proves and leaves unmeasured. Its voice and
vocabulary follow the same skill: "we" for the team's decisions, no agentless
passive for choices, and none of the workflow's people, roles, phase names, IDs
or process labels in reader-facing prose.
