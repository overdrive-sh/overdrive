# Upstream changes — transparent-mtls-host-socket DESIGN (GH #26 folds #222)

Back-propagation flagged for the **product-owner** by Morgan (DESIGN wave,
2026-06-12). The architect does **NOT** edit `jobs.yaml` or the DISCUSS slice files
— those are the product-owner's artifacts. This file records what must change
upstream and why, for the product-owner to action.

## Context

The DESIGN wave formalized the user's LOCKED decision (ADR-0069): ONE universal
**transparent mTLS via an agent-light L4 proxy** for ALL workload kinds, folding
#222 into #26. The previously-primary **in-band kTLS-on-the-workload's-own-socket**
model is SUPERSEDED as v1 (retained as a tracked future optimization). The DISCUSS
job J-SEC-003 and slices 00–05 were authored on the in-band model's properties —
several of which **no longer hold in v1**.

## What changed (the properties that no longer hold)

| DISCUSS premise (in-band model) | v1 reality (proxy model) |
|---|---|
| "the agent EXITS the data path" / "agent fully out" | **Agent is LIGHT, not OUT.** Forward steady state is agent-idle (kernel splice); **return** steady state is agent-LIGHT (`splice` pump, ~1/record) — the agent stays scheduled per-record on the return path for the connection's life. |
| "kTLS on the workload's OWN socket" / "workload-owns-fd" | **kTLS lives on the agent's leg B**, not the workload's socket. The workload holds a plaintext socket to the agent (leg F). |
| "restart-survivable" (kTLS state socket-owned + workload owns fd) | **No restart-survival in v1.** The agent owns both legs + the kTLS state; an agent restart drops in-flight sessions (re-handshake on reconnect). Restart-survival is the in-band model's unique win → tracked future optimization (DEFER-1). |
| "1 socket per connection" | **2 sockets per connection** (leg F + leg B). |
| "host-socket ONLY; guest-stack is #222, a SEPARATE feature" | **#222 folds into #26.** The proxy is universal — guest-stack (microVM/unikernel) routes through the SAME mechanism. |
| race-window "lossy DROP-then-RESET under a named server-speaks-first assumption" | **Lossless for all kinds.** The handshake-window capture is a userspace buffer; no dropped pre-arm bytes, no RESET, NO server-speaks-first assumption. |

## Action items for the product-owner

1. **Re-ground J-SEC-003** (`docs/product/jobs.yaml` § J-SEC-003) on the proxy
   mechanism. The `functional`/`emotional`/`social` dimensions and the `pull`/
   `anxiety` forces reference "agent exits the data path", "kTLS on the workload's
   socket", and "restart-survivable" — re-word to the proxy reality (agent-light
   return; kTLS on the agent's peer-facing leg; no v1 restart-survival; lossless;
   universal across kinds). The CORE job (transparent in-kernel mTLS with the
   workload's own SVID, auth-session == data-session, workload holds nothing,
   provable on the wire) is UNCHANGED and still holds. **Product-owner edits
   `jobs.yaml`.**

2. **Re-ground slices 00–05** (`docs/feature/transparent-mtls-host-socket/slices/`)
   on the proxy mechanism. Specifically:
   - **Slice 00 (spike)** — already DONE; the spikes ARE the evidence base. Mark it
     superseded-by-evidence (the verdict is "proxy", not "in-band").
   - **Slice 01 (sockops detect + fd acquire)** — re-scope to "intercept
     (`cgroup_connect4` rewrite) + sockops detect + agent accepts leg F".
   - **Slice 02 (handshake present held SVID)** — re-scope the handshake to **leg B**
     (the agent's peer-facing leg), not the workload's own socket. The `IdentityRead`
     read is unchanged.
   - **Slice 03 (kTLS install + agent exits + wire capture)** — re-scope: kTLS arms
     on **leg B**; "agent exits" → "agent-idle forward splice + agent-light return
     splice"; the wire capture observable is unchanged (TLS 1.3 on the peer-facing
     wire).
   - **Slice 04 (fail-closed + race-window)** — fail-closed is unchanged
     (`IdentityRead` `None` → refuse handshake). The "no-cleartext-before-kTLS"
     observable is now satisfied by the userspace capture being lossless +
     confidentiality-correct (workload never reaches the peer un-proxied), NOT by a
     lossy DROP gate. Drop the server-speaks-first assumption.
   - **Slice 05 (restart-survival + WASM variant)** — **restart-survival is GONE in
     v1** (DEFER-1). Re-scope to "new connections re-handshake after an agent
     restart" (which the proxy gives unconditionally) + "the WASM and guest-stack
     variants route through the same proxy". The in-flight-survival AC is removed
     (it was the in-band model's property).
   **Product-owner edits the slice files.**

3. **Approve (or reject) the deferral GH issues** — the architect created NONE
   (CLAUDE.md: deferrals need user approval BEFORE creation). Pending the
   product-owner's decision:
   - **DEFER-1** — in-band restart-survival + 1-socket-density future optimization.
   - **DEFER-2** — fully agent-idle bidirectional kernel splice (out-of-tree kernel
     patch).
   - **DEFER-3** — multi-node reachability — verify the existing **#36** covers it
     (`gh issue view 36 --comments`) before citing; create a new issue only if not.

4. **Update the scope-boundary table** in the feature-delta DISCUSS section ("In
   scope (#26) / Out of scope") — #222 is no longer "out of scope, a SEPARATE
   feature"; it is folded in. (The architect left the DISCUSS sections intact; the
   product-owner owns the DISCUSS re-grounding.)

## What does NOT change

- The identity model: one CA, one SVID set, one trust bundle, the `IdentityRead`
  port. #26 remains a READER (never mints/caches).
- The auth-session == data-session property (rustls secrets → leg B kTLS).
- The workload-holds-nothing property.
- The wire-capture acceptance observable (TLS 1.3 records, zero cleartext on the
  peer-facing wire).
- The pinned 6.18 kernel (ADR-0068).

## Cross-references

- ADR-0069 (the decision); `brief.md` § "Transparent mTLS … extension"; the
  feature-delta § "Wave: DESIGN / [REF] …"; `design/c4-diagrams.md`;
  `design/wave-decisions.md`.
- The 6 spike findings (`../spike/findings*.md`) + 3 research docs
  (`docs/research/dataplane/`).
