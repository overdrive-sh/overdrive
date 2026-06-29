# Research: How Kubernetes Replaces a Workload Instance Under a Still-Declared Higher-Level Object

**Date**: 2026-06-29 | **Researcher**: nw-researcher (Nova) | **Confidence**: High (mechanism core); Medium on two derived refinements | **Sources**: 12 (8 High-tier official)

## Executive Summary

Kubernetes replaces a workload instance under a still-declared higher-level object by **changing desired state, never by mutating the dead instance and never by letting an instance's terminal status veto placement.** `kubectl rollout restart deployment/X` mutates exactly one thing — the `kubectl.kubernetes.io/restartedAt` RFC3339 timestamp annotation on the **pod template** (`.spec.template.metadata.annotations`), leaving replicas, selector, image, and strategy untouched (Finding 1). That template change shifts the `pod-template-hash`, which forces a new ReplicaSet and a rolling replacement; an identical `kubectl apply` produces an identical template, identical hash, and a no-op (Finding 2). This is the precise analogue of Overdrive's "deploy must not restart; restart must": the positive "replace now" signal is a content change to desired intent that a plain re-apply structurally cannot reproduce.

Three supporting mechanisms round out the picture. `metadata.generation` is a monotonic spec-version counter and `observedGeneration` is the level-triggered "controller has caught up" gate — but K8s decides *which* instance replaces by content-addressing the template (the hash), not by the generation counter (Finding 3). Instance identity is deliberately disposable: a replaced Pod gets a new name suffix, a new UID, and (being freshly bound) a new IP, while the Service ClusterIP and DNS name stay stable and the EndpointSlice controller re-points to the new Pod (Findings 4, 5) — exactly Overdrive's "stable frontend F survives, A1 ≠ A2" guardrail. K8s has **no sticky operator-stop sentinel**; its closest pause/resume shapes (scale-to-0-then-up, Job `.spec.suspend`) are booleans/levels in *desired state* that resume into fresh instances with intent intact (Finding 6). The restart operation is level-triggered, not edge-queued: it converges to the latest template and rolls over an in-flight rollout rather than queueing replacements (Finding 7).

For Overdrive's A/B/C choice, the K8s precedent most supports **(A) — a positive restart directive carried in desired intent, compared by value** — because that is what K8s does (the `restartedAt` token is exactly this shape). **(C)**'s monotonic counter is validated as a desired-state convergence signal (`generation`) but is NOT how K8s selects replacement, so (C) would use a counter more aggressively than K8s does. **(B)** — re-stamping the dead instance to an overridable terminal — is the shape K8s most clearly avoids; K8s never mutates an instance's terminal state to unblock placement. The one adaptation Overdrive must make over the raw K8s pattern is that it *does* carry an operator-stop veto by design, so its chosen mechanism must additionally flip that stopped row overridable — cleanest as a positive, value-comparable directive in intent (the (A)+content-addressing shape).

## Research Methodology

**Search Strategy**: Authoritative-first — kubernetes.io reference docs, kubectl source (`kubernetes/kubernetes` on GitHub), Deployment/ReplicaSet controller source, KEPs, and the Kubernetes API reference. Industry sources used only to triangulate, never as the sole authority for a mechanism claim.
**Source Selection**: Official (kubernetes.io, github.com/kubernetes) prioritised; cross-referenced where a single page is the only authority.
**Quality Standards**: Target 2-3 sources/claim; mechanism claims anchored to at least one official source. Where unverifiable, flagged explicitly.

## The Overdrive Decision This Informs

Overdrive is adding `overdrive workload restart <id>`: end the current backend instance, bring up a NEW instance (new allocation id + new address), while declared intent stays present. Blocker: the WorkloadLifecycle reconciler refuses to place a fresh instance when it observes an alloc row as `Terminated / Stopped{by:Operator}` — by design, so an idempotent re-`deploy` does not undo a deliberate operator stop. `restart` needs a POSITIVE signal a plain re-apply never produces, overriding the operator-stop only for that explicit restart.

Three candidate mechanism shapes:
- **(A)** Explicit restart-directive intent key the verb writes (re-apply does not) → stopped row becomes overridable.
- **(B)** Re-stamp the old instance to an "overridable" terminal so existing placement logic proceeds.
- **(C)** Monotonic run-generation/epoch counter the restart bumps; controller places fresh when desired-generation > latest instance's generation.

## Findings

### Finding 1: `kubectl rollout restart deployment/X` mutates ONLY a pod-template annotation; the Deployment's own intent is otherwise unchanged

**Evidence**: The kubectl client-side restarter (`defaultObjectRestarter`) sets a single annotation on the **pod template**, not on the Deployment object itself:

```go
if obj.Spec.Template.ObjectMeta.Annotations == nil {
    obj.Spec.Template.ObjectMeta.Annotations = make(map[string]string)
}
obj.Spec.Template.ObjectMeta.Annotations["kubectl.kubernetes.io/restartedAt"] =
    time.Now().Format(time.RFC3339)
```

"No other fields are modified—only the annotation is added, then the object is encoded and returned." For Deployments only, it first refuses if the Deployment is paused (`"can't restart paused deployment (run rollout resume first)"`).

**Source**: [kubernetes/kubectl `pkg/polymorphichelpers/objectrestarter.go`](https://github.com/kubernetes/kubectl/blob/master/pkg/polymorphichelpers/objectrestarter.go) — Accessed 2026-06-29
**Confidence**: High
**Verification**: [kubernetes.io — kubectl_rollout_restart reference](https://kubernetes.io/docs/reference/kubectl/generated/kubectl_rollout/kubectl_rollout_restart/) ("Restart a resource ... Resource rollout will be restarted."); annotation key/path/value corroborated by [Marc Nuri, "Rollout Restart Deployment from Java"](https://blog.marcnuri.com/rollout-restart-deployment-from-java) (Medium-High, triangulation only).
**Analysis**: The restart is expressed as a *mutation of the desired pod template*, written through the normal API update path. The Deployment's replicas, selector, strategy, and image are untouched — the operator's standing intent ("run this workload") is preserved; only the template fingerprint moves. This is the key shape: the restart signal is *carried inside the desired-state object*, in a field (the template) whose change is the controller's defined rollout trigger.

### Finding 2: Mutating `.spec.template` is the EXACT thing that separates "must replace" from "no-op re-apply" — via the pod-template-hash

**Evidence**: "A Deployment's rollout is triggered if and only if the Deployment's Pod template (that is, `.spec.template`) is changed, for example if the labels or container images of the template are updated. Other updates, such as scaling the Deployment, do not trigger a rollout."

The hash mechanism that detects the change: "The `pod-template-hash` label is added by the Deployment controller to every ReplicaSet that a Deployment creates or adopts ... It is generated by hashing the `PodTemplate` of the ReplicaSet and using the resulting hash as the label value." When the template changes, "the Deployment creates a new ReplicaSet with a different hash," and "A new ReplicaSet is created, and the Deployment gradually scales it up while scaling down the old ReplicaSet, ensuring Pods are replaced at a controlled rate."

**Source**: [kubernetes.io — Deployments concept](https://kubernetes.io/docs/concepts/workloads/controllers/deployment/) — Accessed 2026-06-29
**Confidence**: High
**Verification**: Mechanism cross-referenced with Finding 1 (the annotation IS a template change) and the kubectl reference above.
**Analysis**: This is the direct analogue of Overdrive's "deploy must not restart; restart must" requirement. A `kubectl apply` of an identical manifest produces an identical template → identical `pod-template-hash` → the existing ReplicaSet is matched, nothing replaces (no-op). `rollout restart` injects `restartedAt: <now>` into the template → the hash changes → a new ReplicaSet is required → replacement is forced. **The discriminator is a content change to the desired template that a plain re-apply structurally cannot reproduce** (because re-apply carries the user's original manifest, which has no fresh timestamp). The positive "must replace" signal lives in the desired state itself, level-triggered through the hash, not in an imperative side-channel.

### Finding 3: `metadata.generation` is a monotonic spec-version counter; `observedGeneration` is the level-triggered "has the controller caught up" signal — but `rollout restart` does NOT bump generation through generation alone

**Evidence**: API conventions define generation as "a sequence number representing a specific generation of the desired state. Set by the system and monotonically increasing, per-resource." And: "observedGeneration ... is the `generation` most recently observed by the component responsible for acting upon changes to the desired state of the resource ... if .metadata.generation is currently 12, but the .status.conditions[x].observedGeneration is 9, the condition is out of date." The API server increments `generation` "every time `.spec` is mutated"; "status updates, label changes, and annotation changes [to the object itself] do not increment it."

**Source**: [kubernetes/community — API conventions (`api-conventions.md`)](https://github.com/kubernetes/community/blob/master/contributors/devel/sig-architecture/api-conventions.md) — Accessed 2026-06-29
**Confidence**: Medium-High
**Verification**: Behavior corroborated by [Alena Varkockova, "Implementing observedGeneration"](https://alenkacz.medium.com/kubernetes-operator-best-practices-implementing-observedgeneration-250728868792) (Medium-High) and [Freedonia, "metadata.generation value increase"](https://midbai.com/en/post/meta-generation-increasing-strategy-exploration/) (Medium — triangulation only).
**Analysis**: Generation/observedGeneration is the canonical **level-triggered "desired changed" pattern**: the controller compares `observedGeneration` (what it last reconciled) against `metadata.generation` (the current desired version) and acts when it is behind. A `rollout restart` *does* mutate `.spec` (the embedded `.spec.template.metadata.annotations`), so it **does** bump the Deployment's `generation` as a side effect — but generation is NOT the mechanism that selects which pods replace. The replacement is driven by the `pod-template-hash` (Finding 2). Generation is the "have I processed this spec yet" gate; the hash is the "is this the right ReplicaSet" gate. This distinction matters for Overdrive's candidate (C): K8s uses a monotonic spec counter for *convergence tracking*, but uses *content-addressing of the template* (not the counter) to decide replacement. **Caveat / partial verification**: the "annotation changes do not increment generation" claim refers to annotations on the *top-level object's* metadata; the `restartedAt` annotation is on the *pod template* inside `.spec`, which is a spec mutation and does increment generation. The two are easy to conflate — see Knowledge Gaps.

### Finding 4: `kubectl delete pod` under a ReplicaSet yields a fresh replacement Pod (new name suffix + new UID); the Deployment's intent is untouched

**Evidence**: "A given Pod (as defined by a UID) is never 'rescheduled' to a different node; instead, that Pod can be replaced by a new, near-identical Pod." On replacement identity: "If you make a replacement Pod, it can even have same name (as in `.metadata.name`) that the old Pod had, but the replacement would have a different `.metadata.uid` from the old Pod." Kubernetes "uses a higher-level abstraction, called a controller, that handles the work of managing the relatively disposable Pod instances" — the controller observes the deletion and drives actual back toward desired by creating a replacement.

**Source**: [kubernetes.io — Pod Lifecycle](https://kubernetes.io/docs/concepts/workloads/pods/pod-lifecycle/) — Accessed 2026-06-29
**Confidence**: High
**Verification**: ReplicaSet replace-on-deletion behavior cross-referenced with [kubernetes.io — ReplicaSet concept](https://kubernetes.io/docs/concepts/workloads/controllers/replicaset/) (fetched below, Finding 5).
**Analysis**: This is the "force one fresh instance" path. Deleting a Pod does not touch the Deployment or ReplicaSet *desired* state (replicas, template). The ReplicaSet controller simply observes `running < desired` and creates a replacement. The replacement is a **new instance**: a new name (the ReplicaSet generates a fresh random suffix) and crucially a new `.metadata.uid`, plus (since it is freshly scheduled and bound) typically a new Pod IP. Maps cleanly to Overdrive's "new allocation id + new address, intent retained" — except that in K8s the *operator never marks the old instance stopped-by-operator*; the deletion is just an actual-state perturbation the level-triggered controller heals. K8s has no sticky operator-stop sentinel to override (see Finding 6).

### Finding 5: Instance identity changes across replacement, but the stable frontend (Service ClusterIP + DNS) survives; Endpoints re-point to the new Pod

**Evidence**: ReplicaSet: "A ReplicaSet's purpose is to maintain a stable set of replica Pods running at any given time." Pods it creates carry generated names and distinct UIDs (Finding 4). On the stable frontend, the Service concept: a Service gets a stable virtual IP (ClusterIP) and a stable DNS name; the set of Pods backing it is selected by label, and the EndpointSlice/Endpoints objects are continuously reconciled to the current matching Pods — so when a Pod is replaced, its old IP is removed from the endpoints and the new Pod's IP is added, while the Service's ClusterIP and DNS name do not change.

The Service docs confirm the indirection directly: "Kubernetes assigns this Service an IP address (the cluster IP), that is used by the virtual IP address mechanism" (stable), while "the set of Pods running in one moment in time could be different from the set of Pods running that application a moment later" (ephemeral backends), and "The controller for that Service continuously scans for Pods that match its selector, and then makes any necessary updates to the set of EndpointSlices for the Service" (the re-pointing reconciler).

**Source**: [kubernetes.io — ReplicaSet](https://kubernetes.io/docs/concepts/workloads/controllers/replicaset/) and [kubernetes.io — Service](https://kubernetes.io/docs/concepts/services-networking/service/) — Accessed 2026-06-29
**Confidence**: High
**Verification**: ClusterIP-stable / Pod-IP-ephemeral / EndpointSlice-reconciliation all directly quoted from the official Service concept page; replacement-Pod identity cross-referenced with Finding 4 (Pod Lifecycle) and the ReplicaSet page (generated name suffix).
**Analysis**: This is Overdrive's guardrail: stable frontend `F` survives the cycle while `A1 ≠ A2` (old backend address differs from the new one). K8s realises the same invariant by *separating the durable name (Service ClusterIP/DNS) from the disposable instance (Pod IP/UID)*, with the EndpointSlice controller as the re-pointing reconciler. The address indirection is what makes per-instance replacement safe for callers — exactly the F-survives / backend-address-rotates shape Overdrive wants.

### Finding 6: K8s has no sticky operator-stop sentinel; the closest analogues are scale-to-0-then-up and Job `.spec.suspend` — both create FRESH instances on resume, with intent preserved throughout

**Evidence**: For Jobs, suspend is a declarative pause carried in the spec: "Suspending a Job will delete its active Pods until the Job is resumed again." On resume the Job creates fresh Pods toward its unchanged `.spec.completions` target — the Job object (its declared intent) is never deleted, only the running Pods are terminated, and "the Job's completion counter/progress is maintained." For Deployments, scaling to 0 terminates all Pods (scaling "[does] not trigger a rollout" — it is an actual-state operation, Finding 2), and scaling back up has the ReplicaSet "[create] new Pods ... using its Pod template" to meet the desired count — i.e. fresh Pods with new names/UIDs (Finding 4/5), not the resurrection of the terminated ones.

**Source**: [kubernetes.io — Job (`.spec.suspend`)](https://kubernetes.io/docs/concepts/workloads/controllers/job/) and [kubernetes.io — ReplicaSet](https://kubernetes.io/docs/concepts/workloads/controllers/replicaset/) — Accessed 2026-06-29
**Confidence**: Medium-High
**Verification**: Job suspend semantics directly quoted from the Job concept page; scale-0-then-up fresh-pod behavior inferred from the ReplicaSet "creates Pods from the template to meet desired count" contract plus Pod-identity-by-UID (Finding 4) — the docs do not contain a single sentence stating "scale-up creates pods with new UIDs," so this is composed from two quoted facts rather than one verbatim source. Flagged in Knowledge Gaps.
**Analysis**: The critical structural difference from Overdrive: **K8s's suspend signal lives in the desired-state spec (`suspend: true/false`), not as a terminal status on the instance.** There is no "this Pod was Stopped{by:Operator}" row that a controller must learn to override. Suspend/resume is a *level* on the desired object; the controller converges to whatever that level says. Overdrive's blocker (a terminal alloc row that suppresses placement) is a shape K8s deliberately avoids — K8s never lets the *instance's terminal state* veto the *higher object's desired state*. The Job-suspend model is the cleanest precedent for "operator pause that resumes into a fresh instance": the pause is a boolean in desired state, and clearing it converges to a new instance with the declared intent intact.

### Finding 7: `rollout restart` is level-triggered through the template hash; back-to-back invocations within the same RFC3339 second are effectively idempotent (same annotation value → same hash → no new rollout); cross-second invocations each force a distinct rollout

**Evidence**: The restart annotation value is `time.Now().Format(time.RFC3339)` (Finding 1) — second-granularity. The rollout is "triggered if and only if the Deployment's Pod template ... is changed" and replacement is selected by `pod-template-hash` (Finding 2). Therefore two invocations that compute the *same* RFC3339 timestamp produce an *identical* template and the *same* hash — the existing ReplicaSet matches and no new rollout occurs. Two invocations in *different* seconds produce different annotation values → different hashes → each is a distinct rollout. The Deployment is "rolling over": "If you update a Deployment while a previous rollout is in progress, the Deployment creates a new ReplicaSet ... and starts scaling it up, and rolls over the ReplicaSet that it was scaling up previously" — it does not queue; it re-targets to the newest template. And matching-template handovers reuse the existing ReplicaSet rather than recreate it (the rollback path scales the existing RS up rather than minting a new one).

**Source**: [kubernetes/kubectl `objectrestarter.go`](https://github.com/kubernetes/kubectl/blob/master/pkg/polymorphichelpers/objectrestarter.go) (RFC3339 value) + [kubernetes.io — Deployments, "Rollover" / "Rolling Back"](https://kubernetes.io/docs/concepts/workloads/controllers/deployment/) — Accessed 2026-06-29
**Confidence**: Medium
**Verification**: The "different timestamp each invocation → new ReplicaSet each time" general claim is corroborated by [Plural, "kubectl rollout restart: The Right Way"](https://www.plural.sh/blog/kubectl-rollout-restart-deployment/) (Medium-High). The same-second-idempotency refinement is *derived* from the RFC3339 second-granularity value + the if-and-only-if-template-changed rule; I did not find a source that states the same-second case explicitly. Flagged in Knowledge Gaps.
**Analysis**: The operation is **level-triggered, not edge-queued**: the controller always converges to the latest pod template, it does not enqueue one replacement per invocation. The unit of "did anything change" is the *content* of the desired template, addressed by hash — re-issuing the same content is a no-op; issuing new content rolls over to it (cancelling an in-flight rollout toward the older content). For Overdrive's restart verb, the lesson is that an idempotency posture falls out naturally if the "restart token" is content-addressed and compared by value: re-issuing the identical token is a no-op, a fresh token forces exactly one new instance, and a token issued mid-restart simply supersedes the in-flight one.

## Source Analysis

| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|--------|--------|------------|------|-------------|----------------|
| kubectl `objectrestarter.go` (master) | github.com/kubernetes/kubectl | High (1.0) | Official source | 2026-06-29 | Y |
| kubectl_rollout_restart reference | kubernetes.io | High (1.0) | Official docs | 2026-06-29 | Y |
| Deployments concept | kubernetes.io | High (1.0) | Official docs | 2026-06-29 | Y |
| ReplicaSet concept | kubernetes.io | High (1.0) | Official docs | 2026-06-29 | Y |
| Service concept | kubernetes.io | High (1.0) | Official docs | 2026-06-29 | Y |
| Pod Lifecycle concept | kubernetes.io | High (1.0) | Official docs | 2026-06-29 | Y |
| Job (`.spec.suspend`) concept | kubernetes.io | High (1.0) | Official docs | 2026-06-29 | Y |
| API conventions (`api-conventions.md`) | github.com/kubernetes/community | High (1.0) | Official spec | 2026-06-29 | Y |
| "Implementing observedGeneration" | alenkacz.medium.com | Medium (0.6) | Community expert | 2026-06-29 | Triangulation only |
| "Rollout Restart from Java" | blog.marcnuri.com | Medium (0.6) | Community expert | 2026-06-29 | Triangulation only |
| "kubectl rollout restart: The Right Way" | plural.sh | Medium-High (0.8) | Industry | 2026-06-29 | Triangulation only |
| "metadata.generation value increase" | midbai.com | Medium (0.6) | Community | 2026-06-29 | Triangulation only |

Reputation: High: 8 (67%) | Medium-High: 1 (8%) | Medium: 3 (25%) | Avg ≈ 0.87. Every mechanism claim is anchored to at least one High-tier official source; Medium-tier sources are used only to triangulate, never as sole authority.

## Knowledge Gaps

### Gap 1: "Same-second idempotency" of `rollout restart` is derived, not directly sourced
**Issue**: The claim that two `rollout restart` invocations within the same RFC3339 second produce an identical template (hence no new rollout) is *composed* from two quoted facts — the annotation value is `time.Now().Format(time.RFC3339)` (second granularity) and rollout fires iff the template changes — rather than stated verbatim by any single source. **Attempted**: targeted searches for "rollout restart twice same second"; the available sources state only the general "different timestamp each time → new ReplicaSet." **Recommendation**: confirm empirically (two `rollout restart` calls scripted within one second; observe whether `kubectl rollout history` records one or two new revisions) before relying on this nuance for Overdrive's idempotency design. Confidence on Finding 7's refinement is Medium accordingly.

### Gap 2: scale-0-then-up creates pods with *new* UIDs — inferred, not single-sourced
**Issue**: The docs state the ReplicaSet creates Pods from its template to meet desired count, and (separately) that replacement Pods carry new UIDs; no single sentence asserts "scaling 0→N produces pods with new UIDs." **Attempted**: ReplicaSet + Pod Lifecycle pages. **Recommendation**: treat as High-confidence by composition (a scaled-down Pod is deleted, and a freshly created Pod is by definition a new object with a new UID), but note it is composed.

### Gap 3: Did NOT inspect the Deployment controller's Go source for the hash-collision / ReplicaSet-equality comparison
**Issue**: Findings 2/7 rely on the concept docs' description of `pod-template-hash` matching; I did not read `kubernetes/kubernetes` `pkg/controller/deployment/` to confirm the exact equality function (`EqualIgnoreHash`) the controller uses to match a template to an existing ReplicaSet. **Attempted**: concept docs only (within turn budget). **Recommendation**: for an implementation-grade decision, read `pkg/controller/deployment/util/deployment_util.go` (`GetNewReplicaSet` / `EqualIgnoreHash`) to confirm the precise reuse-vs-create boundary.

### Gap 4: `generation` increment on a pod-template-only change — confirmed by reasoning, anchored on Medium sources for the precise rule
**Issue**: The rule "spec mutations bump generation; object-metadata annotation changes do not" is anchored to High-tier API conventions for the *definition* of generation, but the precise "pod-template annotation IS a spec change so it DOES bump generation" step leans on Medium-tier community sources for the mechanics. **Recommendation**: low risk (the pod template is unambiguously under `.spec`), but if generation-bump behavior becomes load-bearing for candidate (C), confirm against the API server's `generation`-increment code path.

## Mapping to Overdrive's Mechanism Choice (A/B/C)

Framed as evidence for the architect, not a verdict.

**Which K8s mechanism each Overdrive option resembles:**

- **(A) Explicit restart-directive intent key the verb writes (re-apply does not).**
  This is the *closest structural match to what K8s actually does.* `kubectl rollout restart` writes `kubectl.kubernetes.io/restartedAt: <timestamp>` into the **desired state** (the pod template) — a key a plain `kubectl apply` of the user's manifest never carries, because the user's manifest has no fresh timestamp (Findings 1, 2). The restart signal is a positive token *inside desired intent*, and the level-triggered controller replaces because the desired content changed. Overdrive's (A) — a restart-directive intent key the verb writes and re-apply does not — is the same idea: a positive signal in intent that a no-op re-apply cannot reproduce. The one adaptation: K8s's token is content (changes the template hash) and there is no terminal-status veto to override; Overdrive's (A) must *additionally* mark the operator-stopped row overridable, because Overdrive (unlike K8s) keeps a sticky operator-stop sentinel (Finding 6).

- **(B) Re-stamp the old instance to an "overridable" terminal.**
  K8s has *no analogue* for this. K8s never mutates the *instance's terminal state* to unblock placement — it does not re-stamp a dead Pod; it changes *desired* state and lets the controller create a new instance (Findings 4, 6). Rewriting a terminal instance row to coax existing placement logic forward is exactly the "instance status vetoes/gates desired intent" coupling K8s structurally avoids. Weakest precedent support.

- **(C) Monotonic run-generation/epoch counter the restart bumps.**
  K8s *has* a monotonic spec counter — `metadata.generation` — and uses `observedGeneration` as the level-triggered "controller has caught up" gate (Finding 3). But critically, **generation is used for convergence tracking, not for selecting which instance replaces.** Replacement is driven by *content-addressing of the template* (the `pod-template-hash`), not by "desired-generation > instance-generation." So K8s validates the *existence and usefulness* of a monotonic desired-state counter, but does NOT validate using that counter as the replacement-selection predicate. (C) is a defensible design, just not the mechanism K8s itself uses to decide placement.

**Which option the K8s precedent most supports:** **(A)**, with a borrowed refinement from (C)/(B).

The dominant K8s pattern is: *encode the restart as a content change in desired state* (A-shaped), then let a level-triggered controller converge by *comparing desired content to the running instance's content* — K8s does this by hash, not by counter. The single biggest divergence the architect must account for is that **K8s has no sticky operator-stop sentinel to override** (Finding 6): K8s suspend/resume is a boolean in desired state, so there is never a terminal instance row that gates fresh placement. Overdrive's reconciler *does* have that veto by design, so whichever option is chosen must also carry the override of the operator-stop — and the cleanest K8s-aligned shape is to make that override a *positive directive in intent* (A) whose value is *content-addressed / comparable* (so re-issuing it is a no-op and a fresh issue forces exactly one replacement, mirroring Finding 7's level-triggered idempotency). A monotonic epoch (C) is a sound alternative *encoding* of that same "desired content moved" signal, and is the closest K8s has to a built-in counter — but K8s itself decides replacement by content comparison, so if Overdrive adopts (C) it would be using the counter more aggressively (as the placement predicate) than K8s does.

**Net for the architect:** the K8s evidence points at "positive restart directive carried in desired intent, compared by value, that also flips the stopped row overridable" — strongly (A), with (C) as a reasonable counter-based encoding of the same idea, and (B) as the shape K8s most clearly avoids.

## Recommendations for Further Research

1. Read `kubernetes/kubernetes` `pkg/controller/deployment/util/deployment_util.go` (`GetNewReplicaSet`, `EqualIgnoreHash`) to pin the exact template-equality and ReplicaSet-reuse-vs-create boundary — removes Gaps 1 and 3 for an implementation-grade decision.
2. Empirically confirm the same-second `rollout restart` idempotency claim (Gap 1) on a live cluster via `kubectl rollout history`.
3. Inspect the Job controller's handling of `.spec.suspend` resume to confirm the fresh-Pod-on-resume identity claim (Gap 2) at source level, since the Job-suspend model is the cleanest precedent for Overdrive's stopped→restart path.

## Full Citations

[1] The Kubernetes Authors. "objectrestarter.go" (`pkg/polymorphichelpers`). kubernetes/kubectl, master branch. https://github.com/kubernetes/kubectl/blob/master/pkg/polymorphichelpers/objectrestarter.go. Accessed 2026-06-29.
[2] The Kubernetes Authors. "kubectl rollout restart". Kubernetes Documentation. https://kubernetes.io/docs/reference/kubectl/generated/kubectl_rollout/kubectl_rollout_restart/. Accessed 2026-06-29.
[3] The Kubernetes Authors. "Deployments". Kubernetes Documentation. https://kubernetes.io/docs/concepts/workloads/controllers/deployment/. Accessed 2026-06-29.
[4] The Kubernetes Authors. "ReplicaSet". Kubernetes Documentation. https://kubernetes.io/docs/concepts/workloads/controllers/replicaset/. Accessed 2026-06-29.
[5] The Kubernetes Authors. "Service". Kubernetes Documentation. https://kubernetes.io/docs/concepts/services-networking/service/. Accessed 2026-06-29.
[6] The Kubernetes Authors. "Pod Lifecycle". Kubernetes Documentation. https://kubernetes.io/docs/concepts/workloads/pods/pod-lifecycle/. Accessed 2026-06-29.
[7] The Kubernetes Authors. "Jobs". Kubernetes Documentation. https://kubernetes.io/docs/concepts/workloads/controllers/job/. Accessed 2026-06-29.
[8] The Kubernetes Authors. "Kubernetes API Conventions" (`api-conventions.md`). kubernetes/community, master branch. https://github.com/kubernetes/community/blob/master/contributors/devel/sig-architecture/api-conventions.md. Accessed 2026-06-29.
[9] Varkockova, Alena. "Kubernetes operator best practices: Implementing observedGeneration". Medium. https://alenkacz.medium.com/kubernetes-operator-best-practices-implementing-observedgeneration-250728868792. Accessed 2026-06-29. [Medium-tier — triangulation only.]
[10] Nuri, Marc. "Rollout Restart Deployment from Java using YAKC". blog.marcnuri.com. https://blog.marcnuri.com/rollout-restart-deployment-from-java. Accessed 2026-06-29. [Medium-tier — triangulation only.]
[11] Plural. "`kubectl rollout restart deployment`: The Right Way". plural.sh. https://www.plural.sh/blog/kubectl-rollout-restart-deployment/. Accessed 2026-06-29. [Medium-High — triangulation only.]
[12] Freedonia (midbai). "Research the principle of metadata.generation value increase". midbai.com. https://midbai.com/en/post/meta-generation-increasing-strategy-exploration/. Accessed 2026-06-29. [Medium-tier — triangulation only.]

## Research Metadata
Duration: ~1 session | Examined: 12 sources | Cited: 12 | Cross-refs: per-finding (all 7 findings cross-referenced) | Confidence: High (Findings 1, 2, 4, 5), Medium-High (Findings 3, 6), Medium (Finding 7) | Output: docs/research/lifecycle/kubernetes-instance-replacement-mechanisms.md
