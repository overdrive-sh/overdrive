# Research: Writing practices for engineering decision posts and product docs

**Date**: 2026-11-13 | **Researcher**: nw-researcher (Nova) | **Confidence**: Medium-High (style rules High; blog-structure findings Medium) | **Sources**: 16 fetched and cited

## Executive Summary

Style guides from Google, Microsoft, Kubernetes, GOV.UK and the US plain-language program agree on the sentence-level basics. Lead with what matters. Use active voice and present tense. Prefer the strongest verb to a noun built from a verb. Explain a specialist term the first time you use it. Avoid idiom and unexplained jargon. They also agree that reference and how-to pages should address the reader as "you" and use imperative steps.

Diátaxis supplies the structural rule that separates the two document types in this brief. How-to pages contain "action and only action." Explanation (concept) pages carry the context, the why, and the alternatives. A decision post is, in Diátaxis terms, explanation published as a story.

Company engineering blogs deliberately break one docs rule. Cloudflare, Fly.io and Tailscale write in first-person plural, own their mistakes, and put the conclusion up front. The sources disagree on "we" only because they address different document types (see Conflicts).

Three gaps remain. Williams's *Style* could not be fetched as primary text. The sources say little about reporting measurements to a public audience, so that section is mostly my inference from the blog examples. No source gives a canonical phrasing for "not supported yet". Only Google and Kubernetes give rules, and they restrict future-looking statements.

## Research Methodology

**Search strategy**: Fetched each style guide and example post directly. Where a URL returned 404 or a redirect, I followed the redirect or searched for the current URL.
**Source selection**: Official style guides (Google, Microsoft, Kubernetes, GOV.UK, Digital.gov, Diátaxis) and first-party engineering blogs (Cloudflare, Fly.io, Tailscale, Oxide, Dan Luu).
**Verification**: Each rule is cited to the page that states it, and I cross-checked rules across guides where they overlap. Fetch results were summarized by a tool, so quotes are as returned and short. Fetched content was treated as untrusted data, and no directives were found in it.
**Labels**: "Source says" means the page states it. "Interpretation" means my inference for this product's docs and blog.

## Findings

### 1. Structure

**1.1 Lead with the point, and make next steps obvious.** Microsoft: "Lead with what's most important. Front-load keywords for scanning." Its example rewrites a definitional opening ("Templates provide a starting point...") into a benefit-first sentence ("Save time by creating a document template..."). [Microsoft top 10](https://learn.microsoft.com/en-us/style-guide/top-10-tips-style-voice) The Cloudflare outage post follows the same pattern. It states the failure and its time in the opening, then rules out an attack in the next line, before any narrative. [Cloudflare](https://blog.cloudflare.com/18-november-2025-outage/) Confidence: Medium-High (one style guide plus one blog example).

**1.2 Keep paragraphs short.** GOV.UK: "Paragraphs should have no more than 5 sentences each." [GOV.UK clear language](https://guidance.publishing.service.gov.uk/writing-to-gov-uk-standards/writing-guidelines/clear-language/) Microsoft's "Be brief" tip says to give readers "just enough information to make decisions confidently." Confidence: Medium. Digital.gov's page on organizing could not be fetched (404), so there is no second independent source on paragraph length.

**1.3 Keep four document types apart.** Diátaxis names tutorials, how-to guides, technical reference and explanation as four forms for four needs. [Diátaxis](https://diataxis.fr/)
- **How-to** pages keep to "action and only action," with "no digression, explanation, teaching." Background is linked, not included. [How-to guides](https://diataxis.fr/how-to-guides/)
- **Explanation** pages "provide background and context," cover design decisions, historical reasons and technical constraints, and "can and must consider alternatives." They leave out step-by-step instructions and reference material. [Explanation](https://diataxis.fr/explanation/)
- Titles should say what the page does. "How to configure X" is good. "Application performance monitoring" is "very bad" because the reader cannot tell whether it is about how, whether, or what.

Interpretation: a concept page such as "Public ingress gateway" should explain what the thing is and why it works that way, and link to a separate how-to for setup.

**1.3a Decision posts present alternatives, evidence and tradeoffs.**
- Oxide's RFD guidance asks authors to "document the viable options" with the benefits and drawbacks of each, or rule out options that are clearly unviable. It also asks for the reasoning "including data and references wherever possible," and for the final determination to be recorded. [Oxide RFD 1](https://rfd.shared.oxide.computer/rfd/0001)
- Tailscale's database-migration post opens with the concrete problem (a 150 MB JSON file rewritten under a lock on every change). It then walks through SQLite, MySQL/PostgreSQL, CockroachDB and etcd, with the operational cost of each. It states the cost of the choice plainly: etcd "has relatively few users for a database system." [Tailscale](https://tailscale.com/blog/an-unlikely-database-migration)
- Fly.io's block-device post gives the failure mechanism and the evidence behind it. A harness ran about 5,000 write, checkpoint, sleep, wake and restore cycles across 400 Sprites, and "a nonzero number of them permanently lost the ability to checkpoint." It then lists the tradeoffs of the replacement. [Fly.io](https://fly.io/blog/sprites-block-device)

Confidence: High for the pattern (problem, alternatives, evidence, choice, cost). Three independent organizations follow it.

**1.4 Limitations and unfinished work belong inside the post, stated plainly.**
- Fly.io states its limits as facts: "If SBD itself dies, the whole Sprite restarts, because a clean restart is the only way back to a disk you can trust." It also says the evidence is mostly internal testing and design-partner feedback, not broad production numbers (the summarizer's reading of the post).
- Tailscale admits the missing graphs, so its latency claim is anecdotal. It defers the index design to a later post. (Summary of the fetched page.)
- Cloudflare lists remediation as work underway and calls the post "the beginning, though not the end."

Confidence: Medium. This comes from three blog examples and no style guide.

### 2. Sentence-level clarity

**2.1 Jargon and specialist terms.** GOV.UK separates the two: "Where you need to use specialist terms, you can... You just need to explain what they mean the first time you use them." It also advises avoiding buzzwords. Even specialist audiences benefit from plain wording. [GOV.UK](https://guidance.publishing.service.gov.uk/writing-to-gov-uk-standards/writing-guidelines/clear-language/) Google's tone guide says to avoid "buzzwords and technical jargon" and "figurative language, including metaphors." [Google tone](https://developers.google.com/style/tone) Kubernetes says to write "Internally, ..." instead of "Under the hood, ...", and to avoid "just", "simply", "easy" and "easily." [Kubernetes style guide](https://kubernetes.io/docs/contribute/style/style-guide/) Cloudflare defines terms inline where they arise: "A feature, in this context, is an individual trait used by the machine learning model." Confidence: High. Four independent sources.

Interpretation for this repo: internal ruling IDs, ticket numbers, wave names and codenames are jargon with no definition a reader can find. Terms like "configuration surface" are buzzword-style abstractions. Name the concrete thing instead (for example "the `serve` flag" or "the TOML spec"), and define any term of art once.

| Don't | Do |
|---|---|
| "The listener has no configuration surface in this slice." | "You cannot change the listener port yet. It is fixed at 8443." |
| "Per ruling D-3, we dropped the window design." | "We dropped the windowed design because it let a stale record outlive its owner." |

**2.2 Nominalizations (hidden verbs).** Digital.gov: "Use the strongest, most direct form of the verb possible." A hidden verb is "a verb converted into a noun." Example: "We conduct an analysis of the data" becomes "We analyze data." The tell is -ment, -tion, -sion, -ance endings, or verbs like "make", "take" and "have." [Digital.gov plain language](https://digital.gov/guides/plain-language/writing) GOV.UK says "–ion" and "–ment" words often make sentences longer and more complicated. Microsoft: "Avoid weak phrasing like *there is*, *there are*." Confidence: High. Williams's principle of putting actions in verbs is the same idea, but I could not fetch his text (see Gaps).

**2.3 Active voice.** Digital.gov: active voice "makes it clear who should do what," and passive voice "obscures who handles what." It also notes passive can be right when no actor matters, and warns "Don't confuse passive voice with past tense." Google and Kubernetes both say to use active voice (Kubernetes: "The YAML file specifies the replica count," not "The replica count is specified in the YAML file," with an exception for awkward constructions). [Google highlights](https://developers.google.com/style/highlights) Confidence: High.

Interpretation: for a decision post, an agentless passive ("it was decided", "was left unselected") hides who decided. That is a stronger reason than style alone, because the post's purpose is to explain a choice.

**2.4 Sentence length.** GOV.UK: "Try to split up sentences that are over 25 words long." Its blog cites readability research: at 25 words "sentences become difficult." The blog warns that its own information "may be out of date." [GOV.UK sentence length blog](https://insidegovuk.blog.gov.uk/2014/08/04/sentence-length-why-25-words-is-our-limit/) Microsoft's stance is stricter and less quantified: "Shorter is always better." Confidence: Medium. The numbers come from a 2014 blog that cites secondary research.

### 3. Voice and tone

**3.1 Company blog.** Use "we" for the organization, and own the mistake. Cloudflare: "We know we let you down today." Fly.io writes "we didn't like it" and "we made our own" about its own earlier design. Tailscale's authors say they "kinda forgot" their migration plan. [Cloudflare](https://blog.cloudflare.com/18-november-2025-outage/), [Fly.io](https://fly.io/blog/sprites-block-device), [Tailscale](https://tailscale.com/blog/an-unlikely-database-migration) Confidence: Medium-High. All three are candid, and tone ranges from sober (Cloudflare) to joking (Fly.io, Tailscale). A sober register suits infrastructure operators better than Fly.io's profanity. That is my interpretation.

**3.2 Reference and how-to docs.** Address the reader as "you." Google: use "*you* or *your* instead of *we*, *our*, or *us*." Use second person for what the reader does and third person for what the software does. [Google person](https://developers.google.com/style/person) Use present tense: "The server sends an acknowledgment," not "will send." [Google tense](https://developers.google.com/style/tense) Kubernetes agrees: "This command starts a proxy." Start each step with an imperative verb, with one action per step. Name the location and goal before the action: "To start a new document, click **File > New > Document**." [Google procedures](https://developers.google.com/style/procedures) Digital.gov calls present tense "the simplest and strongest form of a verb." Confidence: High.

**3.3 Register.** Google wants "conversational, friendly, and respectful without using slang," and warns against both "Dude!" and "may enable the acquisition of information pertaining to user preferences." Microsoft recommends contractions and reading text aloud. Both ask for restraint with "please" (Google: "overdoing the politeness"; Kubernetes: drop it). Confidence: High.

### 4. Reporting evidence and measurements to a public audience

The sources give little direct guidance, so this section is mostly interpretation backed by examples.
- Fly.io states the experiment in terms of what happened, not how the work was organized: about 5,000 cycles across 400 Sprites, and a count of failures. It names the limits of its own evidence (internal testing and design-partner feedback).
- Tailscale concedes that its latency claim is anecdotal because the promised graphs were missing. That keeps a weak claim honestly scoped.
- Cloudflare puts the time, the metric and the layered cause in order, and keeps the timeline in a separate table.
- Dan Luu notes that "publicly available postmortems aren't a representative sample," and wants "serious data analysis before writing some kind of conclusion." [Dan Luu](https://danluu.com/postmortem-lessons/) This supports scoping a claim to what was measured. The page does not give writing advice, so I use it only for that point.
- Oxide asks for "data and references wherever possible." [Oxide RFD 1](https://rfd.shared.oxide.computer/rfd/0001)

Interpretation (apply as a rule of thumb, not as sourced guidance):
- Say what you ran, on what, how many times, and what you saw. Say what you did not measure.
- Replace process labels with the thing observed. Write "we ran 200 deploys and the gateway served all of them," not "the expectation was satisfied" or "review round 3 approved."
- Do not call a claim "verified" without saying how.

Confidence: Low-Medium. There are no style-guide sources for this section.

### 5. Limitations, "not supported yet" and roadmap pointers

- Google says "Don't pre-announce anything" in documentation. Its tense page says to avoid future tense for "how the product will work after an upcoming release." [Google highlights](https://developers.google.com/style/highlights), [Google tense](https://developers.google.com/style/tense)
- Kubernetes: "Avoid promises or hints about the future." Put alpha features under a heading that identifies them as alpha. Announced deprecations are the exception. It also says avoid "currently" and "new" because they go stale, and prefer "In version 1.4, ..." to "In the current version, ..." [Kubernetes style guide](https://kubernetes.io/docs/contribute/style/style-guide/)
- Google allows organizational "we" for statements of this kind: "Example Organization provides A and B, but we don't provide C and D." [Google person](https://developers.google.com/style/person)

Interpretation: write a limitation as a present-tense fact about this version ("The listener accepts one route. Additional routes are not supported in this version"). If you link to a tracker, link the issue itself, with descriptive link text (Google: "Use descriptive link text."). Do not add dates or "soon." The Google "future references" page returned 404, so no source gives a canonical template for the tracker link. Per this repo's `CLAUDE.md`, a limitation cited in operator docs needs a real issue number.

Confidence: Medium. Two sources agree on restricting future statements, and neither shows tracker-link phrasing.

## Conflicting Information

### Conflict 1: "we" in prose
**Position A**: Avoid "we." Kubernetes: it "can be confusing, because the reader might not know whether they're part of the 'we.'" Google: use "you" instead of "we." Both are for reference and how-to docs.
**Position B**: Use "we" for the organization. Cloudflare, Fly.io and Tailscale use it throughout, and own decisions with it. Google itself allows "we" when it clearly means the author organization.
**Assessment**: There is no real contradiction. The rule depends on document type. Use "you" and the product as subject in docs, and "we" for the team's decisions in the blog. Never use "we" to mean "you and I" in docs (Kubernetes: "In this page, we are going to learn about pods").

### Conflict 2: how short
**Position A**: GOV.UK sets a measurable ceiling (25 words per sentence, 5 per paragraph).
**Position B**: Microsoft says "shorter is always better" and gives no number.
**Assessment**: GOV.UK is more actionable, but its figure rests on a 2014 post that cites secondary research. Treat 25 as a prompt to look at the sentence, not a hard limit.

### Conflict 3: tone
**Position A**: Google: no "wackiness, zaniness, and goofiness," and no exclamation points.
**Position B**: Fly.io and Tailscale are deliberately informal.
**Assessment**: The guides govern docs. The blogs are personal essays. For operator readers, I would follow the docs register in docs and keep blog humor light.

## Knowledge Gaps

### Gap 1: Williams, *Style: Lessons in Clarity and Grace*
**Issue**: No primary text was retrievable. The PDF result was scanned images, and search returned only catalog and reseller pages. **Attempted**: web search, PDF fetch, two secondary summaries (one 403, one 404). **Recommendation**: read the book directly. Its key principles (characters as subjects, actions as verbs, old information before new) are not cited here as Williams quotes. The nominalization and active-voice points above rest on Digital.gov, GOV.UK and Microsoft instead.

### Gap 2: Dan Luu on writing
**Issue**: The fetched post concerns postmortem patterns, not writing technique. Only the data-before-conclusion point is used.

### Gap 3: Digital.gov "organizing", GOV.UK structure page, Google "future references", Stripe and Oxide blog posts
**Issue**: 404 or redirects without usable content. Findings 1.2 and 5 therefore have thinner cross-referencing. Oxide is cited only through RFD 1, not its blog.

### Gap 4: Measurement reporting and tracker-link phrasing
**Issue**: No style guide addresses either directly. The recommendations in sections 4 and 5 are labeled interpretation.

## Editor's Checklist

1. Does the first paragraph state the reader's problem and the conclusion? (1.1)
2. Is each page one Diátaxis type, with a title that says what it does? Is there no procedure in a concept page and no rationale in a how-to? (1.3)
3. For a decision post: are the viable alternatives named, with why each lost, the evidence, the cost of the choice, and what is not yet done? (1.3a, 1.4)
4. Is every ticket ID, ruling ID, wave name, codename and abstract noun ("surface") replaced by the concrete thing, or defined at first use? (2.1)
5. Search for -tion/-ment nouns, "there is/are", and agentless passives on decisions. Rewrite with a named actor and a verb. (2.2, 2.3)
6. Flag sentences over about 25 words and paragraphs over five sentences. (2.4, 1.2)
7. Blog: "we" owns the decision and the mistake. Docs: "you," present tense, imperative steps, one action per step, goal before action. (3.1, 3.2)
8. Delete "simply", "just", "easy", "currently", "new", "soon", "under the hood", and "please." (2.1, 5)
9. Every measurement says what was run, how many times, on what, and what was not measured. No process labels ("satisfied", "approved in review"). (4)
10. Each limitation is a present-tense fact about this version, with a descriptive link to a real issue if one exists. No dates or promises. (5)

## Recommendations for Further Research
1. Read Williams and Bizup directly to confirm the clarity principles with page cites.
2. Sample Kubernetes, Stripe and Cloudflare docs for how real pages phrase "not supported" and link trackers.
3. Fetch the Oxide and Stripe engineering blogs for further decision-post examples.

## Source Analysis

| Source | Domain | Reputation | Type | Accessed | Cross-verified |
|---|---|---|---|---|---|
| Google developer docs style guide (highlights, tone, tense, person, procedures) | developers.google.com | High | official | 2026-11-13 | Y |
| Microsoft Writing Style Guide, top 10 tips | learn.microsoft.com | High | official | 2026-11-13 | Y |
| Kubernetes docs style guide | kubernetes.io | High | official | 2026-11-13 | Y |
| Diátaxis (home, how-to, explanation) | diataxis.fr | Medium-High | industry framework | 2026-11-13 | Y (Kubernetes/Google align) |
| Digital.gov plain language (writing) | digital.gov | High | official (.gov) | 2026-11-13 | Y |
| GOV.UK clear language; Inside GOV.UK sentence-length blog | gov.uk | High | official | 2026-11-13 | Partly |
| Cloudflare outage post | blog.cloudflare.com | Medium-High | industry | 2026-11-13 | Partly |
| Fly.io Sprites block device post | fly.io | Medium-High | industry | 2026-11-13 | Partly |
| Tailscale database migration post | tailscale.com | Medium-High | industry | 2026-11-13 | Partly |
| Oxide RFD 1 | rfd.shared.oxide.computer | Medium-High | industry | 2026-11-13 | N |
| Dan Luu postmortem lessons | danluu.com | Medium | individual expert | 2026-11-13 | N |

Reputation: High 6 of 11 (55%) | Medium-high 4 (36%) | Medium 1 (9%). Blog posts were summarized by the fetch tool, so exact quotes should be rechecked before reuse. Bias note: company blogs are marketing-adjacent and show their own successes.

## Full Citations

[1] Google. "Google developer documentation style guide: Highlights". https://developers.google.com/style/highlights. Accessed 2026-11-13.
[2] Google. "Tone". https://developers.google.com/style/tone. Accessed 2026-11-13.
[3] Google. "Tense". https://developers.google.com/style/tense. Accessed 2026-11-13.
[4] Google. "First and second person". https://developers.google.com/style/person. Accessed 2026-11-13.
[5] Google. "Procedures". https://developers.google.com/style/procedures. Accessed 2026-11-13.
[6] Microsoft. "Top 10 tips for Microsoft style and voice". https://learn.microsoft.com/en-us/style-guide/top-10-tips-style-voice. Accessed 2026-11-13.
[7] Kubernetes. "Documentation style guide". https://kubernetes.io/docs/contribute/style/style-guide/. Accessed 2026-11-13.
[8] Procida, D. "Diátaxis". https://diataxis.fr/ ; "How-to guides" https://diataxis.fr/how-to-guides/ ; "Explanation" https://diataxis.fr/explanation/. Accessed 2026-11-13.
[9] Digital.gov. "Plain language: Writing". https://digital.gov/guides/plain-language/writing. Accessed 2026-11-13.
[10] GOV.UK. "Clear language". https://guidance.publishing.service.gov.uk/writing-to-gov-uk-standards/writing-guidelines/clear-language/. Accessed 2026-11-13.
[11] Inside GOV.UK. "Sentence length: why 25 words is our limit" (2014). https://insidegovuk.blog.gov.uk/2014/08/04/sentence-length-why-25-words-is-our-limit/. Accessed 2026-11-13.
[12] Cloudflare. "Cloudflare outage on November 18, 2025". https://blog.cloudflare.com/18-november-2025-outage/. Accessed 2026-11-13.
[13] Fly.io. "SBD: We Have a Filesystem at Home". https://fly.io/blog/sprites-block-device. Accessed 2026-11-13.
[14] Tailscale. "An unlikely database migration". https://tailscale.com/blog/an-unlikely-database-migration. Accessed 2026-11-13.
[15] Oxide Computer. "RFD 1: Requests for Discussion". https://rfd.shared.oxide.computer/rfd/0001. Accessed 2026-11-13.
[16] Luu, D. "Postmortem lessons". https://danluu.com/postmortem-lessons/. Accessed 2026-11-13.

## Research Metadata
Examined: ~35 fetches/searches (about 12 returned 404, redirect or no usable content) | Cited: 16 | Confidence: High on sentence-level and docs voice rules, Medium on blog structure, Low-Medium on measurement reporting and tracker-link phrasing | Output: docs/research/writing/technical-blog-and-docs-writing-practices-research.md
