# Competitive Research — workload-identity-manager (GH #35)

**Wave**: DIVERGE (Phase 2 of 4) · **Agent**: Flux (nw-diverger) · **Date**: 2026-06-08

> **Methodology note.** The `nw-researcher` sub-agent could not be invoked (the Task
> tool is unavailable inside a subagent context; this DIVERGE run is itself a dispatched
> subagent). Research was therefore conducted directly via WebSearch/WebFetch against
> **primary sources** (project source repos, official architecture docs, SPIFFE/SPIRE/
> Istio/Linkerd/Nomad/cert-manager docs), applying the same evidence discipline: every
> claim names a real component/type/RPC and cites a source; inference is flagged.
> Versions noted where behavior is version-specific (June 2026).

We are designing the **in-memory SVID store + per-workload identity lifecycle + read
surface for in-process dataplane consumers** of a sidecarless orchestrator. Rotation is
out of scope for #35 (→ #40) but the **rotation seam shape** of each system is captured
because our design must not foreclose it.

The four load-bearing dimensions per system:
**(a)** storage model · **(b)** read surface · **(c)** rotation seam · **(d)** drop/cleanup.

---

## System 1 — SPIRE agent (the canonical precedent)

**Category:** SPIFFE reference implementation; node-local agent + Unix-socket Workload API.

- **(a) Storage model.** In-memory `cache.Cache` keyed by **registration entry ID**, with
  a secondary **selector index** for workload-selector lookups. Each `cacheRecord` holds
  `{entry, svid *X509SVID, subs map[*subscriber]struct{}}` — i.e. **the SVID and its
  subscriber set live together in the record.** All in agent process memory; ~1 KB per
  X.509-SVID. An LRU bound (`x509_svid_cache_max_size`) was added and made
  unconditionally-enabled to cap memory (RFC #2940 / #2593). Sources:
  [spire#2940 cache redesign](https://github.com/spiffe/spire/issues/2940),
  [spire#2593 LRU caching](https://github.com/spiffe/spire/issues/2593),
  [spire#2591 single-copy chains](https://github.com/spiffe/spire/issues/2591).
- **(b) Read surface — PUSH (streaming subscription).** The Workload API
  `FetchX509SVID` is a **server-streaming gRPC RPC over a Unix domain socket**: the
  handler *subscribes* to cache changes for the caller's selector set and **streams
  updates until the stream closes** — the workload does not poll. Sources:
  [SPIFFE Workload API / agent docs](https://spiffe.io/docs/latest/deploying/spire_agent/),
  [spire#2940](https://github.com/spiffe/spire/issues/2940),
  [spire#401 FetchX509SVID](https://github.com/spiffe/spire/issues/401).
- **(c) Rotation seam — sync recomputes, cache notifies subscribers.** A **periodic sync**
  (~5 s) fetches authorized entries+bundles from the SPIRE *server*, **computes
  missing/expiring SVIDs, updates the cache, and notifies the affected subscribers**. The
  rotation *decision* (server-side signing + agent-side expiry calc) is **decoupled from
  the read surface**: subscribers learn of a new SVID through the *same* notification
  channel they'd learn of any change — the stream just emits a fresh SVID. Source:
  [spire#2940](https://github.com/spiffe/spire/issues/2940).
- **(d) Drop/cleanup.** Entry-driven: when a registration entry is removed during sync,
  its `cacheRecord` is deleted and subscribers notified. LRU eviction caps unused entries.
  **Lifecycle is driven by the authoritative entry set, not by TTL alone** (source: #2940).

**Lesson for us:** the SVID + its subscriber set co-located in one record, with a
**push/subscribe** read surface, is the battle-tested shape. The rotation seam is *"the
sync updates the cache → the cache notifies"* — rotation never reaches into the consumer
directly. This is the single strongest precedent for our fork-B (where material lives)
and fork-E (read surface).

---

## System 2 — istio-agent + Envoy SDS (the push-on-rotate precedent)

**Category:** Service-mesh node agent translating an internal `SecretManager` to Envoy via
the Envoy **SDS (Secret Discovery Service)** xDS API.

- **(a) Storage model.** `security/pkg/nodeagent/cache` is the **in-memory secret store**;
  the `SecretManager` owns it. Keyed by Envoy **resource name** (e.g. `default` for the
  workload cert, `ROOTCA` for the trust bundle). `GenerateSecret(resourceName)` returns a
  cached secret when still valid, else mints a fresh one. Sources:
  [secretcache.go](https://github.com/istio/istio/blob/master/security/pkg/nodeagent/cache/secretcache.go),
  [cache pkg](https://pkg.go.dev/istio.io/istio/security/pkg/nodeagent/cache),
  [istio-agent architecture](https://github.com/istio/istio/blob/master/architecture/security/istio-agent.md).
- **(b) Read surface — PULL request, PUSH update (SDS bidi stream).** Envoy *initiates*
  by sending **SDS requests** over a gRPC stream (`StreamSecrets`); the SDS server (a thin
  translation layer in the agent) calls `SecretManager.GenerateSecret(resourceName)` per
  requested resource and returns it. So **first fetch is pull**, but the *same long-lived
  stream* is the channel for subsequent **server-pushed** updates. Sources:
  [sdsservice.go](https://github.com/istio/istio/blob/master/security/pkg/nodeagent/sds/sdsservice.go),
  [istio-agent architecture](https://github.com/istio/istio/blob/master/architecture/security/istio-agent.md).
- **(c) Rotation seam — callback from SecretManager → SDS → push to Envoy, no restart.**
  When a cert nears expiry (`SECRET_GRACE_PERIOD_RATIO`, default ~½ TTL ± jitter to
  stagger renewals), the `SecretManager` **fires a callback to the SDS server**; *if Envoy
  is still subscribed* the SDS server re-generates the secret and **pushes the new cert
  down the existing stream** — Envoy hot-swaps it with **no restart and no dropped
  connections**. Source:
  [istio-agent architecture](https://github.com/istio/istio/blob/master/architecture/security/istio-agent.md).
- **(d) Drop/cleanup — lazy, subscription-driven.** "We do not permanently watch
  certificates even after Envoy has stopped requesting them; if there are no subscriptions
  the update will be ignored." Cleanup is tied to **the consumer's subscription**, not a
  manual eviction. Source: same architecture doc.

**Lesson for us:** istio is the textbook **"rotation is a callback that pushes down the
existing read channel"** model — the rotation seam and the read surface are *the same
stream*, which is exactly why a rotated cert reaches Envoy without tearing connections.
Critically: cleanup is **subscription-lifetime-driven**, not workload-lifetime-driven —
a contrast to SPIRE's entry-driven model and to what #35 wants (drop on *workload* stop).

---

## System 3 — Linkerd2 identity (the per-pod in-memory leaf precedent)

**Category:** Service mesh; per-proxy identity via CSR to a control-plane identity issuer.

- **(a) Storage model.** **Per-pod, in-proxy memory.** At startup the linkerd2-proxy
  **generates its private key into a `tmpfs` emptyDir** (in-memory, never leaves the pod);
  after receiving the signed leaf it **loads it into an in-proxy in-memory store**. There
  is **no shared cross-pod store** — each proxy holds exactly its own identity. Sources:
  [Linkerd automatic mTLS](https://linkerd.io/2-edge/features/automatic-mtls/),
  [Linkerd identity pipeline (Porta, blog — flagged secondary)](https://gtrekter.medium.com/from-trust-anchors-to-spiffe-ids-understanding-linkerds-automated-identity-pipeline-e57a90ce1414).
- **(b) Read surface.** Trivial — the proxy that *holds* the cert *is* the consumer. The
  proxy uses the in-memory leaf directly as both client and server cert; no fetch API,
  because there's no separation between holder and consumer. Source: Linkerd auto-mTLS docs.
- **(c) Rotation seam — proxy self-renews via fresh CSR.** Leaves are 24 h; the proxy
  **auto-renews at ~70 % of TTL by generating a new CSR** to the identity controller and
  swapping the in-memory leaf. Rotation is a **background loop inside the holder**, driven
  by the holder's own clock against its own leaf. Source: Linkerd auto-mTLS docs.
- **(d) Drop/cleanup.** Pod (and thus proxy process + tmpfs) is destroyed on stop — the
  identity dies **with the process**. No explicit eviction because there's no shared store
  to evict from. Source: Linkerd auto-mTLS docs.

**Lesson for us:** Linkerd shows the **holder==consumer, key-in-tmpfs, self-renew**
shape — which is the model #35 explicitly *cannot* use (Overdrive is **sidecarless**:
there is no in-pod proxy to hold the key or run the renew loop; the *node agent* holds
the leaf on the workload's behalf — ADR-0063 D9). It's the instructive negative: the
reason we need a *separate, shared* `IdentityMgr` at all is precisely that we have no
in-pod holder. (It also confirms "key lives in volatile memory, dies with the workload"
as the leak-resistant default — relevant to O2.)

---

## System 4 — Nomad Workload Identity (the JWT contrast)

**Category:** Orchestrator-native workload identity — **JWT**, not X.509.

- **(a) Storage model.** When an alloc is accepted, the **leader signs a JWT per task**
  with the server keyring. The token is **delivered into the task** (not held in a shared
  read surface for *other* consumers): via an **env var** (`NOMAD_TOKEN_*`) and/or a
  **file in the task filesystem** (`secrets/…jwt`), per the `identity` block. Sources:
  [Nomad workload identity](https://developer.hashicorp.com/nomad/docs/concepts/workload-identity),
  [identity block](https://developer.hashicorp.com/nomad/docs/job-specification/identity).
- **(b) Read surface — file/env, validated out-of-band via JWKS.** Consumers (Vault,
  Consul) don't read a Nomad store; they **validate the JWT against Nomad's JWKS URL**
  (Nomad publishes the public keys; the verifier pulls them). The "read surface" is the
  **delivered token + a public JWKS endpoint**, not a held credential store. Sources:
  [WI concepts](https://developer.hashicorp.com/nomad/docs/concepts/workload-identity),
  [WI with Vault](https://developer.hashicorp.com/nomad/docs/secure/workload-identity/vault).
- **(c) Rotation seam.** Tokens are short-TTL; Nomad re-issues and **re-delivers** to the
  task (re-writes the file / refreshes env on renewal). Rotation = re-delivery into the
  task, not a push to a separate consumer. Source: WI concepts.
- **(d) Drop/cleanup.** Token lives in the **task's own filesystem/env**; it dies when the
  alloc is GC'd. No shared store to evict. Source: WI concepts.

**Lesson for us:** the JWT model **inverts the read surface** — instead of a shared store
consumers fetch from, the credential is *delivered into the workload* and *verified via a
public key endpoint*. This is a real alternative worth a brainstorming lens (SCAMPER-R /
Reverse: "what if the workload holds it and consumers verify, rather than a shared store
holding it?") — but it's a poor fit for our **kernel-side** sockops/kTLS consumer that
needs the *actual X.509 leaf + key in-process* to run the TLS 1.3 handshake on the
workload's behalf. Captured as a structural fork, not a likely winner.

---

## System 5 (NON-OBVIOUS) — cert-manager + csi-driver-spiffe (the tmpfs-mount alternative)

**Category:** Kubernetes CSI plugin projecting SVIDs into pods via ephemeral volumes —
**a fundamentally different distribution path** from a shared in-process store. *This is
the flagged non-obvious alternative.*

- **(a) Storage model.** **No shared store at all.** Per pod, the driver **generates a
  private key on the node, creates a cert-manager `CertificateRequest`**, and on signing
  **mounts the key+cert into the pod's volume backed by a per-pod `tmpfs` directory** the
  driver manages. The private key **never leaves the node**; each pod gets a unique
  key/cert pair on a unique tmpfs mount. Sources:
  [csi-driver-spiffe docs](https://cert-manager.io/docs/usage/csi-driver-spiffe/),
  [csi-driver-spiffe README](https://github.com/cert-manager/csi-driver-spiffe).
- **(b) Read surface — the filesystem.** The consumer (the app in the pod) reads the
  cert+key **as files** off the mounted volume. The "read surface" is a **tmpfs path**,
  not an API or a shared handle. Source: csi-driver-spiffe docs.
- **(c) Rotation seam — driver watches expiry, re-mints, re-writes the mount.** The driver
  **watches each cert and renews based on its expiry**, re-issuing via cert-manager and
  **updating the file in the mount**. Rotation = re-write the volume; the app re-reads (or
  uses a file-watch). Note: re-writing a bind-mounted file has known consistency caveats
  ([csi-lib#74](https://github.com/cert-manager/csi-lib/issues/74)). Sources:
  [csi-driver-spiffe docs](https://cert-manager.io/docs/usage/csi-driver-spiffe/).
- **(d) Drop/cleanup — volume lifecycle.** The ephemeral volume is **created and destroyed
  with the pod**; the tmpfs (and the key) are reclaimed automatically when the pod
  terminates. Cleanup is **bound to the volume/pod lifecycle**. Source: csi-driver-spiffe
  README.

**Lesson for us:** the CSI model proves the **"deliver into the workload via a
volatile-memory mount, lifecycle-bound to the workload"** pattern — closest in *intent*
to #35's drop-on-stop, but via the *filesystem* rather than a shared in-process handle.
Its rotation-via-file-rewrite has documented consistency hazards (csi-lib#74) that an
in-process atomic swap avoids. For a sidecarless, in-process-consumer platform, a tmpfs
mount is a heavier, less-coherent path than a shared `Arc` — but it's the sharpest
contrast and a legitimate brainstorming fork (SCAMPER-S / Substitute the read surface
with a vsock/fs delivery).

---

## Cross-system synthesis — the load-bearing lessons

| # | Lesson | Evidenced by | Bears on |
|---|---|---|---|
| **L1** | **The live SVID and its consumer-notification channel belong in one record/store, keyed by the identity's stable id.** SPIRE co-locates `{svid, subs}` per registration entry; istio keys secrets by resource name. A shared, keyed in-memory store is the dominant shape. | SPIRE `cacheRecord`, istio `SecretManager` cache | Fork B (where material lives) — favors a shared keyed map over per-consumer copies. |
| **L2** | **PUSH/subscribe beats poll for the read surface — and the rotation seam reuses the SAME channel.** SPIRE streams `FetchX509SVID`; istio pushes rotated certs down the existing SDS stream. In *both*, a rotated credential reaches the consumer through the *same* notify path as any update — which is *why* connections don't drop on rotation. | SPIRE subscriber-notify; istio SDS callback-push | Fork E (read surface) + Fork D (rotation seam). Strong signal: a **watch/subscribe** read surface makes the future #40 rotation a *push down the same channel*, not a new mechanism. A pure-pull getter forecloses that elegance. |
| **L3** | **Decouple the rotation *trigger* from the store and the read surface.** SPIRE's trigger is the periodic server sync; istio's is a `SecretManager` expiry callback; Linkerd's is the proxy's 70 %-TTL timer; CSI's is the driver's per-cert expiry watch. In every case the trigger is a *separate clock/loop* that updates the store, which then notifies. **No system inlines rotation into the read path.** | All five | Fork D — validates #35's decision to defer rotation to a separate workflow (#40) and leave the store/read surface as the stable seam it pushes into. The seam must be *"something updates the held material; consumers observe the update"* — exactly what #40 needs. |
| **L4** | **Lifecycle-bind the credential, but choose the binding axis deliberately.** SPIRE binds to the **registration-entry set** (entry removed → SVID dropped). istio/CSI bind to the **consumer/volume subscription**. Linkerd binds to the **process**. #35 wants binding to the **workload-allocation Running↔Stopped lifecycle** — which the platform *already observes* via the reconciler. This is closest to SPIRE's "authoritative set drives eviction," with the alloc-status row as our authoritative set. | SPIRE (entry-driven), istio/CSI (subscription/volume), Linkerd (process) | Fork A (lifecycle wiring) + O1/O2. The drop-on-stop driver should be the **same convergence loop** that observes Running↔Stopped — not a TTL, not a consumer ref-count. |
| **L5** | **Keep the leaf private key in volatile memory, lifecycle-scoped, never persisted.** Linkerd's tmpfs emptyDir, CSI's tmpfs mount, SPIRE's in-process cache — all keep the key in RAM and reclaim it with the holder. None persist the leaf key. | Linkerd, CSI, SPIRE | O2 (drop-on-stop / leak-resistance) + the project's own ADR-0063 D6 (leaf key never an audit input, never persisted). Confirms the held store is **in-memory only**; the *audit fact* (not the key) is what gets persisted/observed. |
| **L6** | **The trust bundle is a first-class, separately-keyed entry in the same store as the leaves.** istio keys the root bundle (`ROOTCA`) right next to the workload cert (`default`) in the same `SecretManager`. The bundle and the leaves are read through the same surface. | istio SDS (`ROOTCA` resource) | Fork C (trust-bundle currency) + the read surface — favors holding the bundle *in the same `IdentityMgr`* the leaves live in, exposed via the same handle, rather than a separate component. |
| **L7 (negative / non-obvious)** | **A "deliver into the workload" distribution path (JWT-into-env, SVID-into-tmpfs-mount) is a real alternative — but mismatched for an in-process kernel-side consumer.** Nomad and csi-driver-spiffe both push the credential *to* the workload; our sockops/kTLS/gateway consumers live *in the node agent process* and need the material *in-process*, not on a mount the workload reads. | Nomad WI, csi-driver-spiffe | Forecloses the "fs/vsock delivery" forks as likely winners, but they remain valid brainstorming diversity (SCAMPER-S/R). The contrast sharpens *why* a shared in-process `Arc` fits this platform. |

**Bottom line for brainstorming:** the real-world convergent design is **a shared,
identity-keyed, in-memory store holding both leaves and the trust bundle, with a
push/subscribe read surface, a rotation trigger decoupled as a separate loop that updates
the store, and lifecycle-binding driven by the authoritative workload set.** Overdrive's
twist is that this store is an **in-process `Arc` shared with kernel-side consumers** (no
gRPC socket, no tmpfs mount — whitepaper §7 "direct in-process access, no IPC"), and the
binding axis is the **reconciler-observed allocation lifecycle**. The genuine open
architectural questions (the forks) are: *push vs pull on the read surface* (L2 strongly
favors push for the #40 seam, but in-process getters are simpler — a real tension); *View
vs RwLock vs observation-row as store-of-record* (L1/L4/L5 inform but don't settle);
*where the trust bundle's currency comes from* (L6); and *how the drop-on-stop is wired*
(L4 favors the existing convergence loop).

---

## Gate G2 evaluation

- [x] **≥3 real competitors named** — five systems + a non-obvious alternative
      (SPIRE, istio/Envoy SDS, Linkerd2, Nomad WI, cert-manager+csi-driver-spiffe). **PASS.**
- [x] **≥1 non-obvious alternative (different category, same job)** — csi-driver-spiffe
      (a CSI/tmpfs-mount distribution path, explicitly flagged) + Nomad WI (JWT, not X.509).
      **PASS.**
- [x] **No generic market claims** — every dimension names a concrete type/RPC/component
      (SPIRE `cache.Cache`/`FetchX509SVID`/`cacheRecord`; istio `SecretManager`/SDS
      `StreamSecrets`/`GenerateSecret`/`SECRET_GRACE_PERIOD_RATIO`; Linkerd tmpfs emptyDir
      / 70%-TTL CSR; Nomad `identity` block/`NOMAD_TOKEN_*`/JWKS; CSI `CertificateRequest`
      /tmpfs mount) with cited sources; inference flagged. **PASS.**

**Phase 2 gate: PASS.** Ready for Phase 3 (brainstorming).
