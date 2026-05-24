---
name: technical-writer
description: Write technical prose in the plain style — direct, concrete, position-taking, free of hedge-words and filler. Use whenever the user asks for technical writing of any kind: articles, blog posts, essays, whitepapers, design docs, documentation, technical explainers, architecture writeups, engineering analysis, design reviews, or any prose intended for technical readers. Also use when revising existing technical writing to make it tighter, clearer, or more confident. Trigger even when the user does not name a style explicitly — if the task is technical prose for a thinking reader, this skill applies.
---

# Technical Writer

A skill for writing technical prose in the plain style: concrete nouns, active verbs, sparing hedges, owned positions, named tradeoffs, varied rhythm, no filler.

The style traces back through Orwell and Hemingway to classical rhetoric's "plain style" — the rhetorical mode that hides its rhetoric. Modern practitioners include Paul Graham, Patrick McKenzie, Bret Victor, Dan Luu, and William Zinsser. The principles below are operational distillations of what makes their writing work.

The hardest thing about this style is that it exposes confused thinking. Vague writing hides confusion; plain writing shows it. When the prose resists being made plain, the underlying thought is usually not finished yet. Sharpen the thought, then write.

---

## Core principles

### 1. Concrete nouns and active verbs do the work

The verb carries the sentence. Qualifiers, adjectives, and noun phrases should support the verb, not substitute for it.

**Do this:** The lease manager rejects the request.
**Not this:** The request is subject to rejection by the lease manager's policy enforcement.

**Do this:** Two agents racing on the same file produce broken merges.
**Not this:** There are situations in which multi-agent concurrent file access can result in suboptimal merge outcomes.

When revising, find the real verb of each sentence. If the verb is "is," "has," "involves," "exhibits," or any other low-content verb, look for the action hiding inside the noun phrase and promote it.

Subject and verb should stay close. Long parenthetical clauses parked between them force the reader to hold the subject in working memory while parsing modifiers, and the verb lands late and weak.

**Do this:** The lease manager rejects stale acquisitions. It handles coordination across regions, including the cross-region replication path.
**Not this:** The lease manager, which handles coordination across regions including the cross-region replication path, rejects stale acquisitions.

When a modifier needs to be there, split the sentence. The "modifier between subject and verb" shape is one of the most common buried-verb patterns in technical writing.

### 2. Be specific, not just concrete

Concrete nouns beat abstract ones. Specific concrete nouns beat generic concrete ones. `Three engineers at Stripe debugged this for six hours` lands harder than `several engineers debugged this for a while`, which lands harder than `personnel investigated the issue`. The first version is concrete *and* specific; the second is concrete but generic; the third is neither.

Specificity does three things at once: it forces the writer to know the actual facts, it gives the reader something to verify or remember, and it signals that the claim isn't being smuggled past on vague-sounding plausibility.

**Do this:** Verification went from forty minutes to under three.
**Not this:** Verification became significantly faster.

**Do this:** Cilium and Tetragon both use little-vm-helper for kernel-matrix CI.
**Not this:** Several similar projects use a comparable approach.

**Do this:** The 2014 Stripe outage was caused by a Postgres long-running transaction holding a lock through a deploy.
**Not this:** Real-world incidents have shown that similar patterns can cause outages.

Watch for the generic-quantifier words: *several, various, many, multiple, some, most, often, typically, generally.* Each one is a place where a specific number, name, or condition was available and got dropped. Sometimes the specifics genuinely aren't known — in which case say so, don't paper over the gap with `several`.

The exception: when generality is the actual claim. `Most distributed systems eventually need a coordination primitive` is a fine sentence because the claim *is* the generality. The failure mode is generic words standing in for facts the writer could have looked up.

### 3. Hedge rarely, hedge meaningfully

If every claim is hedged, no claim reads as confident, and the genuinely uncertain claims are indistinguishable from the safe ones. Hedge only when the uncertainty is real and informative.

**Do this:** This approach reduces verification time by one to two orders of magnitude in typical deployments.
**Not this:** This approach may potentially help reduce verification time, in some cases, by what could be a significant amount.

If a section contains genuine uncertainty (open questions, future work, areas where data hasn't been collected), concentrate the hedges there. The asymmetry between confident sections and uncertain sections is what makes the confident parts credible.

Words to use sparingly: *perhaps, possibly, potentially, arguably, somewhat, generally, often, may, might, could.* Each one should earn its place.

### 4. Take positions, own them

Claims need an owner. The owner can be you, or it can be the mechanism, but it has to be someone. Three modes, in order of force:

1. **Bare declarative** — `Branching is the wrong primitive for agents.`
   Strongest. No hedge, no "I", just the claim. Default to this whenever the claim will land on its merits.

2. **First-person staked** — `I argue branching is the wrong primitive for agents.`
   Use when the surrounding text needs to flag *this sentence is my position, weigh it*. Worth it when (a) you're explicitly framing a thesis against existing consensus, (b) the piece is structured around a position the reader will evaluate, or (c) the claim is a judgment call and honesty demands attribution. First-person is a signpost, not a hedge — it buys scope to defend the claim at the cost of a small amount of confidence.

3. **Impersonal mechanism** — `The lease manager rejects stale acquisitions.`
   For describing how systems work. The system is the agent of the sentence.

The failure mode that sits outside all three: `It could be argued that branching may not be optimal for agent workflows.` Passive voice plus modal hedge plus author-removal in one phrase. Disowns the claim mid-sentence. Replace every instance, but the replacement is usually the bare declarative, not the first-person staked form.

Heuristic: reach for `I argue` (or `I think`, `I propose`) when dropping it would leave the reader unsure whether the next paragraph is a finding or an opinion. Otherwise drop it. The "I" earns its place by clarifying epistemic status, not by softening the claim.

Avoid the inverse failure too: false ownership of mechanism descriptions. `I have designed the lease manager such that it rejects stale acquisitions` puts the author in a sentence that should be about the system. Cut to `The lease manager rejects stale acquisitions.`

**Back claims with one beat of reasoning.** Taking a position isn't bare assertion — when a claim isn't self-evident, the next sentence usually shows why in one beat. Not a proof, not a derivation; a hint. Readers extend more trust to confident claims that are followed by a reason than to confident claims that just sit there.

**Do this:** Pessimistic locking penalizes the common case. Most acquisitions don't conflict, so the lock cost is paid every time to prevent the rare collision.

**Not this:** Pessimistic locking penalizes the common case. We use optimistic concurrency instead.

The second version states a position and a choice but never connects them — the reader has to supply the reasoning. The first version closes the loop. One sentence of backing is usually enough; if a claim needs three, it probably needs its own paragraph.

### 5. Name tradeoffs, commit to a side

Real technical writing involves choices, and choices involve tradeoffs. Name them concretely and say which side the writing takes.

**Do this:** Pessimistic locking penalizes the common case to defend against the rare one. This system uses optimistic concurrency instead.
**Not this:** Various considerations apply when choosing a concurrency strategy, and different approaches have different characteristics.

The second sentence has no content. It describes the *existence* of a tradeoff without telling the reader what it is or what was chosen. This is consultant-speak — the rhetorical mode that conveys having opinions without holding any.

The inverse failure is fake balance: manufacturing a tradeoff to look even-handed when one option is just better. `Approach A is simpler and faster, but Approach B has its own merits` is dishonest if Approach B's merits don't actually weigh against A's win. Real tradeoffs have a real cost on the chosen side; if you can't name the cost, there isn't a tradeoff, there's a choice. State it as a choice and move on.

### 6. Vary rhythm, default short

Most sentences should be short and declarative. Long sentences exist to carry complex thoughts, not to sound sophisticated. A long sentence followed by a short declarative landing creates rhythm and lets key claims punch.

**Example:** The infrastructure agents currently use is a thin agent-shaped wrapper around tooling built for humans. As agent capability and concurrency increase, the mismatch becomes the bottleneck. The response is not to constrain agents to human-shaped workflows but to build infrastructure shaped to how agents actually operate. Git becomes an export format. The agent layer lives underneath.

Notice the rhythm: long, medium, long, short, short. The short sentences land the key claims after the longer ones do the explaining.

### 7. Cut filler aggressively

Filler is any phrase that adds words without adding meaning. Most filler hides in transitions, throat-clearing openings, and academic flourishes.

**Cut these:** *It is important to note that, in order to, at this point in time, due to the fact that, it should be mentioned, as a matter of fact, needless to say, the fact of the matter is.*

**Do this:** To do X, do Y.
**Not this:** In order to accomplish X, it is necessary to do Y.

**Do this:** Now, this matters because...
**Not this:** At this point in time, it is important to note that the reason this matters is...

When in doubt, delete the phrase and re-read the sentence. If meaning is unchanged, the phrase was filler.

### 8. Be honest about uncertainty without performing humility

Confident writing acknowledges limitations. Performative humility undermines the writing without communicating anything real.

**Do this:** This is approximate. Some classes of regression will escape the impact graph and need to be caught by periodic full-suite runs.

**Not this:** Of course, we make no claims about completeness, and reasonable people may certainly disagree with this characterization, which is offered tentatively and subject to revision.

The first version states the limitation as fact. The second performs humility while saying nothing. Honest limitation is content; performative humility is decoration.

---

## Voice: a few edge cases

Principle 4 covers the three modes (bare declarative, first-person staked, impersonal). Two additional notes:

**Avoid the editorial "we" in single-author writing.** It either sounds royal or implies co-authors who don't exist. First-person singular is more honest. The exception is genuinely co-authored work, where "we" refers to the actual authors.

**"We" as the reader-and-writer.** A different "we" sometimes appears in tutorials and explanations: `We start by defining the lease table.` This one is fine in instructional writing where the writer is walking alongside the reader. Don't use it for claims or positions — those are the writer's alone.

---

## Section conventions

A few patterns that appear repeatedly in this style:

**Open with the conclusion, not the setup.** The first paragraph of a section should state what the section argues, not promise that an argument is coming. Readers can skim a piece that opens with conclusions; they can't skim a piece that opens with windups.

**Use headings as content, not as labels.** "Why existing tools fail agents" tells the reader what the section will argue. "Background on existing tools" does not.

**Concede what's true before pushing back.** When disagreeing with a position, acknowledge what the position gets right first, then make the disagreement specific. This is rhetorically stronger and intellectually honest.

**End with implication, not summary.** A summary repeats what was said. An implication tells the reader what to do with it or what comes next.

---

## What this style is not

A few clarifications, because the style can be misapplied.

**Plain style is not casual style.** Plain writing is direct, but it is not chatty, joking, or filler-laden. The directness is a craft choice, not an absence of effort. Avoid "honestly", "to be clear", "real talk" and similar conversational markers in formal technical writing.

**Plain style is not minimalist style.** Sentences can be long when they're carrying complex thoughts. The goal is not "short prose" but "no wasted words." A 40-word sentence with no filler is plain; a 12-word sentence padded with hedges is not.

**Plain style is not opinion-free style.** Quite the opposite. The whole point is to make positions visible and ownable. Writing without positions ("various perspectives exist on this question") is the failure mode the style is designed against.

**Plain style is not academic style.** It avoids the conventions of academic writing — passive voice as default, hedges as politeness, citations as decoration. Use first-person, take positions, cite only what informs the argument.

---

## When revising existing text

Most technical writing benefits from a revision pass focused specifically on these principles. A useful sequence:

1. **Find the weak verbs.** Search for *is, are, was, were, has, have, involves, exhibits, contains, includes, represents, constitutes, comprises*. For each, ask whether a stronger verb is hiding in the noun phrase.
2. **Find the hedges.** Search for *perhaps, possibly, potentially, arguably, somewhat, generally, often, may, might, could, seems, appears, tends to.* Delete the ones that aren't earning their place.
3. **Find the filler.** Search for *it is important to note, in order to, at this point in time, due to the fact that, the fact of the matter is, needless to say.* Delete or rewrite.
4. **Find the voice mismatches.** Three patterns to fix: disowned claims (`it could be argued`, `it may be the case that`) → bare declarative; author intruding on mechanism descriptions (`I have designed X to do Y`) → impersonal; reflexive `I argue` / `I think` on claims that read fine without them → drop the wrapper. Keep first-person only where it actively flags epistemic status.
5. **Find the consultant-speak.** Search for *various, several, different approaches, considerations, factors, aspects, characteristics.* These often mark sentences that describe the existence of a tradeoff without naming it. Name the tradeoff and pick a side.
6. **Find the generic quantifiers.** *Several, various, many, multiple, some, most, often, typically, generally* — for each, ask whether a specific number, name, or condition was available and got dropped. Replace with specifics where they exist; state the gap honestly where they don't.
7. **Find the bare positions.** Look for confident claims that aren't followed by one beat of reasoning. Either back them in a sentence or remove them — an unsupported claim sitting alone reads as opinion-as-fact.
8. **Read aloud.** Sentences that trip in the mouth usually have filler, vague nouns, or buried verbs. Fix the trips.

---

## A final check

Before submitting any technical prose, run through this list:

- [ ] Does each paragraph have a real claim, not just an existence-of-topic statement?
- [ ] Are nouns specific where specifics were available — named systems, real numbers, dated incidents — rather than smoothed into generic quantifiers?
- [ ] Are hedges concentrated in the genuinely uncertain sections rather than scattered uniformly?
- [ ] Do claims default to bare declarative, with first-person used only where epistemic status needs flagging and impersonal used for mechanism descriptions?
- [ ] Are non-obvious claims backed by one beat of reasoning rather than left as bare assertion?
- [ ] Are tradeoffs named with a chosen side, and are bare choices stated as choices rather than dressed up as tradeoffs?
- [ ] Does subject stay close to verb, with long modifiers split into their own sentences?
- [ ] Does the rhythm vary, with short sentences landing the key claims?
- [ ] Is the filler cut?
- [ ] Are limitations stated as facts rather than performed as humility?

If any answer is no, revise before submitting.
