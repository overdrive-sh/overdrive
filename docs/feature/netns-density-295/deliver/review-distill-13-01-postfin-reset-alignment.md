# DISTILL review — STEP 13-01 PFRO-1 post-FIN reset alignment

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP13-01-DISTILL-PFRO1-REVIEW-01` |
| Feature / roadmap step | `netns-density-295` / `13-01` |
| Review date | 2026-10-27 |
| Reviewer | Fresh independent DISTILL source reviewer |
| Iteration | 1 |
| Scope | Existing-cell post-FIN observation correction and its bounded documentation fold |
| Design authority | PFRO-1; `review-design-13-01-postfin-reset-observation.md` iteration 3 |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium, 0 low |

This review applied the repository's current `AGENTS.md`, Rust/testing/review
rules, and the nWave DISTILL, acceptance-review, completeness, test-design and
BDD criteria. Repository-specific Rust and native-Markdown conventions take
precedence over the installed review definition's generic `.feature` and YAML
examples.

## Review boundary and authority

The reviewed DISTILL-owned source correction is limited to the private remote
peer observation support and the three existing contrasting assertions in:

- `crates/overdrive-worker/tests/integration/vsock_mesh_egress_mtls.rs`;
- `docs/feature/netns-density-295/deliver/distill-13-01-postfin-reset-alignment.md`;
- `docs/feature/netns-density-295/distill/red-classification.md`; and
- `docs/feature/netns-density-295/distill/test-scenarios-p6-real-kernel-mtls-firewall.md`.

The canonical feature contract is DG-10 reading 2 and D20 as amended by
PFRO-1 in `feature-delta.md`. The exact outcome remains: dropping a claimed
registration while the loopback leg is open or half-ended resets the remote
leg-B endpoint; only a loopback leg that ended cleanly in both directions
clears linger and closes without reset. The accepted replacement evidence is
the same leg-B four-tuple's FIN then RST, represented at the Linux 7.0 peer by
`TCP_CLOSE` plus pending `SO_ERROR=EPIPE`. A read after consumed EOF is neither
positive nor negative reset evidence.

The controlling standing authority is quoted exactly in the approved DESIGN
review:

> “Stop only for a gap that changes the outcome; for anything else, take the design's evident intent and report it after the step.”

PFRO-1 changes only the falsified evidence premise, so iteration 3 approved
the canonical fold without another user-approval gate. This review does not
approve roadmap execution or replace the original STEP 13-01 crafter's exact
native triple/full-suite validation.

The shared file also contains the original crafter's activation of the twelve
existing cells by removal of their `pending DELIVER step 8` markers. Those
marker removals and all production/DES work remain the original crafter's
work. They were preserved and are neither authored nor claimed by this review.

Only this review artifact was added. No test source, production source,
feature/design artifact, marker, roadmap, execution log, DES event, model,
driver, lane, native substrate or commit was modified.

## Reviewed artifact identity

| Artifact | SHA-256 at review |
|---|---|
| `deliver/design-13-01-postfin-reset-observation.md` | `0b9319831636577ca1ce85cc79f6214325d73c94ac92ef6a494c42cfdbfef2ba` |
| `deliver/review-design-13-01-postfin-reset-observation.md` | `492c55ebc872e4876e10d72d06cb1aa414fcf252bb7ac78833939b8ec21efc02` |
| `deliver/native-13-01-postfin-reset-observation.md` | `694f919056d5b39bf40d0c5468130c0a57a22415f02264ac81bfbb5ba55fc672` |
| `deliver/distill-13-01-postfin-reset-alignment.md` | `e17381ed8af704ad569ad5da5d139ac2b54666fe89bbb66d2a68bffac156df0d` |
| `vsock_mesh_egress_mtls.rs` | `dffc6d6292288197618af686427948cdc241492ca5047930abdfa615ca7e86e0` |
| `feature-delta.md` | `f70569fb7479368d5aab9d74de2e4f898d7daaed55bd9ff3af9542376efc4e9b` |
| `distill/red-classification.md` | `c05cba54432d848d461d38c93019a5be30ff85dbd2bfa9e65e92f74c906df00c` |
| `distill/test-scenarios-p6-real-kernel-mtls-firewall.md` | `6e8f7681f73744fb74788285c5118a1d2a14777da81804c0fc7d07be542258da` |

## Strengths

- The correction observes the accepted effect on the exact accepted peer
  descriptor. The same `RecordingTcp.tcp` that completes the mTLS handshake,
  receives the request, sends the response and consumes FIN is passed by
  reference to the terminal observer.
- The first `Ok(0)` ends all reads on that descriptor. The post-FIN path polls
  only `TCP_INFO`, waits for exact state `TCP_CLOSE`, consumes `SO_ERROR`
  exactly once and stores that value in the same `PeerSession` record.
- Exact equality prevents weakened outcomes. The half-ended cell accepts only
  `TCP_CLOSE` plus `EPIPE`; the open-drop control accepts only pre-FIN
  `Reset`; both clean-close controls accept only `TCP_CLOSE` plus zero error.
- The correction keeps the existing scenario, production composition, owner,
  registered tuple, mTLS exchange, observation deadlines, zero re-resolution
  assertion and exact `/// CONTRACT_SHAPE: bounded-change.` declaration.
- The change adds no production accessor, runtime probe, public/cross-crate
  type, method, parameter, enum variant, typed project error, owner, task,
  completion promise, close mechanism, Sim seam, Quint action, scenario or
  lane.

## Exact source assessment

### Same descriptor, tuple and session

`MeshPeer::spawn_with` creates one `PeerSession` entry for each accepted peer
socket and passes that accepted `TcpStream`, its fixed session index and the
shared session store to `run_peer_session`
(`vsock_mesh_egress_mtls.rs:745-758`). `run_peer_session` wraps that socket once
as `RecordingTcp`, uses it for the real TLS handshake/request/response, and
then passes the same wrapper to `watch_termination` (`:841-912`).

After FIN, `watch_termination` passes `&rec.tcp` directly to
`observe_fin_terminal` (`:915-948`). No clone, reconnect, second accept,
alternate peer or tuple lookup intervenes. The resulting `Termination` is
written through the original session index (`:907-912`). The half-ended cell
also requires `sessions.len() == 1` before accepting that record
(`:1427-1436`). The observed terminal state and error therefore belong to the
same accepted leg-B descriptor, four-tuple and peer session that recorded the
FIN.

### Honest post-FIN observation

The raw read loop ends at the first `Ok(0)` and records `saw_fin`
(`:925-943`). Neither branch reads from the peer socket again. The
half-ended branch retains its original 60-second session deadline, while the
clean-close branch retains its original two-second settle deadline
(`:927,942-948`). The outer cell still uses the existing ten-second
`PEER_TERMINATION_DEADLINE`, and its five-second forwarder-side
`StillOpen` precondition is unchanged (`:145-147,1408-1431`).

`observe_fin_terminal` polls `TCP_INFO.tcpi_state` until exact state 7 and only
then calls `take_socket_error` (`:951-962`). Repository search finds exactly
one call to `take_socket_error`, at that transition. `TCP_INFO` does not
consume `sk_err`; the only `SO_ERROR` query is the recorded one. Its result is
kept in `Termination::Fin { tcp_state, pending_error }`, so the one-shot Linux
error is not queried a second time and accidentally replaced with zero.

Both `getsockopt` helpers operate on the live peer descriptor, provide correctly
sized output storage, check the syscall result, and retain the exact kernel
values (`:964-1012`). They are private Tier-3 test support. They do not create
a production hook or duplicate the production reset mechanism.

### Exact half-ended and control outcomes

| Existing case | Reviewed assertion | Assessment |
|---|---|---|
| Both loopback directions open at claimed-registration drop | `Termination::Reset`, plus `Closure::Reset` on leg F's forwarder side (`:1305-1346`) | **Preserved.** No FIN or EOF is accepted by the remote control. |
| Guest-to-remote FIN consumed; reverse direction open | Same peer session must be `Termination::Fin { tcp_state: TCP_CLOSE, pending_error: libc::EPIPE }`, plus `Closure::Reset` on leg F's forwarder side (`:1391-1443`) | **Exact PFRO-1 alignment.** `Open`, EOF-only, zero, another errno, another session and another flow do not satisfy equality. |
| Both directions clean before registration drop | The record before and after the drop is exact `TCP_CLOSE`/zero (`:1349-1388`) | **Preserved negative control.** No reset result can masquerade as clean unless the sole pending error were consumed elsewhere; the reviewed path has no such consumer. |
| Both directions clean in the H-5 control | Exact `TCP_CLOSE`/zero (`:1591-1625`) | **Preserved negative control.** Uses the same terminal observer as the half-ended cell and distinguishes it by the retained error value. |

The half-ended cell retains `assert_eq!(rig.resolve.calls(), 0)` at line 1440,
so the observation correction cannot pass through a second resolution. The
request/response exchange, registered forwarder address, allocation, remote
backend, mTLS identity and one-session requirement are unchanged.

There is no clean-case false positive in the reviewed observation path. The
clean peer shuts down its own write direction after FIN, reaches terminal
`TCP_CLOSE`, and must expose zero pending error. The half-ended peer deliberately
keeps that direction open; it reaches `TCP_CLOSE` only through the abort and
must expose `EPIPE`. Because the observer performs no post-EOF read and no
earlier `SO_ERROR` query, it neither clears the reset evidence nor mistakes
permanent EOF for clean completion.

## Canonical and documentation alignment

| Gate | Result |
|---|---|
| DG-10 reading 2 / D20 | PASS — same-flow reset remains mandatory for open and half-ended claimed-registration drops; clean two-direction completion remains the sole FIN-only case. |
| Native premise | PASS — the preserved production receipt identifies FIN then RST on the same leg-B four-tuple; the endpoint receipt identifies `TCP_CLOSE`, `EPIPE` and later EOF. |
| No outcome weakening | PASS — no `must`, `never`, `exactly`, reset, cleanup, security or fail-closed requirement is removed or relaxed. EOF-only remains insufficient. |
| Feature-delta fold | PASS — D20 records the independently approved PFRO-1 status and DG-10 records the exact replacement evidence while leaving native validation pending. |
| RED classification | PASS — the prior timeout is correctly classified `WRONG_ASSERTION`, not missing reset functionality; GREEN remains pending on this review plus original-crafter native validation. |
| Scenario inventory | PASS — the document now reports the actual twelve existing cells and changes no scenario identity. |
| Handoff | PASS — it assigns the existing Metal triple and complete twelve-cell suite to the original STEP 13-01 crafter and authorizes no alternate lane or probe. |

The approved DESIGN review iteration 3 explicitly leaves test-support
representation to DISTILL. The private `Termination` fields and
`TCP_INFO`/`SO_ERROR` helpers therefore implement the exact approved
observation without inventing interface-contract surface. No semantic project
error representation is required: Linux `EPIPE` is endpoint evidence, not an
Overdrive error returned to a caller.

## Acceptance-review dimensions

| Dimension | Score | Assessment |
|---|---:|---|
| Happy-path bias | 10/10 | The corrected abort cell remains paired with a positive open-reset control and two exact clean-close controls. |
| Given/When/Then form | 10/10 | The existing cell retains one precondition, one registration-drop stimulus and exact observable outcomes on both owned endpoints. |
| Domain language | 10/10 | The test and handoff preserve the precise claimed registration, loopback direction, leg F, leg B, peer endpoint and resolver terms. |
| Coverage completeness | 10/10 | PFRO-1's half-ended result and both contrasting outcome classes are represented without adding a duplicate scenario. |
| Walking-skeleton user-centricity | 10/10 N/A | No walking skeleton changes; this is the existing focused Tier-3 host-adapter cell. |
| Priority validation | 10/10 | The source corrects the reproduced invalid oracle and does not add machinery for a reset native capture already proved. |
| Observable behavior | 10/10 | Assertions observe the production peer descriptor's terminal TCP state/error and the existing forwarder-side reset, not a mock call or private production field. |
| Traceability | 10/10 | Source and documentation trace directly through PFRO-1 to DG-10 reading 2 and D20. |
| Walking-skeleton boundary | 10/10 N/A | The existing Tier-3 cell still composes the real worker/intercept/enforcement path; no in-memory substitute or boundary change is introduced. |

All applicable dimensions meet the approval threshold. Every one of the twelve
live Rust cells retains a `CONTRACT_SHAPE` declaration; the affected cell keeps
the exact required `/// CONTRACT_SHAPE: bounded-change.` line.

## Test-design mandates and completeness

| Gate | Result | Evidence |
|---|---|---|
| CM-A — production/hexagonal boundary | PASS | The existing cell still drives `register_forwarded_outbound` through the composed real worker and host enforcement adapters. Private peer inspection does not bypass production behavior. |
| CM-B — domain-language abstraction | PASS | Scenario name, comments and assertions express claimed-registration and endpoint outcomes; syscall details stay in private helpers. |
| CM-C — complete focused journey | PASS | Registration, mTLS exchange, guest FIN, retained reverse direction, owner drop, both endpoint outcomes and zero re-resolution remain in one cell. |
| CM-D — pure logic / fixture isolation | PASS | Fallible Linux observation is isolated in two private test helpers; no business/production logic is copied into them. |
| CM-E — observable universe | PASS | This layer-4/Tier-3 integration cell may use traditional assertions; the exact forwarder/peer universe and unchanged resolver complement are retained. |
| CM-F — layer-dependent PBT | PASS | The real-kernel cell remains one explicit example and adds no property generator. |
| CM-G — two-tier rich journey | PASS / N/A | No new journey or state-space tier is introduced. |
| CM-H — explicit integration sad path | PASS | The half-ended abort remains an explicitly named example with exact controls. |

For the canonical 15-item C1-C7 completeness checklist, this bounded
observation correction introduces no new input partition, collection,
operation, mode flag, typed project error, resource environment or concurrency
claim. Those eleven delta-inapplicable items count as passing under the
checklist's N/A rule. The four applicable items pass: the three terminal-state
classes are documented (C2a), exact `EPIPE` endpoint evidence is exercised
(C6b), exact equality excludes every other error (C6c), and dropping the
claimed registration while the flow is half-ended remains the explicit
interruption case (C7b). Delta completeness is therefore **15/15** without
inventing another scenario, model or lane.

## Mechanical verification

| Check | Result |
|---|---|
| Live cells in `vsock_mesh_egress_mtls.rs` | PASS — 12. |
| `CONTRACT_SHAPE` declarations | PASS — 12, including exact bounded-change declaration on the affected cell. |
| Post-FIN read search | PASS — the only peer raw read is the pre-FIN loop; no read occurs after `Ok(0)`. |
| `SO_ERROR` consumption | PASS — one helper and exactly one call, after observed `TCP_CLOSE`. |
| Open / half-ended / clean matrix | PASS — exact `Reset`, exact `TCP_CLOSE/EPIPE`, and two exact `TCP_CLOSE/0` controls. |
| Deadlines and resolver complement | PASS — constants and target `resolve.calls() == 0` assertion unchanged. |
| Public API / typed error / production mechanism | PASS — no addition in the DISTILL source correction. |
| Driver, model, scenario and lane additions | PASS — none. |
| Original crafter marker activation | PASS — all twelve marker removals remain preserved and unclaimed. |
| `cargo fmt --all --check` | PASS. |
| Scoped `git diff --check` | PASS. |
| `jq empty deliver/roadmap.json` | PASS; validation remains pending. |
| Native execution | NOT RUN — intentionally reserved for the original STEP 13-01 crafter after this source approval. |
| Mutation testing | NOT RUN — test-support-only correction; mutation remains the single final DELIVER-wave gate. |
| Commit | NOT CREATED. |

## Iteration 1 verdict

**APPROVED.** The DISTILL correction replaces only the falsified post-EOF read
oracle. It proves the half-ended outcome with exact `TCP_CLOSE` plus one retained
`SO_ERROR=EPIPE` observation on the same peer descriptor/session/tuple that
consumed FIN, keeps the open-drop reset and both clean-close zero-error controls
exact, and retains the existing deadlines, resolver complement, composition,
scenario and Contract Shape.

No accepted outcome is weakened, no false clean case is introduced, and no
production API, typed error, owner, lifecycle promise, reset mechanism, test
seam, driver, model, scenario or lane is added. There are no open DISTILL
findings. The original STEP 13-01 crafter may now run the existing Metal
open-drop/half-ended/clean-close triple and complete twelve-cell
`vsock_mesh_egress_mtls` suite. Roadmap validation remains pending until that
exact native evidence passes.

# APPROVED

---

# Review iteration 2 — same-step D9 fixture migrations

## Iteration metadata

| Field | Value |
|---|---|
| Iteration | 2 |
| Review date | 2026-10-27 |
| Scope | Three existing shared-owner lifecycle bodies that failed before their retained outcomes because their fixture bypassed D9 registration |
| Failure class | `FIXTURE_BROKEN`, not `MISSING_FUNCTIONALITY` and not a PFRO-1 semantic change |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium, 0 low |

This iteration is additive. It preserves iteration 1's PFRO-1 evidence and
approval, then reviews the bounded fixture migrations in:

- `crates/overdrive-worker/tests/integration/netns_density_shared_owner.rs`;
- `crates/overdrive-worker/tests/integration/outbound_enforce_substrate_splice.rs`;
- the `Same-step D9 fixture migration` section of
  `deliver/distill-13-01-postfin-reset-alignment.md`; and
- the corresponding classification/inventory additions in
  `distill/red-classification.md` and
  `distill/test-scenarios-p6-real-kernel-mtls-firewall.md`.

Only this native Markdown review artifact was extended. No production source,
test source, fixture, marker, roadmap, execution log, DES event, model, native
substrate or commit was modified by this reviewer. No Metal job was started;
the exact replay remains assigned to the original STEP 13-01 crafter.

## Reviewed artifact identity

| Artifact | SHA-256 at iteration 2 review |
|---|---|
| `netns_density_shared_owner.rs` | `656669056fac721af354c1ad4c05d1e6de31876b50e5f56f2e19941b65d0db79` |
| `outbound_enforce_substrate_splice.rs` | `60ec3433da90bfe95ea481c4f483b8114e495b085ead722cca3a2c251ca89ad8` |
| `deliver/distill-13-01-postfin-reset-alignment.md` | `385df1c73284fe916d467440f2068e04d9039cbb391fdc18fa62802d5a03136c` |
| `distill/red-classification.md` | `81695e197ad9c3d0ac318f96edde3dd678558def272487375b83f325943a38aa` |
| `distill/test-scenarios-p6-real-kernel-mtls-firewall.md` | `3898ecfa2d4b4649f5231f7b7058ad90409134e43aeb603c86cab5cb6be48dfb` |
| Original netns-owner failure receipt | `228638b34811f0b8cb023102c8a2991ded2d2769563b059f7cb5e8460fc50491` |
| Original outbound failure receipt | `afd47010f22ef4ed8c3a969076b2ed1ec48bf92fa4f30ac9d8d2d2cc59834e22` |

The receipts are
`.context/netns-density-295/13-01/targeted-netns-owner-postretire.log` and
`.context/netns-density-295/13-01/targeted-outbound-postretire.log`. They are
preserved failure evidence from the original crafter, not post-migration pass
evidence.

## Reproduced failure premise and correct classification

The first receipt records Metal run `e623caae0d03d4aa5ae30cad`: the existing
`owner_shutdown_waits_the_active_claim_then_drains_every_shared_capability_and_listener`
body timed out waiting for two production enforcement handles. The second
records Metal run `785ec2db92073e43085fc6c8`: both retained
`outbound_enforce_substrate_splice` bodies received early EOF on their first
exchange. All three runs exited 100 before their retained lifecycle assertions.

Before this migration, each affected client bound a source-specific socket and
connected it directly to the published leg-F listener. It never called the
accepted D9 `register_forwarded_outbound` port. The current production entry
`handle_shared_outbound` requires `claim_forwarded` to return the active
capability claim, registration state and owner-selected backend; otherwise it
drops the accepted leg-F descriptor (`mtls_intercept_worker.rs:3130-3158`).
The observed timeout/EOF therefore follows the accepted fail-closed refusal of
an unregistered tuple. Relaxing that path or restoring D14 cleartext
pass-through would be the defect.

The documentation correctly classifies these failures as `FIXTURE_BROKEN`.
They do not disprove the owner shutdown/drain or unrelated-handle outcomes,
because the old setup never reached those outcomes with a D9-admitted flow.
They also do not change PFRO-1's post-FIN observation contract.

## Exact production registration path

The two private helpers now execute the accepted public API in the production
order:

1. create the existing IPv4 `TcpSocket` and bind its existing allocation
   source address;
2. read `forwarder_local` from that still-unconnected socket, including the
   kernel-selected source port;
3. call `MtlsInterceptWorker::register_forwarded_outbound` with the active
   allocation, that exact tuple, the fixture's direct leg-F target as its
   directly dialled destination, and the already-selected backend;
4. retain the returned move-only `ForwardedOutbound` binding; and
5. connect the same bound socket to the same published leg-F listener.

That is an authentic invocation of the exact accepted surface at
`feature-delta.md:6508-6530`; no method, type, parameter, enum variant or test
seam was added. `register_forwarded_outbound` verifies the published owner and
active allocation, records the registration under the exact `forwarder_local`
key, and returns the existing move-only token
(`mtls_intercept_worker.rs:3263-3303,1105-1146`). When leg F accepts the
connection, its observed peer is the same bound socket tuple; the production
handler claims that registered tuple before spawning enforcement
(`:3130-3158,1148-1167`).

Passing the direct leg-F address as `original_destination` is honest for this
fixture boundary: it is the address the retained source-bound test socket
actually dials and the listener observes. These bodies prove shared-owner and
capability lifecycle behavior; they do not claim to replace the separate D9
guest-owner routing scenarios that use a mesh service destination. The
owner-selected backend remains distinct and exact.

## Backend and token authenticity

The recording-owner fixture factors its pre-existing loopback backend into
`shared_backend()`. `worker_with_enforcement` supplies that value to its
existing resolver, and `connect_registered_shared_source` supplies the same
value to the forwarded registration
(`netns_density_shared_owner.rs:71-83,1423-1445`). No replacement route or
second resolution is introduced.

Each real-kernel body similarly constructs its pre-existing mesh backend once,
passes it to `AllMeshResolve`, and passes the same value to every forwarded
registration (`outbound_enforce_substrate_splice.rs:886-930,984-1035`). Leg F
therefore enforces to the exact peer those bodies already used. The source
correction does not create a different peer, broaden `expected_svid`, or hide a
resolver call behind fixture logic.

All returned tokens are named local bindings such as `_first_registration`,
not wildcard `_` patterns. Rust therefore retains each move-only
`ForwardedOutbound` through the surrounding body scope. The tokens remain live
across the initial exchange, allocation stop or owner shutdown whose lifecycle
is under test. Their eventual drop follows the existing `ForwardedOutbound`
drop path; no explicit early drop or leaked `mem::forget` fabricates the
capability claim (`mtls_intercept_worker.rs:1373-1393`).

The third shutdown connections also retain their own token while enforcement
holds the active claim. Releasing the fixture barrier, not dropping a token or
client, permits the enforcement future to finish. This preserves the original
oracle that owner shutdown remains pending while one immutable claim is in
flight.

## No hidden oracle weakening

A zero-context source diff confirms that no test attribute, pending marker,
`CONTRACT_SHAPE` declaration, timeout or lifecycle assertion changed. The
source delta consists of the existing public-type import, two private fixture
helpers, backend-value factoring, and replacement of eight direct connections
with registered connections:

| Existing body | Registered connections | Retained evidence |
|---|---:|---|
| `owner_shutdown_waits_the_active_claim_then_drains_every_shared_capability_and_listener` | 3 | Two active handles before the barrier; third immutable claim; both listeners closed; shutdown waits; three handles torn down; six allocation guard drops; zero shared guard drops; exact owner call-count/address/observation complement; no allocation elements. |
| `two_real_shared_capabilities_keep_the_unrelated_tls_handle_live_after_one_stops` | 2 | Both pre-stop exchanges; second post-first-stop exchange; exact dynamic-member set; unrelated TLS handle/traffic; unchanged listener addresses, node program and shared owner; exact guard-drop counts. |
| `real_owner_shutdown_closes_admission_waits_one_claim_and_drains_every_shared_handle` | 3 | Two live exchanges; third enforcement-held claim; shutdown remains pending; all real handles/claims drain; both listener addresses rebind; exact guard counts and empty dynamic-member set. |

The retained source counts corroborate that bounded shape:

- `netns_density_shared_owner.rs`: 15 tests and 15 Contract Shape
  declarations before and after;
- `outbound_enforce_substrate_splice.rs`: 3 tests and 2 existing Contract
  Shape declarations before and after; and
- no body, scenario, assertion, test attribute or lane was added or removed.

The migration cannot manufacture a claim without production admission. A
wrong allocation, inactive capability, duplicate tuple, unpublished owner or
shutdown owner still fails through the existing typed
`MtlsInterceptInstallError` contract. A wrong source port cannot consume the
registration because the production registry is keyed by the bound socket's
exact `forwarder_local` tuple.

## Acceptance and scope gates

| Gate | Iteration 2 result |
|---|---|
| D9 exact path | PASS — resolve/backend selection is retained, registration precedes connect, and leg F claims the exact registered source tuple through production code. |
| D14 fail-closed boundary | PASS — no unregistered allowance, fallback or cleartext pass-through is restored. |
| Token ownership/lifetime | PASS — one existing move-only token per connection is retained across the flow's relevant stop/shutdown lifecycle. |
| Production composition | PASS — real worker/intercept/enforcement adapters remain in the Metal bodies; the recording body retains its existing test port and production worker dispatch. |
| Lifecycle assertions | PASS — all shutdown, claim-drain, listener, member, program, traffic and unrelated-handle assertions are byte-for-byte unchanged. |
| Contract Shape | PASS — declarations and bounded observable universes are unchanged. |
| Scenario/body count | PASS — unchanged in both files. |
| Public API / typed errors | PASS — only accepted `ForwardedOutbound` and `register_forwarded_outbound` surface is used; no new or flattened error is introduced. |
| Model/driver/lane | PASS — no Sim/Quint action, driver, expectation, scenario or execution lane is added. |
| PFRO-1 | PASS / unchanged — iteration 1's exact post-FIN oracle and approval are untouched. |

CM-A passes because the fixture now enters through the accepted production D9
port and same bound socket. CM-B and CM-C pass because the three existing
shared-owner lifecycle journeys and their domain outcomes remain unchanged.
CM-D passes because registration composition is isolated in private fixture
helpers with no copied production decision logic. CM-E passes because the
existing bounded-change assertions and complements are unchanged. CM-F through
CM-H are unchanged: these remain explicit Tier-3/integration examples with no
PBT or new journey tier.

For the C1-C7 completeness taxonomy, the migration adds no input partition,
state, collection, operation, mode, typed failure, environment or concurrency
claim. The applicable C2/C4/C6/C7 coverage is preserved by the unchanged
lifecycle bodies, exact typed admission failures and interruption/claim-drain
oracles. No `AT_GAP_IN_DELIVERY_SCOPE` or `SPECIFICATION_AMBIGUITY` is present.

## Mechanical verification

| Check | Result |
|---|---|
| Preserved failure receipts | PASS — exact run IDs, failure locations and exit 100 recorded. |
| Registration before connect | PASS — every one of the eight changed client setups registers after bind/local-address discovery and before connecting the same socket. |
| Backend identity | PASS — resolver and registration share the same pre-existing backend value in each fixture. |
| Move-only token retention | PASS — eight named token bindings remain live through the relevant body scope. |
| Test/Contract Shape counts | PASS — unchanged in both files. |
| Test-oracle/timeout/attribute zero-context diff | PASS — no changed lifecycle assertion, timeout, test attribute or Contract Shape line; added `expect` calls belong only to the registration fixture helpers. |
| `cargo fmt --all --check` | PASS. |
| Scoped `git diff --check` | PASS. |
| Native replay | NOT RUN — intentionally reserved for the original STEP 13-01 crafter after this review. |
| Mutation testing | NOT RUN — correctly reserved for the single final DELIVER-wave gate. |
| Commit | NOT CREATED. |

## Iteration 2 verdict

**APPROVED.** The three retained bodies previously failed in fixture setup
because direct leg-F connections had no D9 registration. The migration now
binds each exact source tuple, registers it through the accepted production
port with the existing allocation and backend, retains the move-only token,
then connects that same socket. Production still refuses every unregistered or
incorrect tuple.

Every lifecycle, drain, listener, dynamic-member, node-program, traffic and
unrelated-handle assertion remains unchanged, as do all timeouts, Contract
Shape declarations, bodies, counts and lanes. No claim is fabricated, no
oracle is weakened, and no production API, typed error, compatibility path,
D14 pass-through, driver, model or test is added. The original STEP 13-01
crafter may now replay these exact three existing bodies on Metal alongside the
already-approved PFRO-1 validation gates.

# APPROVED

---

# Review iteration 3 — shared-peer abort observation

## Iteration metadata

| Field | Value |
|---|---|
| Iteration | 3 |
| Review date | 2026-10-27 |
| Scope | Private terminal-observation taxonomy for the two retained real-kernel shared-capability lifecycle bodies |
| Evidence threshold | Exact native error receipt; prior Boolean-only peer-join failure was insufficient by itself |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium, 0 low |

This iteration is additive. It preserves iterations 1 and 2, including the
PFRO-1 oracle and D9 registration-fixture approvals, then reviews only the
private peer-support correction in
`crates/overdrive-worker/tests/integration/outbound_enforce_substrate_splice.rs`
and its bounded documentation fold.

Only this native Markdown review artifact was extended. No test source,
production source, temporary diagnostic, scenario, lane, roadmap, execution
log, DES event, model or commit was modified by this reviewer. No Metal job was
started; the original STEP 13-01 crafter retains the exact native replay.

## Reviewed artifact identity

| Artifact | SHA-256 at iteration 3 review |
|---|---|
| `outbound_enforce_substrate_splice.rs` | `437b8228eee17de8566c13b47435e41e3c9a9536047dd595faa0df3635133b28` |
| `deliver/distill-13-01-postfin-reset-alignment.md` | `78990c932e769df1620ee6bba8a5b0ac2940740b20165a2f872140078e8a89f9` |
| `distill/red-classification.md` | `b19e4cb674cc29a3e921e21caf082eccc9384c0185821afd99b60f19d84ae638` |
| `distill/test-scenarios-p6-real-kernel-mtls-firewall.md` | `31ed5bdb4aaa3d9097821af8be177f976c87828794113325a758e66c10dea53f` |
| First post-registration Boolean receipt | `50df5d90af1c0cb3835ddf1662ed6913f77e30112292205d0f1f3c8416c2c31d` |
| Exact peer-error diagnostic receipt | `e72322b080e4cbd2a20f4b61a03eee9a50be4b511ee5848e03dadf4e5c746e6c` |

The receipts are
`.context/netns-density-295/13-01/focused3-postdistill.log` and
`.context/netns-density-295/13-01/focused2-peer-error-diagnostic.log`. The raw
diagnostic receipt is retained. Its temporary source print was removed before
review; repository search finds no surviving `eprintln!`, `dbg!`, raw errno
print or `shared peer terminal read error` diagnostic in the test source.

## Evidence progression

The first post-registration replay, Metal run
`ad70ecaabd63f14b53bc6012` / nextest
`895a9f95-aa4a-4ca1-b9bd-6f844ecc5868`, established only this:

- the recording-enforcement body passed after the D9 fixture migration;
- both real-kernel bodies reached their retained lifecycle paths; and
- their peer-owner join returned `false`.

That Boolean could not distinguish reset, clean EOF, TLS failure, timeout or an
arbitrary I/O error. It was correctly treated as suspicion, not authority for
loosening the terminal oracle.

The bounded failing diagnostic then ran only the two real-kernel bodies on the
durable Metal route:

- Metal run `d612c668bf4e92d7fa10006e`;
- nextest run `f74b2eeb-c710-49f9-82ca-d30e47448a88`;
- three `ConnectionReset`, raw errno 104 observations for the three
  owner-shutdown peers; and
- one `ConnectionReset`, raw errno 104 observation for the first allocation
  stopped by the independent-capability body.

Both tests still failed at `SharedPeerOwner::finish`, so this receipt is an
honest RED for the old Boolean recorder: production emitted the required
abort, while the recorder accepted only EOF/`UnexpectedEof` and rejected the
actual reset.

## Exact termination contract

The accepted lifecycle contract is abortive, not merely “connection ended.”
The already-established production path sets `reset_peer_on_reclaim`, installs
`SO_LINGER{1,0}` on both owned legs, joins the pumps, then closes those legs
(`overdrive-dataplane/src/mtls/mod.rs:313-353,428-446`; outbound leg B is armed
at `mtls/outbound.rs:107-111`). DG-10/D20 require a still-open registered flow
drained by allocation stop or owner shutdown to reset its remote peer.

The new private `SharedPeerEnd` has two explicit meanings:

| Expectation | Accepted terminal observation | Rejected observations |
|---|---|---|
| `AbortRequired` | Only `ConnectionReset`, `ConnectionAborted`, or `BrokenPipe`, and only after `drive_server_handshake` succeeds | `Ok(0)`, rustls `UnexpectedEof`, timeout, handshake/authentication failure, echo/flush failure, and every other I/O error |
| `EofOrAbort` | `Ok(0)`, rustls `UnexpectedEof`, or the same closed abort set, after successful handshake | Timeout, handshake/authentication failure, echo/flush failure, and every other I/O error |

The three abort kinds are not an arbitrary generic-I/O broadening. They are
the exact closed reset classifier already used by the sibling
`vsock_mesh_egress_mtls::is_reset` acceptance support
(`vsock_mesh_egress_mtls.rs:620-626`). `ConnectionReset` is the reproduced
Linux read result; `BrokenPipe` is Rust's `EPIPE` projection, which the accepted
failure projection already classifies with `ECONNRESET` as `PeerReset`
(`feature-delta.md:4982-4988`); `ConnectionAborted` is the remaining explicit
abort-kind member of that existing closed classifier. No `Other`, generic
`io::Error`, EOF, timeout or TLS truncation is admitted as an abort.

This is narrower than the old recorder for every abort-required peer: the old
code accepted clean EOF and rustls `UnexpectedEof`; the corrected code rejects
both and requires an explicit abort error. It broadens only the previously
missing representation of the already-required reset.

## Per-connection assignment

`SharedPeerOwner::spawn` now receives one expected terminal class per accepted
connection. The assignments match the existing body order and lifecycle:

| Existing peer | Expected class | Contract basis |
|---|---|---|
| First allocation in `two_real_shared_capabilities...` | `AbortRequired` | `stop_alloc(first)` drains its still-open registered flow after a completed byte-exact exchange. The diagnostic observed errno 104. |
| Unrelated second allocation in `two_real_shared_capabilities...` | `EofOrAbort` | It completes both byte-exact exchanges, then the client is explicitly dropped before the body stops the second allocation. Its final peer termination is fixture cleanup, not the stopped-first-allocation oracle. |
| All three peers in `real_owner_shutdown...` | `AbortRequired` | Owner shutdown drains every still-open registered flow; the diagnostic observed errno 104 on all three. |

Only the unrelated second client's final post-drop cleanup permits EOF or
rustls `UnexpectedEof`. That permission cannot conceal failure of its retained
traffic/liveness contract: the body first completes
`metal-shared-second-before-stop` and
`metal-shared-second-after-first-stop` through the unchanged
`assert_shared_exchange` calls. If its handshake, echo or post-first-stop
traffic fails, the body fails before cleanup.

The first stopped allocation likewise completes its byte-exact exchange before
`stop_alloc(first)`. In the shutdown body, the first two peers complete their
existing byte-exact exchanges before shutdown. The third peer retains the
existing admission barrier: the backend connection is accepted, shutdown is
proved pending on the in-flight claim, and only then is the peer released.
`shared_peer_connection` must complete the real mTLS server handshake before
any terminal error is classified. An early reset, TLS/authentication failure or
failed handshake therefore remains a failure for that third peer.

## Handshake, data and timeout integrity

The correction leaves `drive_server_handshake` unchanged. It returns `false`
for EOF, TLS packet/authentication error, TLS write failure or any socket error
before handshake completion (`outbound_enforce_substrate_splice.rs:407-437`).
`shared_peer_connection` checks the new terminal taxonomy only after that
function returns `true` (`:670-703`).

Every positive application read still writes the exact bytes back and flushes;
either failure returns `false`. Client-side `assert_shared_exchange` remains
unchanged and still requires the exact byte-distinct marker under its existing
15-second deadline (`:823-832`). `WouldBlock` and `TimedOut` merely continue
until the unchanged overall 15-second peer deadline; reaching that deadline
returns `false`. All unlisted errors return `false` immediately.

The peer's accepted count, third-connection barrier, release ordering, child
join, shutdown-pending assertion, connection setup and lifecycle assertions
remain intact. The correction changes only the private terminal interpretation
after authenticated handshake and retained traffic.

## No hidden outcome or scope change

A zero-context diff and source count audit confirm:

- the file still has 3 tests and the same 2 live Contract Shape declarations;
- both retained lifecycle bodies, test attributes and names are unchanged;
- every original byte-exact exchange remains;
- `wait_third`, the 100 ms shutdown-pending assertion and `release_third`
  remain in the same order;
- all stop, shutdown, listener-rebind, dynamic-member, node-program,
  guard-count and unrelated-handle assertions remain unchanged;
- no production source, public/cross-crate API, typed project error, scenario,
  body, model, driver or lane is added; and
- no temporary diagnostic source remains.

The documentation accurately distinguishes the four proven abort-required
peers from the one unrelated final-cleanup peer. It retains the raw diagnostic
receipt and does not claim the diagnostic run passed. The post-correction
three-body replay remains pending with the original crafter.

## Acceptance and completeness gates

| Gate | Iteration 3 result |
|---|---|
| Native evidence | PASS — four exact errno 104 / `ConnectionReset` observations through the registered real-kernel path. |
| Abort semantics | PASS — all stop/shutdown peers require an explicit closed-set abort error; EOF is rejected. |
| Cleanup exception | PASS — only the unrelated second client after explicit client drop permits EOF/`UnexpectedEof`; its two traffic exchanges remain mandatory. |
| Authentication | PASS — terminal classification is unreachable until the existing mTLS handshake succeeds. |
| Traffic | PASS — every retained echo/flush and client byte-exact assertion remains. |
| Deadlines | PASS — existing handshake/socket/application deadlines remain; timeout cannot pass. |
| Third claim | PASS — admission barrier, shutdown-pending proof and release ordering remain exact. |
| Contract Shape / body count | PASS — unchanged. |
| Public API / production | PASS — private test support only; no production or interface change. |
| PFRO-1 / D9 registration | PASS / unchanged — iterations 1 and 2 remain intact. |

CM-A passes because the peer still observes the real registered worker and host
enforcement path. CM-B and CM-C pass because the precise owner-shutdown,
allocation-stop and unrelated-handle outcomes remain explicit. CM-D passes
because the terminal taxonomy is isolated in private peer support. CM-E passes
because the bounded observable universe is strengthened for the four abort
peers and unchanged elsewhere. CM-F through CM-H remain satisfied: explicit
Tier-3 examples, no PBT, no new journey or failure scenario.

For C1-C7, this correction changes no input partition, state machine,
collection, operation, mode, public typed error, environment or concurrency
claim. C6 becomes more exact: known abort representations form a closed set,
while EOF, timeout, authentication failure and all other errors are excluded.
The existing C2/C4/C7 lifecycle and interruption coverage remains unchanged.
No `AT_GAP_IN_DELIVERY_SCOPE` or `SPECIFICATION_AMBIGUITY` is present.

## Mechanical verification

| Check | Result |
|---|---|
| Exact diagnostic receipt | PASS — run IDs, four errno 104 observations and failing join are retained. |
| Abort-required peers | PASS — first stopped allocation plus all three owner-shutdown peers. |
| EOF-capable peer | PASS — only unrelated second client's post-drop cleanup. |
| Closed abort set | PASS — `ConnectionReset`, `ConnectionAborted`, `BrokenPipe`; matches existing sibling reset support. |
| Handshake/authentication gate | PASS — unchanged and precedes terminal classification. |
| Echo/flush and byte-exact exchanges | PASS — unchanged. |
| Timeout/other-error rejection | PASS — unchanged deadlines; no additional accepted error kind. |
| Test and Contract Shape counts | PASS — 3 and 2 before/after. |
| Temporary diagnostic source | PASS — absent; raw log retained. |
| `cargo fmt --all --check` | PASS. |
| Scoped `git diff --check` | PASS. |
| Native replay | NOT RUN — intentionally reserved for the original STEP 13-01 crafter after this review. |
| Mutation testing | NOT RUN — correctly reserved for the single final DELIVER-wave gate. |
| Commit | NOT CREATED. |

## Iteration 3 verdict

**APPROVED.** Exact native diagnostics now prove the old Boolean peer support
rejected the required production reset: all three shutdown peers and the first
stopped-allocation peer returned `ConnectionReset`/errno 104 after the
registered production path. The correction requires a closed abort taxonomy
for those four peers and permits EOF/truncated EOF only for the unrelated
second client's final post-drop cleanup.

Handshake/authentication, echo/flush, byte-exact traffic, deadlines, third-
claim admission ordering and every lifecycle assertion remain mandatory. No
arbitrary I/O error, clean EOF on an abort-required peer, failed TLS session or
timeout can pass. No production behavior, API, typed project error, scenario,
body count, Contract Shape, model, driver or lane changes. The original
STEP 13-01 crafter may now repeat the exact three-body durable Metal replay.

# APPROVED
