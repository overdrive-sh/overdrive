# STEP 14-01 UDP-deferral DISTILL alignment

## Status

The bounded acceptance alignment authorized by the approved UDP-deferral
design is complete in the existing P6-S13 body and private support. The scoped
source gate is green. The exact 117-body native runner was attempted from the
same source identity and reached an honest semantic RED at the retained
TCP/HTTP probe outcome: neither readiness probe produced the required initial
`Pass` row. This is an outcome-changing production-boundary gap; this DISTILL
handoff does not weaken the oracle or claim the later retained outcomes ran.

The roadmap validation was subsequently restored to `approved` by the
independent DESIGN release after the alignment review completed. The private
fixture finding and remediation recorded below do not falsify that DESIGN or
self-change roadmap validation. No production change, DES event, commit, new
test identity, scenario, lane, selector, public API, override seam or #310
mechanism is included in the DISTILL work.

## Authority and exact scope

The user approved this correction verbatim:

> “yeah, let's not do that now. add all the relevant info to the github issue, and then we keep it deferred to #310 and correct the contradictory acceptance checklist.”

`design-14-01-udp-deferral.md` and its independent review
`review-design-14-01-udp-deferral.md` release DISTILL to remove only the
Service-VIP UDP-to-VM action and its gate. Canonical V-6 and roadmap criterion
3 still require off-node TCP/UDP (including a genuine empty datagram), gateway
DNS, node-source identity, mesh-name and issued-VIP TCP with mTLS, remote
inbound TCP, TCP/HTTP application-state probes, V-LEGB-SERVE no-uplink and
socket identity, the bounded `IP_BIND_ADDRESS_NO_PORT` comparison, public
stop, VMM disappearance and cleanup/loss accounting.

The general D24 guest-datagram `connect4` contract and its existing tests are
unchanged. This alignment adds no non-VM backend or alternate UDP path.

## Bounded source alignment

The existing P6-S13 body remains the one CLI identity with its existing
`CONTRACT_SHAPE: bounded-change.` declaration. The five control-plane
identities and runner composition are unchanged, preserving
`111 + 1 + 5 = 117`.

The aligned source does the following:

- removes the Service-VIP UDP port, request, response, peer listener/server,
  client socket exchange, `vip-udp ok` marker and body dependency;
- keeps `paths-ready` gated by the retained mesh-name and issued-VIP TCP byte
  exchanges;
- keeps the peer VM Service and client VM Service distinct;
- keeps both dedicated readiness listeners and the public
  `Pass -> newer Fail -> newer Pass` oracle on the unchanged peer allocation;
- keeps the route-disjoint same-CA remote mTLS client, loss-accounted
  loopback/uplink captures, exact socket identities and bounded population
  comparison;
- changes only the private `RemotePeer::wait_for` error carrier from
  `PeerLog` to `Box<PeerLog>`, preserving the same last-log error value while
  resolving Clippy's `result_large_err` finding without an allow or skip.

An exact source search has no `V6_UDP`, `vip-udp` or Service-VIP UDP action in
the four P6-S13/private-support files. Production code and the general D24
mechanism tests were not edited by this alignment.

## Retained-outcome traceability

| Retained outcome | Exact stimulus and fail-closed oracle | Source | Native receipt disposition |
| --- | --- | --- | --- |
| Public deploy/serve Job; off-node TCP, non-empty UDP and genuine empty UDP | The public deploy handler starts the VM Job. The remote peer must record exactly one TCP request, one tagged datagram and one `recv_from(n == 0)` datagram, all from the node-side address; the console must contain the three success markers; the terminal row must be `Terminated` and its VMM absent. | `vsock_walking_skeleton.rs:50-100`; `guest_peer.rs:239-287` | **Executed and passed before the later panic** in `a6dd517d4325327656fd47a6`; control reached peer-Service deployment at line 102. |
| Peer VM Service, issued VIP/backend and declared TCP application port | A separate peer Service is submitted through public deploy; public describe supplies the VIP, leased workload address and listener. | `vsock_walking_skeleton.rs:102-124`; `vsock_support.rs:290-321` | **Executed**; the peer reached Running and printed `v6-peer: ready population=65`. |
| Exact public TCP/HTTP readiness state | Public describe must expose readiness index 0 as TCP port 18082 and index 1 as HTTP `/healthz` port 18083 on the same allocation, then observe both `Pass`, both newer `Fail`, and both newer `Pass`. Missing, partial, stale or replacement-allocation evidence fails. | `vsock_walking_skeleton.rs:125-126,162-179,220-271`; `vsock_support.rs:290-321`; `guest_peer.rs:289-470` | **Semantic RED.** The exact descriptors were public, but both latest rows were `Fail { last_fail_reason: "unreachable" }`; no initial `Pass` pair existed by the 30-second bound. |
| Route-disjoint same-CA remote inbound TCP | A private remote client SVID comes from the production persisted CA; the client verifies the peer chain/SPIFFE URI and exchanges byte-distinct records with the registered VM backend tuple; the peer console must record the request. | `vsock_walking_skeleton.rs:128-133`; `vsock_support.rs:148-223,463-566` | **Not reached after semantic RED; no credit claimed.** |
| Client VM Service gateway DNS, mesh name, off-node TCP/non-empty+empty UDP, issued-VIP TCP | The distinct client Service reads `/etc/resolv.conf`, resolves `v6-peer.svc.overdrive.local`, performs byte-distinct name/VIP TCP exchanges and both off-node UDP forms; the body requires gateway `100.95.0.1`, the resolved-name marker, `vip-tcp ok`, exact remote-peer cardinalities and node-source addresses. | `vsock_walking_skeleton.rs:135-158,197-200`; `vsock_support.rs:323-341`; `guest_peer.rs:548-624` | **Not reached after semantic RED; no credit claimed.** |
| Issued-VIP TCP is bidirectional TLS 1.3 kTLS with no cleartext | Capture starts before client deployment; the held tuple must correlate to one fd/inode with `tcp-ulp-tls`, TLS 1.3 and software TX/RX; sealed bidirectional reassembly must contain application-data records, no marker plaintext, no loss, truncation or ambiguity. | `vsock_walking_skeleton.rs:139-160`; `vsock_support.rs:568-756` | **Not reached after semantic RED; no credit claimed.** |
| V-LEGB-SERVE no-uplink and loopback leg-C identity | A disposable default-route/veth positive control proves capture is live; the held backend tuple must have exactly one loopback leg-C peer and zero exact-tuple uplink frames with zero loss; route/link/netns absence is read back. | `vsock_walking_skeleton.rs:136-139,187-195`; `vsock_support.rs:977-1048` | **Not reached after semantic RED; no credit claimed.** |
| Bounded `IP_BIND_ADDRESS_NO_PORT` set/unset comparison | The unset cohort proves option 0, exact backend tuples, narrowed-range unique source ports, the terminal exhaustion failure and post-close absence. The production set cohort proves option 1 on every exact tuple, all sources in range, fewer sources than destinations, one fd owner per inode and post-close absence. | `vsock_walking_skeleton.rs:135,181-186,215-217`; `vsock_support.rs:758-975` | **Not reached after semantic RED; no credit claimed.** |
| Public stop, VMM disappearance and final cleanup | Both Services stop through the public operator verb, each allocation becomes terminal, both VMMs disappear, serve shuts down, population rows are absent and the ephemeral range is restored. | `vsock_walking_skeleton.rs:202-217` | **Not reached after semantic RED; failure teardown killed the peer scope. No success-path cleanup credit claimed.** |

The deferred Service-VIP UDP-to-VM action appears in none of these oracles and
receives no pass disposition. It remains deferred to #310.

## Durable source and execution evidence

Both decisive records identify commit
`c9f2f6d6975b236fa4a049093550d3ea8dcf4242` and the identical synchronized
source digest
`f4fac4fe3c0f99241c5ad3d8757780df6a2cc2df0b1128889fa6a399303ad6da`.

| Purpose | Metal run | Exact terminal result | Durable raw evidence |
| --- | --- | --- | --- |
| Final aligned source gate | `50d017156bfcb85944a1bd32` | exit 0, `command_returned` | `.context/netns-density-295/14-01/raw/50d017156bfcb85944a1bd32/` |
| Exact existing cgroup-default runner, intended 117 bodies | `a6dd517d4325327656fd47a6` | exit 101, `command_returned`; semantic RED in the unchanged P6-S13 identity | `.context/netns-density-295/14-01/raw/a6dd517d4325327656fd47a6/` |
| Read-only physical-host audit | `d87ba6b4e1fb0bb400f79e39` | exit 0, `command_returned` | `.context/netns-density-295/14-01/raw/d87ba6b4e1fb0bb400f79e39/` |
| Read-only pre-existing-netns diagnosis | `41f74f715171fd6cfb5cdb4f` | exit 0, `command_returned` | `.context/netns-density-295/14-01/raw/41f74f715171fd6cfb5cdb4f/` |

The final source gate ran, through `cargo xtask metal run`, all of:

```text
cargo fmt --all -- --check
cargo check -p overdrive-testing --bin overdrive-guest-peer --features integration-tests
cargo clippy -p overdrive-testing --bin overdrive-guest-peer --features integration-tests -- -D warnings
cargo check -p overdrive-cli --test integration --features integration-tests,kvm-tests
cargo clippy -p overdrive-cli --test integration --features integration-tests,kvm-tests -- -D warnings
```

Its output SHA-256 is
`60d76a98d61479ef92439a8e7776fe7d7b79a9a757590e522c06bcb779bdfdfc`.
The native output SHA-256 is
`5080fc263aee729183c28e77765da823a17504945a64357bc49d537deb2c15fd`.

Before the CLI extension failed, the runner recorded 101 successful libtest
summaries with exactly 111 passed bodies, zero failed and zero ignored. It
then invoked the exact P6-S13 identity once; that test process exited 101 at
the public probe oracle, so the runner correctly failed closed and did not
launch the five control-plane bodies. There are no recognized runtime-SKIP
diagnostics; the three kernel boot messages containing the lowercase word
`skipped` are CPU calibration messages, not test skips.

## Semantic RED

The decisive native failure is at
`vsock_walking_skeleton.rs:261`, reached through the real in-process
production `serve`, public deploy handler, nested real VM and production
probe observation surface. The peer Service was Running, its guest program
bound the dedicated application/HTTP/population listeners and printed its
ready marker, and public describe returned the exact two readiness
descriptors. At the 30-second deadline, public results for allocation
`alloc-v6-peer-0` were:

```text
readiness idx=0 Tcp(18082): Fail { last_fail_reason: "unreachable" }
readiness idx=1 Http(/healthz,18083): Fail { last_fail_reason: "unreachable" }
```

The required initial pair of `Pass` rows therefore does not exist. DISTILL
does not assign a production root cause from this evidence, modify production
code, relax the public probe oracle, substitute direct probers, or move the
criterion. The STEP 14-01 crafter must treat this receipt as semantic RED at
the retained production boundary.

## Cleanup and confinement

All kernel-sensitive and real-VM work ran through the existing Metal runner
inside disposable GuestKit/Cloud Hypervisor guests. Lima was not recreated.
The read-only audit found no live Cloud Hypervisor process, no nft table and
no STEP 14-01 physical-host work directory after the failed run.

The audit also listed `ovd-ns-0000`. The follow-up read-only diagnosis proves
that namespace predates this work: its mount mtime is
`2026-10-26 00:25:02 +0000`, it has no member process, and it contains the
pre-existing `ovd-wl-0000` veth/address. This handoff neither created nor
removed it and does not misstate the physical host as namespace-empty.

## Exact aligned file identities

| File | SHA-256 |
| --- | --- |
| `crates/overdrive-cli/tests/integration/vsock_walking_skeleton.rs` | `2e9ede9135e82065375bbb3b8c4a6f5c7fc28cc5c24b3da2aad560c9bf725d89` |
| `crates/overdrive-cli/tests/integration/vsock_support.rs` | `08d07e0336f8e6c86902bd082e98541c7ef09b5f3840cf1b14762412cebff2e1` |
| `crates/overdrive-testing/bin/guest_peer.rs` | `a83191b2e9b817dfa7da27bf806fc406fec2a361688fba81a03705675e4e4136` |
| `crates/overdrive-testing/src/guest_app.rs` | `395c3fc937de129f66a751edf1226be5f928f592d003fb98ab11219d2c80f4c7` |
| `crates/overdrive-cli/Cargo.toml` | `62dc418667abeb974210bf975712bb5a1a60288a4a1d9b8c412ad908700e51d1` |
| `Cargo.lock` | `ed270b0d8c385186b1c75e0eb955dd8dc72cac30469486154cf9ba5b36490c5a` |

`AGENTS.md`, canonical design/roadmap edits, production crafter changes and
all unrelated dirty work remain outside this alignment's ownership and were
not reverted or committed.

## Iteration 2 — private recovery-channel remediation

Independent DISTILL review iteration 2 rejected established issued-VIP stream
survival as a product or DESIGN requirement. The accepted readiness contract
withdraws and restores backend eligibility for new traffic and expressly does
not promise survival of an existing connection. Review finding F-01 therefore
owned one exact private-fixture correction.

The existing guest workload now implements that correction without changing
the P6-S13 body:

- after reading and acknowledging only `V6-STATE:unhealthy`, the peer handler
  sets the real shared health flag false, emits its existing unhealthy marker,
  waits the existing twelve-second failed interval without further stream I/O,
  sets that same flag true and emits the peer healthy marker;
- the client sends only the unhealthy command on the initial issued-VIP
  stream, then drops that stream instead of requiring it to survive backend
  withdrawal;
- after the same twelve-second interval, the client retries for at most the
  existing thirty-second bound to open a fresh connection to the same public
  issued VIP and application port, and it repeats the existing byte-distinct
  `V6_VIP_REQUEST` / `V6_VIP_RESPONSE` exchange;
- only that successful post-recovery traffic restoration emits the existing
  `v6-client: state=healthy` marker; the fresh stream, rather than the old
  stream, is held for the downstream exact socket/population/no-uplink
  observations.

The public body remains byte-for-byte unchanged from the iteration-2 review:
it still requires initial `Pass`, newer `Fail` and newer `Pass` for both exact
readiness descriptors on the same peer allocation. No console-only state,
direct prober, result-row injection, replacement allocation or connection-
retention assumption can satisfy it.

The remediated guest workload SHA-256 is
`9bf7e7e46f3ace97ef2c9dcad216e67ef1646f4617082fdd82107e9dead2a7fa`.
The original crafter's separate HTTP-prober candidate remains untouched by
this DISTILL change at
`d9296f0a002de28defe9f73ba0854eafcb2e400af0ab018010e48709b7045c55`.

### Iteration-2 durable evidence

The final source gate and both native attempts name commit
`c9f2f6d6975b236fa4a049093550d3ea8dcf4242` and synchronized source digest
`2fe5f3057dcfdea7d005198b9edd44f8d7cff563e4a6f5fb5930e6650127940b`.

| Purpose | Metal run | Result | Output SHA-256 |
| --- | --- | --- | --- |
| Initial formatting diagnostic | `4029eea8355439bffe5184be` | exit 1; rustfmt diff only, applied exactly before the decisive gate | `edf9bd9859bb90aae3954ca71445018c5dbdbb0b34c76e4f64219fd7279452d6` |
| Decisive fixture/CLI source gate | `9055199a935c497303fcaea5` | exit 0, `command_returned` | `a7b8d638ddeb49aba592b193e7d85b4dcc23dbf6cc2e8f900fdfa9df41e50a3d` |
| Exact existing 117-body runner, attempt 1 | `100cce0a3d7d451c293de4b1` | exit 101 at the initial public readiness `Pass` gate | `46a17977776e57e0b97d6c053f4323e7a58a78ff9a8fa0189353fd415e9ad902` |
| Exact-source retry | `cdead78197b4a7221fe4fa36` | exit 101 at the same initial public readiness `Pass` gate | `911cc1c631c952b96336189f19ddad6d30b5fa4a47fbaf396f1371503dcc3902` |
| Read-only physical-host cleanup audit | `a3f6d3ee50385104fd330145` | exit 0, `command_returned` | `986b2c2e29b7b2b706e956f3dd3aa2bf51b41e5ae5667c602b761ea8e2b31de0` |

The decisive gate again ran rustfmt, check and Clippy with `-D warnings` for
the guest workload and CLI integration target. It is green.

Both exact native attempts retained 101 successful libtest summaries totaling
111 passed, zero failed and zero ignored before invoking the exact P6-S13 CLI
identity. Attempt 1 ended with TCP readiness `unreachable` and HTTP readiness
`connection reset`. Attempt 2 observed TCP readiness `Pass`, while HTTP
readiness remained `connection reset`. Both failed at
`wait_probe_state(..., true, ...)` before deploying the client Service, so the
new private recovery sequence did not execute and receives no invented native
credit. The five control-plane identities were not launched after the
fail-closed CLI exit.

This earlier retained HTTP readiness failure is independent of F-01: it occurs
before any client Service, state command, failed interval or fresh recovery
connection exists. DISTILL made no additional fixture or product change to
hide it. The original crafter's HTTP-prober candidate remains subject to its
DELIVER investigation and review; a later exact-source native run must still
prove all 117 bodies.

The cleanup audit again found no live Cloud Hypervisor process, nft table or
STEP 14-01 work directory. It listed only the already documented pre-existing
`ovd-ns-0000` namespace. All real-VM and kernel-sensitive work remained inside
the existing disposable GuestKit boundary, and Lima was not recreated.

## Iteration 3 — bounded HTTP request-consumption correction

The next exact native receipt `5edb0decd8f9dcd2e0da458d` reached TCP
readiness `Pass` while HTTP `/healthz` still reported
`Fail { last_fail_reason: "connection reset" }`. The production HTTP-prober
candidate remained unchanged and had already observed the complete HTTP 200
response in diagnostic `48a5b24dd855d5fa22e72d82` and passed the initial
gate in `3cb8fe14a208f2f315e8f6e5`.

The remaining intermittent reset was reproduced at the private guest fixture
boundary. `v6_http_accept` used `TcpStream::peek`, wrote the status/body,
half-closed and dropped the socket without consuming the request bytes. Linux
can reset a closed TCP socket that retains unread request data, matching the
client's observed `connection reset`. This is a bounded fixture cause; it does
not select a production lifecycle or ordering mechanism.

The private HTTP workload now consumes one complete request head through the
existing stream before responding:

- the existing one-second read deadline is preserved;
- the read is bounded to 8,192 bytes and requires the `\r\n\r\n` terminator;
- EOF, timeout/read failure or a head exceeding the bound fails the fixture
  connection and emits its existing request-read diagnostic;
- the exact ports, shared health flag, HTTP 200/503 statuses, response bodies,
  `Connection: close`, half-close, readiness descriptors and public
  `Pass -> newer Fail -> newer Pass` assertions are unchanged.

No acceptance body, scenario, production API, product mechanism, lane,
selector, timeout contract, marker or state sequence changed. The original
crafter's production HTTP-prober source remains byte-identical at
`d9296f0a002de28defe9f73ba0854eafcb2e400af0ab018010e48709b7045c55`.
The corrected guest workload SHA-256 is
`0a1c76962a370f010b39337d8a97ea15fd469043aaba6639129835383b5feb3f`.

### Iteration-3 evidence and remaining RED

The source gate, native run and cleanup audit share synchronized source digest
`1fb156b1957d7e79fac0d76b5c5a89ddb00d4f75e2dcde77c4ee3ec0bd3e5b1c`.

| Purpose | Metal run | Result | Output SHA-256 |
| --- | --- | --- | --- |
| Corrected fixture/CLI source gate | `8c190158fb86f85a81fb1474` | exit 0, `command_returned` | `7cb1b2c1cc599ea3c7249a68861e8fe799ce2d021599ea26a8584222cf1c970f` |
| Exact existing 117-body runner | `17e58b1b7e76e89bdc58cd4f` | exit 101 at the later production population gate | `77a0319ee59d76f19067c11a508efb808c1bd1100827063bef0b84217a15496b` |
| Read-only cleanup audit | `7ea0663ef77f63755d502eb8` | exit 0, `command_returned` | `e6a396ee856c37da3d91c3ccc9341268a232a49dbba680e29195b0f9c4df65eb` |

The corrected native run passed the former HTTP reset boundary. Control
reached all statements before the population wait at
`vsock_walking_skeleton.rs:181-185`, which proves, without substituting
console evidence for body assertions:

- the exact initial TCP and HTTP readiness pair reached `Pass`;
- route-disjoint remote inbound, gateway DNS, mesh-name TCP, issued-VIP TCP
  and the loss-accounted mTLS/no-cleartext oracle passed;
- both newer public readiness rows reached `Fail` on the same peer allocation;
- the peer's real application health independently recovered;
- the client established the required fresh byte-distinct issued-VIP round
  trip and emitted `state=healthy`; and
- both still-newer public readiness rows reached `Pass` on that same
  allocation.

The fixture correction is therefore non-vacuously proved through the exact
production composition and closes the earlier HTTP `connection reset` and
private recovery-channel blockers without assertion weakening.

The same run then reached an honest later semantic RED: the client logged
`v6-client: population: Connection reset by peer`, never emitted
`population-held=65`, and P6-S13 failed at line 185. The population, exact
leg-C/no-uplink observation, public stop and success cleanup suffix receives
no credit. The runner retained 101 successful libtest summaries totaling 111
passed, zero failed and zero ignored before the exact CLI identity; the five
control-plane identities were not launched after its fail-closed exit. No
recognized runtime-SKIP diagnostic occurred.

DISTILL did not alter population retries, destination tuples, option-state
oracles, production code or the body to fit this later failure. A subsequent
bounded owner must classify that semantic RED separately; complete
exact-source 117-body GREEN evidence remains outstanding.

The cleanup audit found no live Cloud Hypervisor process, nft table or STEP
14-01 work directory and again listed only the documented pre-existing
`ovd-ns-0000`. All native effects remained inside disposable GuestKit on
Metal; Lima was not recreated.

## Iteration 4 — population-reset diagnostics

The population reset's exact owner and cause remain unproven. This iteration
adds observation only to the existing private workload and support; it does
not alter population success, retry, timeout, held-stream cardinality, body
assertions, production code or the six-identity/117 inventory.

### Diagnostic source

The client population helper still uses one fixed thirty-second deadline,
iterates the same indices and public issued-VIP ports, writes the same request,
requires the exact same ACK and retains only acknowledged streams. Its final
error now carries:

- population index and exact VIP:port;
- one of `CONNECT`, `WRITE`, `READ` or `ACK`;
- raw errno when the kernel supplied one; and
- the underlying error or mismatched expected/received bytes.

The peer population handler records exact index, declared port, local/peer
tuples and `ACCEPT`, `REQUEST`, `ACK` and `CLOSE` stages. After a valid ACK it
blocks in a closure read while retaining the same live stream; the population
client sends no post-ACK data. Thus closure becomes observable without
changing a live acknowledged stream's ownership or count.

The existing private `Lane` support installs the repository's existing
tracing subscriber before in-process `serve`, at INFO plus worker/dataplane
DEBUG. The trace is output only and is never read by an assertion. A timeout
now identifies the requested allocation and includes the retained console
snapshot of every allocation, so client and peer workload stages are available
together on a future failure. This final all-console addition followed the one
authorized native reproduction after that receipt showed the client stage and
production trace but not the peer console; it changes timeout rendering only.

Exact final diagnostic source identities are:

| File | SHA-256 |
| --- | --- |
| `crates/overdrive-testing/bin/guest_peer.rs` | `5a28e5aab223a38b4b1391130f48a21c8d28b46b89345994eae60f19c2a254ec` |
| `crates/overdrive-cli/tests/integration/vsock_support.rs` | `ad5d80cd7e535afa49f5d6f6916a8d51773563630ffa7af852531f5d8ad6fca9` |
| P6-S13 body | `2e9ede9135e82065375bbb3b8c4a6f5c7fc28cc5c24b3da2aad560c9bf725d89` unchanged |
| production HTTP prober | `d9296f0a002de28defe9f73ba0854eafcb2e400af0ab018010e48709b7045c55` unchanged |

Independent review iteration 5 approved the diagnostic design before native
execution. The pre-native decisive source gate was
`d24a016595077e7e3e546489`, exit 0, synchronized digest
`9a4bbeed97c932c1f2e460bbfbc97fd084e6e2b19da1ee45adf0735bf2beee35`.
After adding the all-console timeout rendering, final source gate
`49909e2115de1404ed681a1b` exited 0 with synchronized digest
`84b1033ab7c90d64c28953b783d306da1e4da864e755cfaf1c3e148239ec632f`.

### One authorized native reproduction

Exact existing-runner receipt `b2d55cadd3db0611ef5a7d59` used the reviewed
pre-native diagnostic source, synchronized digest
`6a20d5054ce1907c7c9fe768c0fe4ad79824cb57b67cfcdeff11adc2cfd3c0f1`,
and exited 101. Its output SHA-256 is
`a641c0a1f5ce8e75d3ce77d283d2b6a91f91f251a020c5dfdd94704cf59ff0d1`.

The actual failure is now precise:

```text
allocation=alloc-v6-client-0
population index=37 target=10.96.0.2:18137 stage=READ errno=104
error=Connection reset by peer (os error 104)
```

Reaching index 37 is non-vacuous: the unchanged helper advances only after an
exact ACK and pushes the stream, so indices 0 through 36 had completed their
exact request/ACK exchange and were retained before index 37's ACK read reset.
The failure is neither connect refusal, request-write failure nor ACK-content
mismatch.

The in-process production trace was captured. Around the failure it records
the current client allocation's guest-originated `tcp_connect` flows first
ending as several `peer_eof` observations, then a burst of `peer_reset`
observations, followed by `control_session_lost`. It also records continued
successful peer-allocation HTTP/TCP probe dials around the same interval.
These observations establish the real production owner path and client
allocation but do not establish why index 37 reset or which end initiated the
first reset. No architecture or remedy is inferred from their order.

The one authorized receipt did not render the peer VM's private population
console, so it supplies no honest per-index peer `ACCEPT/REQUEST/ACK/CLOSE`
claim. The final all-console timeout rendering closes that diagnostic-source
omission for any later owner-authorized run; this handoff does not invent the
missing peer observation or launch a second reproduction to seek one.

The runner again recorded 101 successful libtest summaries totaling 111
passed, zero failed and zero ignored before the exact CLI body failed. The
five control-plane bodies were not launched, and no recognized runtime-SKIP
diagnostic occurred.

Read-only cleanup receipt `229b05df3d17597c7c5d1bfd` exited 0 at final
source digest `84b1033a...`, with no live Cloud Hypervisor process, nft table
or STEP 14-01 work directory. It listed only the documented pre-existing
`ovd-ns-0000`. No Lima or mutation run was used.

## Iteration 5 — capacity-accounted leg-B comparison

Independent review iteration 7 established that the earlier global
`N = C + 1` construction exhausted D's required concrete loopback binds
before the intended leg-B comparison. It did not falsify ADR-0190, D9,
V-LEGB-SERVE or roadmap approval. The attempted no-port change to D was
rejected and reverted; production registration and the original HTTP-prober
candidate remain unchanged.

The private comparison now derives a measured plan from the same 64-port
window:

```text
C = currently free concrete loopback ports
N = floor(C / 2) + 1
2 <= N <= 65
N < C
2N > C
```

The unset control first holds `N` raw option-unset sockets bound to
`127.0.0.1:0`, representing production D's concrete-port cost. With those
reservations live, it attempts `N` option-unset connections to the existing
distinct fixed listeners. It requires exactly `C - N` unique in-window source
ports, then the next connection must fail with `EADDRINUSE` or
`EADDRNOTAVAIL`, with no live failed row. Both reservation and connection
classes are closed/reset; `/proc/net/tcp` must show the reservation ports
absent, the control destinations must have no live row, and the measured free
capacity must return exactly to `C` before production starts.

The same `PopulationPlan { count: N, capacity: C }` is then passed explicitly
through the existing P6-S13 body to the existing client workload and
production leg-B assertion. Production must still complete all `N` exact
guest request/ACK exchanges and hold them. The external leg-B rows must equal
the exact `N` backend tuples, have one live fd each, read
`IP_BIND_ADDRESS_NO_PORT == 1`, and use sources inside the bounded window. If
`S` is their distinct source count, the source now requires:

```text
S < N
N + S <= C
2N > C
```

Thus the production cohort succeeds through leg-B sharing while an unset
leg-B cohort with the same concrete D cost cannot fit. All post-close
population absence, original-range restoration, mTLS/no-uplink, probe-state,
public stop and cleanup assertions remain unchanged. This changes only the
private measured population, not a product density promise or #326 benchmark.

### Source and exact execution evidence

| Artifact | SHA-256 |
| --- | --- |
| CLI private support | `59a3211b55f920fc25bb1cbb7fc2d6129672cf8e7fe233030e216e665701b63b` |
| P6-S13 body with private plan flow | `f64f7beb8a25bbe34aec9b73d4b292926151514a4bf3224cdaaceffb8e95971d` |
| guest workload and diagnostics | `5a28e5aab223a38b4b1391130f48a21c8d28b46b89345994eae60f19c2a254ec` |
| production HTTP prober | `d9296f0a002de28defe9f73ba0854eafcb2e400af0ab018010e48709b7045c55` unchanged |

Scoped source gate `cb3d36129bb6cf7f62c6683c` exited 0. Exact existing
runner `ea80ce9b99a8cfb3d906ee37` used the identical synchronized source digest
`b2d214a3f653232497f811210c4dcc58965bff66f4bb6d656747ba49c5676338`
and exited 101. Its output SHA-256 is
`8942daa1c18fe6c07ee7123ea5335209438b3573545bc28a0e1c6f02afde7e29`.

The existing P6-S13 identity passed in 47.43 seconds. Because its body cannot
return before every assertion executes, that receipt non-vacuously proves:

- unset reservations and connections represented the same D cost and reached
  exact `C - N` exhaustion;
- both control socket classes disappeared and the same capacity `C` was
  restored;
- all `N` production guest exchanges were byte-exact and held;
- exact production leg-B backend/fd/option/window observations passed;
- `S < N`, `N + S <= C` and `2N > C` all held;
- leg-C/no-uplink, final remote-peer cardinalities, public stop, VMM
  disappearance, production-population absence and range restoration passed;
  and
- every earlier retained Job, DNS/name, mTLS, remote-inbound and
  `Pass -> newer Fail -> newer Pass` oracle remained green.

The run retained 102 successful libtest summaries totaling 112 passed, zero
failed and zero ignored: the 111 retained pre-extension bodies plus the one
CLI P6-S13 identity. It then entered the first unchanged control-plane
identity,
`the_forwarder_inventory_counts_probe_and_relay_watches_through_serve`, but
that separate disposable guest produced no EXEC reply within its 600-second
cell bound. The runner therefore failed closed before any of the five
control-plane bodies received pass credit. This is an honest runner/
infrastructure timeout after the corrected P6-S13 pass, not a 117-pass
receipt and not evidence of a new P6-S13 semantic failure. No retry was
launched beyond the one authorized exact run.

Read-only cleanup receipt `11fd70ae687507e276cf8713` exited 0 at the same
source digest, with no live Cloud Hypervisor process, nft table or STEP 14-01
work directory. It listed only documented pre-existing `ovd-ns-0000`. No
runtime-SKIP diagnostic, Lima execution, mutation run, DES event or commit was
introduced by DISTILL.

## Iteration 6 — control-plane wait-flowing diagnostics

Native `d3a9eb7b7450deb799193f18` had preserved 114 prior passes before the
third control-plane body failed its existing `wait_flowing` assertion: the
client allocation never printed `hold: flowing` and ended with observer
terminal EOF. Its later no-EXEC-reply timeout was secondary to that body
panic. The original receipt did not preserve both workload consoles or the
in-process owner stages before accepted VM-directory cleanup.

This iteration changes only the existing private
`guest_flow_through_serve.rs` fixture support:

- a test-local `ConsoleCapture` follows the established CLI support pattern,
  reading each existing allocation console while production owns it and
  retaining the latest bytes before cleanup removes the run directory;
- `console_of` still reads the live production path first and uses the
  retained bytes only after that path is empty;
- the unchanged `wait_flowing` predicate and 120-second deadline now render
  the requested client console plus every retained allocation console, which
  includes the known Service and client allocations; and
- the repository's existing tracing subscriber is initialized before
  `run_server`, at INFO plus control-plane/worker/dataplane DEBUG, so the
  already-staged private owner events appear in the existing test output.

No assertion, stimulus, allocation selection, deadline, production hook/API,
runner selector, lane, scenario or identity changed. The capture and trace
output are never consumed by a success predicate. Production owner source
remains the separately staged private diagnostic source at SHA-256
`6e79e166df9eda1f35b6be01a5a48b704868185d4c1f051f26adfa3a5d2cf021`.
The control-plane body/support file is
`8600a93f36600c88d2b90d827fa58598ae229d528e69e24550179804336f1380`.

### Source gate and one reproduction

Scoped Metal source gate `f244eae46ec860c1c71fad85` exited 0 with output
SHA-256
`26de759c4834447b828f4b1cb3842f2d7fe397f2c00eecf94982df4c0c110fb0`.
The source gate, one native reproduction and cleanup audit used synchronized
source digest
`cd7d76112dc454fbf4b73754d41326a4580c607be3d0571ce66ace59a63211a9`.

The unchanged existing GuestKit runner was executed once as
`4dbc4f792d41a57f393b4308`; it exited 101. That schedule did **not** reproduce
the earlier third-body `wait_flowing` failure. The fail-closed sequential
control-plane manifest advanced through that identity and the fourth identity
to the fifth existing body,
`a_vmm_exit_under_a_held_relay_stream_resets_the_client_through_serve`.
The fifth body's outer disposable guest then produced no EXEC reply within
its unchanged 600-second cell bound. Its console shows the Service listener
and probe-reset traffic, but there is no body-level panic or valid replacement
for the missing third-body evidence.

The durable output contains 102 successful libtest summaries totaling 112
passed, zero failed and zero ignored. Because the fifth outer EXEC timed out,
the runner never reached its final five-body reconciliation and this is not a
117-pass receipt. The run is recorded as a non-reproduction of the original
primary failure followed by a different runner/outer-guest timeout. No retry
was launched to seek another schedule, and no production cause or remedy is
inferred.

The diagnostic source remains ready to preserve both Service/client consoles
and owner stages if an independently authorized owner later reproduces the
original wait-flowing failure. This handoff does not claim that source
capability as executed evidence.

Read-only cleanup receipt `745c909c3d357acdfe60e273` exited 0 with no live
Cloud Hypervisor process, nft table or STEP 14-01 work directory. It listed
only the documented pre-existing `ovd-ns-0000`. No recognized runtime-SKIP,
Lima, mutation, DES or commit action was introduced by DISTILL.

## Iteration 7 — fifth-body phase diagnostics

Native receipt `4dbc4f792d41a57f393b4308` reached the fifth existing
control-plane identity,
`a_vmm_exit_under_a_held_relay_stream_resets_the_client_through_serve`, but
its outer disposable guest reached the unchanged 600-second EXEC bound
without returning. The retained output proves neither a body panic nor the
exact stage at which the body stopped. Static inspection shows that the
existing `wait_observation` deadline is checked around repeated observations,
while one observation can remain inside the owner snapshot lock or forwarder
inventory read. That is a diagnostic hypothesis only; it is not classified as
a product defect or used to prescribe a remedy.

The exact existing fifth body now emits output-only `step14_01.cp_phase`
events immediately before and after these existing stages:

- client `wait_flowing`;
- the observation that the relay is held;
- the actual Service VMM `SIGKILL` request and successful return;
- the existing client-terminal polling loop;
- the observation that the dead launch and its relay dial are gone; and
- the existing `Node::shutdown` call.

Every event carries the exact test identity plus the current Service and
client allocation identifiers. The VMM-kill pair also carries the actual VMM
PID. The helper is private to the existing integration module, and no event is
read by a predicate or assertion. The sequence of awaits, kill stimulus,
allocation selection, predicates, assertions, deadlines and shutdown remains
unchanged.

The crafter-owned companion diagnostics use the same existing tracing output
around the owner snapshot state lock, `HostGuestFlowIo` forwarder-state lock
and inventory call, kernel forwarder inventory ioctl typed result/errno, and
existing owner/`ServerHandle` shutdown stages. Those events do not change
lock scope, ioctl invocation, ordering, timeout, error mapping, API or
shutdown behavior. The existing tracing dependency was added to the private
forwarder adapter only so those events reach the already-installed test
subscriber.

### Exact combined source identity

| Source | SHA-256 |
| --- | --- |
| Existing control-plane body and private fixture support | `8a4bda90d5b700c22f24f3edb62dc71a31ce093ad93bc53f4f78264d99119795` |
| Owner snapshot diagnostics | `9eae807a7d2225435353d2ac2d80b89da0f1ac6e83f366f30d5da1ec772abd7a` |
| Host inventory diagnostics | `be7e03a2a5229d6cbd9d5427b40fc93484c58aaff43d0ad53d377a7301bf7b00` |
| Owner shutdown diagnostics | `4d8356d7c35b14a3c09c4cf6307334a75e6714f43c3936ef9a8ede6344eec9bd` |
| Kernel inventory typed-result diagnostics | `e5cb0c94fb42ad807713f2540546d5e5774ff3e8f648b01a0c712d588d6144d4` |
| Forwarder tracing dependency declaration | `e4b9e5a1bd7069bdaf61bfb8acd819374bfeddf88af2bf6c524618ba22c6f0cb` |
| Workspace lockfile fallout | `fdebdb7b566a7698ae6615e0e1fc5664c9550b44285db83c2a064a5e202d9834` |

### Source-gate receipts and execution boundary

All three source-only Metal receipts used synchronized source digest
`d68140dde187ee885c14880a15e71e92b1128b5fefff29ae377dca12d9c469bd`
and exited 0:

| Purpose | Run ID | Output SHA-256 |
| --- | --- | --- |
| Combined production diagnostic compile | `1df2bc8dc4f9768a40d59d26` | `d44135316b18ca018e446af86ffa139b8cf67d05c8f3354ad7d8b23d0cabb7a4` |
| Format, control-plane integration check and Clippy `-D warnings` | `adb0e7c4afe13418a0e773bf` | `9a66fee8277634969737aea8c2f27d2e4c393e048710fba8de583e2b99941ec3` |
| Forwarder Clippy `-D warnings` | `e3eb13387a680bbdacdbcd20` | `c202da562890c95e2aaa7d0e67f4cd1b9fef7bc75f411026ab5324b030e3ac90` |

The final two durable records are retained byte-for-byte under
`.context/netns-density-295/14-01/raw/<run-id>/`; their terminal records both
declare `kind=command`, `exit_code=0`, `cause=command_returned`. No acceptance
body, GuestKit VM, kernel mutation or cleanup path ran in these source gates,
and no runtime-SKIP or pass count is claimed. No new native reproduction has
been launched. The combined diagnostics await independent source review and
the orchestrator's authorization for the single coordinated existing-runner
reproduction.

### Independent source review and one coordinated reproduction

Independent DISTILL review iteration 10 approved both halves of the combined
diagnostics. It found the events output-only and preserved all existing
predicates, assertions, deadlines, stimuli, ordering, identities, APIs and
117-body accounting. The review granted no native pass credit and prescribed
no product remediation.

After that approval, the orchestrator authorized one exact existing GuestKit
runner reproduction. Durable run `af7d3395010cef474da2c000` used workspace
source digest
`6ffef783cab4058806440f04579a465d6676df8fd2f2546049e22e279e94de90`
and exited 101 by `command_returned`. Its executable-source file hashes equal
the exact iteration-10 identities above; the workspace digest also includes
the subsequently written independent review artifact.

This schedule did not reach any control-plane body. Before the activated CLI
identity, 101 successful libtest summaries declared 111 passed bodies, zero
failed and zero ignored. The activated existing CLI identity then entered but
failed its retained health-sequence oracle at
`vsock_walking_skeleton.rs:166` after the unchanged client Service workload
failed to emit `v6-client: state=unhealthy` within the existing 40-second
bound:

```text
client Service did not drive unhealthy:
allocation=alloc-v6-client-0
```

The output contains no recognized runtime-SKIP result. It contains zero
`step14_01.cp_phase` events, and none of the five control-plane identities has
a result line. It therefore supplies no fifth-body `wait_flowing`, VMM-kill,
observation, inventory-ioctl or shutdown-stage evidence. The earlier
third-body EOF did not recur because the fail-closed runner never entered the
control-plane manifest. No cause or remedy is inferred from this separate
precondition failure, and the one authorized reproduction was not retried.

The 712,816-byte native output has SHA-256
`ab00640c36b8ec1487ebef35f7bd8c482a706c544e7d4152594a580523ce6c73`;
its terminal record has SHA-256
`f23aaa2c9b40c2b9dd95cd32a0c04d51993921aabbe558a80663c266f6ad033f`.
The full record is retained under
`.context/netns-density-295/14-01/raw/af7d3395010cef474da2c000/`.

Read-only physical-host cleanup receipt `df620b468fcb43b6e4a94f61` used the
same workspace source digest and exited 0. It reports zero live Cloud
Hypervisor processes, no physical-host nft table and no STEP 14-01 work
directory; the only listed namespace is the documented pre-existing
`ovd-ns-0000`. Its output SHA-256 is
`8d052939fb8d08930ee30e28cd3f26876d23a6d87729ce95b54b9ce71ca46b97`.
No Lima, mutation, DES or commit action was introduced.

## Iteration 8 — isolated existing fifth-cell diagnostic boundary

The one full-runner reproduction `af7d3395010cef474da2c000` remains a real
failure: after the retained 111 passes, the activated CLI body entered and
failed because `alloc-v6-client-0` did not emit its existing
`v6-client: state=unhealthy` marker within the existing bound. It produced
zero control-plane phase events and no control-plane identity result. That
receipt is not retried-until-pass, reclassified or replaced.

The canonical runner exposes only its complete `cgroup-default` selection.
Its existing helper (then named `run_step14_01_binary`, now
`run_nested_vm_test_binary`) and STEP 14-01 manifests are
private, so no existing public invocation can start the control-plane
diagnostic after the unrelated CLI prefix. The diagnostic execution therefore
uses the isolated spike boundary required by `.claude/rules/spike.md` at
`spike-scratch/netns-density-295-cp-14-01/increment-a/`. Nothing under
`crates/` or the canonical runner is changed for this boundary.

The standalone crate has a one-way path dependency on the existing
`overdrive-testing` test-support crate. At build time it reads the current
`fwd_guest_suite.rs`, removes only that binary source's file-level docs and
crate attributes, and includes every item unchanged. A private appended
wrapper calls that existing nested-VM test-binary helper with:

- the existing `overdrive-control-plane` `integration` prepared binary;
- the existing registered slot 15;
- a one-element slice containing the existing fifth
  `STEP14_01_CONTROL_PLANE` entry; and
- the existing helper's ordinary outer GuestKit staging, nested prerequisites,
  security mounts, cgroup/nofile wrapper, exact libtest filter, 600-second
  cell bound, required one-pass summary, runtime-SKIP rejection and cleanup.

The existing fifth body, production `run_server`/public submission path,
phase markers, owner/inventory/ioctl/shutdown tracing, workload images and
both console captures are unchanged. The standalone crate adds no public or
canonical selector, environment group, CI lane, product hook, replacement
test logic or expected-value injection. Its result is diagnostic evidence
only and cannot contribute to the unchanged 117-body inventory.

### Isolated source identity and source gates

| Source | SHA-256 |
| --- | --- |
| Standalone manifest | `951508fa477bdbc061cacd22637829f621a5d99855b1e9ee6b60cd7a07abe5fb` |
| Standalone lockfile | `7559a9949ba69490d0073682c29deea69ecd18ded49987a15d1b1bb4ad339937` |
| Runner-source generator | `5745bca11c41b70eedd3d2283422e053d241da4b2fa81510954243715e19b1ad` |
| Diagnostic entrypoint | `9ae793c327f7b707a9a7870a0d90eb8016f7031463201a2ab1a41b726e1f42b1` |
| Boundary README and exact commands | `fa06896b14f9cb35592dc6e53a9e96a95fc5c4ab7dd8d684051976bf74f0abc5` |
| Included canonical runner | `45a735911e846c15baa8c0a2cc4e5ac8553548617c61de7af4a0142f37c1bbb9` |
| Existing fifth body and fixture support | `8a4bda90d5b700c22f24f3edb62dc71a31ce093ad93bc53f4f78264d99119795` |

Initial source gate `9431daa0352f84a9fce5b7ed` exited 101 because a
direct macro inclusion could not legally retain the binary source's inner
docs and crate attributes inside a module. It launched no test or VM. The
retained generator is the neutral compiler correction: it starts at the
first Rust item and appends only the private exact-fifth wrapper.

Compiler gate `496ace560d7f1928c945afc3` then exited 0. Final locked Clippy
gate `301fd2d5f02795b5091c5b9f` exited 0 with `-D warnings`, synchronized
workspace digest
`21cf0e148603d9d11e86094c02bb13a235b4e04b721504d4c27cea4966751e6f`
and output SHA-256
`bf8c58ad43e85113e178ea12e547087396519ded47b432d2bd6e66c8c1252833`.
Full raw records for every source attempt are retained both under
`.context/netns-density-295/14-01/raw/<run-id>/` and the standalone
`increment-a/evidence/raw/<run-id>/` directory. No native diagnostic has been
launched; the boundary awaits independent DISTILL approval first.

### Pre-execution compiler and workspace-root corrections

Independent review iteration 11 approved the isolated source/boundary and
preserved the zero-control-plane-credit result of `af7d…`. Two authorized
preparation attempts then failed before any diagnostic body or outer GuestKit
VM launched, so neither consumes or contributes native body evidence.

Run `b8eb7c5200dabe33d95c6ed0` exited 1 during `guest-kit-prepare` because a
subsequently added crafter-owned private mTLS diagnostic did not compile. The
exact source-only reproduction `8f3d422621e756caed17ddf4` exposed Rust E0282
at `overdrive-dataplane/src/mtls/mod.rs:472`: the cast from
`&storage as *const _` had no inferred pointee type. The original crafter
changed only that cast to name `libc::sockaddr_storage`; current `mod.rs` is
`e56a4a10fb6da5441dc0bac275801b7c6c9bad608ed953279179c5b8ee7fb011`.
Metal dataplane gate `fd3f392aa58852c2a24e48c3` then exited 0 at source
digest `d0a21a4af00271c77fc7d787fa94e85f00f0a055b11194e47fc668bb64a531f6`.
No diagnostic behavior, socket observation, error mapping or API changed.

Run `bafb505ab7a31300e5294938` used that same digest. It completed
`guest-kit-prepare`, then stopped before image construction could launch the
outer VMM: the included runner's private `workspace_root()` had expanded
`CARGO_MANIFEST_DIR` in the standalone crate and consequently sought the
unchanged BPF object below `spike-scratch/target` instead of the repository
root. It exited 101 with the exact missing path and emitted no test, phase or
runtime-SKIP result.

The isolated generator now requires one byte-identical occurrence of the
canonical private helper and replaces only that helper with a `PathBuf` from
`OVERDRIVE_DIAGNOSTIC_WORKSPACE_ROOT`. The build script derives that value by
canonicalizing `increment-a/../../..`, the exact root the original helper
derives from `crates/overdrive-testing`. No staged artifact path changes; this
neutralizes only the standalone compilation context.

The runtime no longer invokes Cargo as root. Final source gate
`e96d51565e124918953f6796` runs locked Clippy with `-D warnings`, builds the
standalone executable as the unprivileged Metal user, and exits 0 at workspace
digest `7e00d96bfe777cf85c9f2b3f4c1a19b4b3ae4f6c1a20fdd8980cbc8f1ef131a4`.
It records executable SHA-256
`1133301132fe4f6302f6e61261d51b5c932425eed7d9bf669131470f76cf13ff`.
The privileged runtime command will print the same digest and invoke that
binary directly after the ordinary `guest-kit-prepare`; it cannot leave
root-owned Cargo output in the isolated source tree.

Updated isolated source identities are:

| Source | SHA-256 |
| --- | --- |
| Runner-source generator | `f5e87ea63a8da7d625cfe47ff042a40ca802700806c5539857b34bb34daa2555` |
| Boundary README and exact commands | `fa3d37d6a4c756f9956cd9635f7c821355d39df4688aaa63ddb4ebfe2d1b5289` |
| Diagnostic entrypoint | `9ae793c327f7b707a9a7870a0d90eb8016f7031463201a2ab1a41b726e1f42b1` unchanged |
| Included canonical runner | `45a735911e846c15baa8c0a2cc4e5ac8553548617c61de7af4a0142f37c1bbb9` unchanged |
| Existing fifth body and fixture support | `8a4bda90d5b700c22f24f3edb62dc71a31ce093ad93bc53f4f78264d99119795` unchanged |

Full raw records for `b8eb…`, `8f3d…`, `fd3f…`, `bafb…` and `e96d…` are
retained under both evidence roots. The updated neutral boundary requires the
same reviewer to approve it before the one authorized existing body executes.

Independent review iteration 12 approved that workspace-root correction and
kept both preparation failures at zero credit. The next authorized preparation
run, `037cb50421d19352ac4609ff`, completed the ordinary
`guest-kit-prepare` but then found that the subsequent workspace rsync had
removed the unprivileged binary from the noncanonical `target-codex-14-01`
directory. It exited 1 at the pre-execution SHA check. No standalone binary,
GuestKit VM or test body ran.

The source gate now installs the already-reviewed binary outside the
rsync-managed workspace under the Metal user's cache, in a directory named by
the exact executable SHA-256. The privileged runtime requires that same digest
before `exec`. It still runs no Cargo build as root and introduces no test,
selector, payload or runtime behavior. Locked Clippy/build/install gate
`be330805a138397f3149d764` exited 0 at workspace digest
`99a0e6e6248760d8acefbf5fdedba728692f0bdcdcb278567acb0d7dc41dbc17`;
the exact binary remains
`1133301132fe4f6302f6e61261d51b5c932425eed7d9bf669131470f76cf13ff`.
The updated README SHA-256 is
`b6eaeb65559030b080256df5738aead4ec2ca68dfe61d3df6c9eb78599fb1740`.
Both new raw records are retained under both evidence roots. This artifact
staging correction requires the same reviewer before body execution.

### One isolated existing-body execution — decisive terminal mismatch

Independent review iteration 13 approved the digest-addressed artifact
staging. Exact isolated run `154337257e1bebd3531ce423` then executed the one
existing fifth control-plane identity under its ordinary GuestKit boundary.
It used workspace source digest
`39f8917bd798c9661895aa5a436f4e657fd5f0d70f93c89135a3e7859915cf91`
and the exact reviewed executable
`1133301132fe4f6302f6e61261d51b5c932425eed7d9bf669131470f76cf13ff`.
The outer runner exited 101 after its unchanged 600-second EXEC bound.

The body itself entered and supplied decisive ordered stage evidence within
its first second:

| Timestamp | Existing body stage | Result |
| --- | --- | --- |
| `16:16:26.084771` | `client_flowing` start | exact Service/client allocations recorded |
| `16:16:26.088956` | `client_flowing` complete | existing `hold: flowing` predicate passed |
| `16:16:26.092291` | `observe_relay_held` start | observation began |
| `16:16:26.114906` | `observe_relay_held` complete | existing relay-held predicate passed |
| `16:16:26.122446` | `vmm_kill` request | Service VMM PID 248 |
| `16:16:26.126014` | `vmm_kill` complete | real `SIGKILL` returned success |
| `16:16:26.126876` | `client_terminal` start | existing console polling began |
| `16:16:26.333688` | `client_terminal` complete | a terminal line was parsed |

The relay-held observation disproves an owner-snapshot or inventory-ioctl
stall on this schedule. It acquired the owner state lock and host forwarder
lock, completed the kernel inventory ioctl with
`Ok(ForwarderInventory { live_pairs: 1, live_watches: 1, queued_ends: 0,
attached: 6, completed: 2, aborted: 6, unclassified_eof: 0,
dgram_oversize: 0 })`, and returned the owner snapshot before the VMM kill.

After the kill, production recorded the Service VMM reaped by signal 9 and
the Service guest-flow control session lost. The private transport-death
snapshot recorded:

- Service connection `alloc-d17-relay-svc-0#1`: terminal role
  `inbound_guest_relay_leg_c_to_s`; leg C remained TCP state 1 with zero
  retransmits and the connection was reclaimed.
- Client connection `alloc-d17-relay-client-0#0`: auxiliary terminal role
  `outbound_return_leg_b_to_f`; leg F remained TCP state 1, while leg B had no
  connected peer (`ENOTCONN`, errno 107), TCP state 7 and one total
  retransmission before reclamation.
- The production owner then recorded the client allocation's guest-originated
  flow end as `cause="peer_reset"`.

The unchanged client-console parser completed 207 milliseconds after the kill
and returned `TerminalEvent::Eof`. The existing assertion then failed
immediately and explicitly:

```text
assertion `left == right` failed: the mesh client sees a reset, never EOF
  left: Eof
 right: Reset
```

`terminal_event_in_console` can return `Eof` only after reading the exact
existing `observer guest-app: terminal=eof` console line. Thus the body-level
outcome is non-vacuous even though the complete client and Service console
files were not rendered into the outer guest's final 16-KiB tail. No
`observe_dead_launch` or shutdown marker follows: the assertion stopped the
body first. The later outer no-EXEC-reply timeout is secondary to that captured
body panic, not evidence of a snapshot, inventory or VMM-kill stall. This run
does not establish why the reset-classified owner path reached the guest
application as EOF and prescribes no production remedy.

No libtest pass summary or recognized runtime-SKIP result exists, so the
isolated body receives zero pass credit and cannot alter the full-runner
requirement of 117. Its 21,804-byte output has SHA-256
`21a1155d882faa4a7e1724ad7a5e8885c1386fed05ecab38a9b9f2ab5a7a8e1e`;
its terminal record has SHA-256
`2bcdc424b78a2b11c87ed317c3ca2ed6490e80ade7ae7188055c566aaffb1a39`.
The full raw record is retained under both evidence roots.

Read-only cleanup receipt `407aefabec02418e0234bea6` exited 0. It found
zero live Cloud Hypervisor processes, no physical-host nft table and no
STEP 14-01 work directory; only the documented pre-existing `ovd-ns-0000`
remained. Its output SHA-256 is
`d3d97964338664b99d620c518861ba71547a397523d069727cc95f0bfca14411`.
No retry, Lima, mutation, DES, commit or production remediation was performed.

Independent DISTILL review iteration 14 approved this execution evidence and
confirmed the semantic RED: the client observed EOF while Reset remains the
required outcome. It preserves zero isolated/full-runner pass credit and
prescribes neither a production remedy nor oracle weakening.

## Iteration 9 — panic-safe correlated allocation-console capture

Subsequent isolated receipt `cb98290768fcb273c3bd4525` retained the same
semantic RED and added Service guest-owner traces, but the assertion-panic
path still rendered only the Service allocation's final console. It showed
`ForwardId(1)` ending as `Aborted { cause: PeerAbort { side: A } }`, the
guest handle consuming that end, and deferred close with
`clear_linger=false`. The client allocation's guest-owner Abort/ForwardEnd,
close/socket trace and exact terminal line were absent. Cleanup receipt
`5561c7b2e0bd3f01922b9f020` found no VMM, nft table or STEP 14-01 workdir.
That asymmetric capture cannot establish that the Service `ForwardId(1)` is
the client relay rather than a probe and cannot classify the client-side
close path.

The existing private `ConsoleCapture` now covers every failure path rather
than adding another body-specific assertion message:

- it starts before either workload is submitted, discovers each exact
  allocation directory, opens its existing `console.log` once and retains the
  file descriptor until capture teardown;
- it incrementally reads every appended byte through that descriptor, so
  accepted `VmRunDir` unlink cannot hide bytes already written or the final
  bytes written through the still-open file;
- it retains the complete byte stream under the exact allocation identifier;
- it mirrors only complete guest application/owner control-and-end lines to
  the existing test output as they arrive, prefixed with the exact allocation
  identifier, so an outer timeout preserves both allocations without flooding
  the final console tail with kernel boot output; and
- on any unwinding assertion panic, its existing guard stops and joins the
  watcher, performs one final descriptor drain, then renders every retained
  control/end line plus a two-KiB final-byte tail for every exact allocation.

The streamed and panic output is never read by a predicate or assertion. The
matching fragments are limited to existing `overdrive-init: guest-flow`,
`observer guest-app:`, `listen:` and `hold:` lines; all matched lines are
retained, including probe and relay flows, so later classification must still
use the exact allocation, ForwardId and `hello=yes|no` evidence and cannot
credit a probe flow as the held relay. `console_of` remains live-path-first and
uses the complete retained byte stream only after that path is empty.

No test body, Reset-vs-EOF assertion, predicate, deadline, stimulus, identity,
runner manifest, lane, selector, public API, production hook or socket owner
changed. The correction does not keep a production TCP socket alive; its only
new descriptors are read-only handles to the already-existing console files.

### Source gates

The corrected control-plane integration source is
`c19a6351570d768174b297b70b7e5fd22a2bcfe91e1698fd78c31e454583abb8`.
Initial workspace-wide format/Clippy attempt `bce4777e8fd142b00b57206c`
exited 1: it exposed unrelated formatting and Clippy findings in the
crafter-owned mTLS diagnostic sources and did not execute a test or VM. No
production source was changed by DISTILL.

Scoped Metal gate `20085833257fec7f391c461f` used workspace digest
`a272c673938a6485c2bdd4ae9316d1e3ccf11b1f1d242d6db7bfba5bce586785`
and exited 0. It verified control-plane package formatting, checked the exact
integration binary with `integration-tests,kvm-tests`, and ran Clippy on that
package/test with `--no-deps -D warnings`; its output SHA-256 is
`2bc11d3856f36e910a5f1387eef238c3443a06680c6ec4b6fd61d884eebd9d01`.
No native body has run with the corrected capture. The existing approved
isolated fifth-cell launcher and GuestKit boundary remain unchanged, and the
one authorized matching-source reproduction awaits independent DISTILL source
review.

### Iteration 15 F-03 remediation

Review iteration 15 approved the capture lifecycle, allocation correlation,
read-only handle retention, final descriptor drain, live-first lookup and
unchanged behavioral boundary, but found one evidence-labeling defect. A final
unterminated console fragment could be emitted with the same prefix as a
complete control/end line, both from the teardown pending buffer and from
`String::lines()` over the complete retained bytes.

The private correction follows the prescribed boundary exactly:

- capture teardown performs the existing final descriptor drain and no longer
  emits the remaining unterminated pending fragment as a complete line;
- panic matched-line rendering scans only the retained byte prefix through the
  final newline, or an empty prefix when no newline exists; and
- the complete retained bytes and separately headed two-KiB raw tail remain
  unchanged, so any final partial fragment is preserved only as raw tail data.

No capture timing, file handle, allocation label, body, assertion, deadline,
stimulus, identity, runner, production code or Reset-vs-EOF outcome changed.
The corrected fixture source is
`fc49db5d5c0e2e70cdfe07565db9137faeb39a0acf82c81228854c8c3a1a4743`.
Scoped Metal gate `8f7649f923141f24a5503cd8` exited 0 at workspace digest
`0c249fa6c5f77e751a573f3eef8814f2f853fcf92520c93cbf15f7edef30cf6c`;
its output SHA-256 is
`db0a67d3e4a5f4d9b069f8429e1449e20921f18f9004f9fe5c5d372ac6c1f619`.
No native body ran. F-03 now awaits the same independent reviewer before the
one authorized isolated fifth-cell reproduction.

### Correlated Service/client reproduction

Independent review iteration 16 approved the F-03 correction. The one
authorized isolated fifth-cell reproduction then ran as
`6d39b863859629e23a31ec86`, using workspace source digest
`1c03cb00b3b176ccc6e61fe16d2dc380e569f5c97d6f9f8dd8644aeadce707db`
and the unchanged reviewed launcher ELF. It exited 101 after the existing
outer 600-second bound, secondary to the same captured Reset-vs-EOF assertion
panic.

This receipt retains complete, exact-allocation panic evidence from both
production-init workload VMs:

```text
--- retained allocation=alloc-d17-relay-client-0 bytes=28685 control/end lines ---
allocation=alloc-d17-relay-client-0 hold: flowing
allocation=alloc-d17-relay-client-0 observer guest-app: terminal=eof
allocation=alloc-d17-relay-client-0 overdrive-init: guest-flow forward-end id=ForwardId(1) outcome=Aborted { cause: PeerAbort { side: B } }
allocation=alloc-d17-relay-client-0 overdrive-init: guest-flow handle-end id=ForwardId(1) outcome=Aborted { cause: PeerAbort { side: B } } tcp_pairs=1 pending_end_closes=0
allocation=alloc-d17-relay-client-0 overdrive-init: guest-flow deferred-close stream id=ForwardId(1) clear_linger=false outer=GuestPairSocket(Handle { seam: SeamInstance(1), id: SocketId(3) }) vsock=GuestPairSocket(Handle { seam: SeamInstance(1), id: SocketId(2) })
```

The exact client allocation therefore correlates the one held guest flow's
successful application progress, application EOF, owner-side
`PeerAbort { side: B }`, matching `ForwardId(1)` handle-end, zero pending end
closes and deferred close with `clear_linger=false`. This is direct production
PID-1 owner evidence; `/sbin/overdrive-guest-peer` remains only the ordinary
`hold` application.

The Service allocation is also retained completely for its emitted
control/end lines:

```text
--- retained allocation=alloc-d17-relay-svc-0 bytes=29218 control/end lines ---
allocation=alloc-d17-relay-svc-0 listen: listening port=8080
allocation=alloc-d17-relay-svc-0 listen: conn=1 hello=no terminal=reset
allocation=alloc-d17-relay-svc-0 overdrive-init: guest-flow forward-end id=ForwardId(1) outcome=Aborted { cause: PeerAbort { side: A } }
allocation=alloc-d17-relay-svc-0 overdrive-init: guest-flow handle-end id=ForwardId(1) outcome=Aborted { cause: PeerAbort { side: A } } tcp_pairs=1 pending_end_closes=0
allocation=alloc-d17-relay-svc-0 overdrive-init: guest-flow deferred-close stream id=ForwardId(1) clear_linger=false ...
allocation=alloc-d17-relay-svc-0 ... listen: conn=2 hello=no terminal=reset
allocation=alloc-d17-relay-svc-0 overdrive-init: guest-flow handle-end id=ForwardId(2) outcome=Aborted { cause: PeerAbort { side: A } } tcp_pairs=1 pending_end_closes=0
allocation=alloc-d17-relay-svc-0 overdrive-init: guest-flow deferred-close stream id=ForwardId(2) clear_linger=false ...
```

The Service serial stream interleaves the second `forward-end` write with the
concurrent `listen: conn=2` write; the raw tail preserves the exact bytes. Both
application terminal records declare `hello=no`, so these Service connection
ends remain probe evidence. Neither Service ForwardId is credited as the held
`hello=yes` relay, whose VMM was killed before it could emit a Service
application terminal record. The client allocation's own `ForwardId(1)` is
the non-vacuous held-flow correlation.

The production host-side evidence remains consistent with the prior semantic
RED: the Service VMM PID 248 was killed, the client mTLS leg B was closed and
unconnected, and the owner recorded the client flow end as `peer_reset`; the
client production owner then consumed side-B `PeerAbort`, closed with
`clear_linger=false`, and the guest application observed EOF. This handoff
records that ordered evidence without selecting a production remedy or
weakening the required Reset outcome.

The panic guard rendered both allocation sections before accepted cleanup.
The later outer timeout supplied no pass result. The output contains no
recognized runtime-SKIP diagnostic and no libtest pass summary, so it earns
zero isolated or 117-body credit. The 21,862-byte output has SHA-256
`e3b525cfe98ab2c67f95534175276a6b9056dea8a232b4d1fe027e75b5066bc7`;
its terminal record has SHA-256
`b09c33784b17e3d8f42773655e07bbc82dedf4cca13d9201eb7bac4d7254aa9d`.

Read-only cleanup receipt `6c1c6640f48c80481c4ca359` used the same source
digest and exited 0 with no live Cloud Hypervisor, physical-host nft table or
STEP 14-01 workdir; only documented pre-existing `ovd-ns-0000` remained. Its
output SHA-256 is
`f85b5c911eae9411e67647c727f81af2445a912866c8d34f0d96b97bf25772f0`.
No retry, product edit, Lima, mutation, DES or commit action was performed by
DISTILL.

Independent DISTILL review iteration 17 approved the correlated evidence and
confirmed the semantic RED. The client allocation directly correlates EOF
with production PID-1 side-B `PeerAbort` and `clear_linger=false`; the Service
`hello=no` records remain probe-only evidence. Reset remains required, the run
receives zero pass or 117-body credit, and no remedy is inferred.

## Iteration 10 — completed-operation probe-watch sampling

Canonical run `c325287a325d309b8231305c` advanced through the retained 111
bodies and the CLI body, then failed the first control-plane U-D17 body with
`live_watches=1` and zero Paired owner dials. Independent review iteration 18
proved the observation landed inside a successful probe stream's synchronous
Drop: owner bookkeeping had been removed, while the inner
`HostGuestStream::drop` had not yet completed the kernel-watch abort. The
receipt did not show a mismatch after the public Drop returned.

The private `sample_agreement` helper now acquires each sample through the
existing `Node::wait_observation` and existing `SETTLE_BOUND`. Its completion
predicate requires a readable forwarder inventory and both exact equalities:

```text
inventory.live_pairs   == paired_guest_flows(observation)
inventory.live_watches == paired_dials(observation)
```

The returned observation still passes through the unchanged
`assert_inventory_agrees`, including the same exact pair assertion, watch
assertion and per-CID quota assertion. `most` is computed from that same
observation; the existing sampling windows and 100-ms sleep remain unchanged.
The predicate accepts a Paired held probe with one matching watch or a fully
dropped probe with neither, while any persistent mismatch still fails closed
at the existing settle deadline.

No count is weakened or approximated. No assertion, probe interval, deadline,
stimulus, body, identity, selector, runner, lane, API, production Drop/order or
117-body inventory changed.

Read-only existing-Sim receipt `36ae03f443918412eb24d345` independently
corroborates this ownership. Four existing drop/abort watch invariants pass,
including Quint seed `0x1127719f` and host seed `0xf7a`, with no
post-completion disagreement. This does not erase or reclassify the separate
historical baseline `quint_fwd_pair` seed failure.

The corrected control-plane integration source is
`6cbd1247a0986b11880fee5ed02fa16c404df066b4b313a9ad40ba4265ee6a81`.
The first gate attempt `a4710dd3de54ece8b7599575` exited 75 without
starting because the authorized Sim receipt held the canonical Metal lease.
After that lease completed, scoped gate `abc14bf1ae79f67d51e4723a` used source
digest `dfbd5734fe36d970f8de012563bab2795b4d43ae2571cf43a010b5f027387b57`
and exited 0. It verified control-plane formatting, exact integration-target
compilation with `integration-tests,kvm-tests`, and scoped Clippy with
`--no-deps -D warnings`; its output SHA-256 is
`df36c612634c894f1573d0fce0b128a5112c7191ee3493206af2cca14e6597d3`.
No native body ran with the correction. Independent source review precedes
the one authorized full canonical 117-body run.

### Iteration 19 canonical execution evidence

Independent review iteration 19 approved F-04 and authorized one full
canonical existing `cgroup-default` run. Run `8f6666cab490a60207221780`
used workspace source digest
`4fd0fbc034b0281868fc9545eb062f0c1de2fc1ec7da600a16fcd7b3a01dd4fa`
and exited 101.

The durable output contains 102 visible successful libtest summaries totaling
112 passed, zero failed and zero ignored: the retained 111-body boundary plus
the CLI body. It contains no recognized runtime-SKIP diagnostic. The
fail-closed runner then reached the third control-plane filter,
`the_forwarder_inventory_counts_probe_and_relay_watches_through_serve`; this
also proves the first two control-plane commands had returned through their
required exit/summary/no-SKIP checks, even though the unfinished outer guest
never reached `GuestRun::finish` to print their console records.

The third command did not reply to outer EXEC within the unchanged 600-second
bound. Its retained final 16-KiB console tail is inside the private panic
renderer and contains Service allocation probe records through connection 66,
all `hello=no terminal=reset`, with exact ForwardId handle-end and deferred
close records. Thus the body did unwind through an assertion/poll panic before
the outer timeout; the timeout is secondary rather than a completed 117
result.

The panic renderer iterates exact allocation identifiers in sorted order. Its
complete client section and the original panic text were printed first, then
displaced from the outer runner's fixed final tail by the much larger Service
probe section. The retained tail has no `hello=yes` Service record and no
client allocation bytes. That supports only the bounded observation that the
held client had not produced a retained Service application connection end;
it does not identify which existing client-flow poll/assertion panicked, and
it does not prove a new production cause. The original F-04 equality failure
is not present in the available tail, but its absence is not a body-pass
claim.

No third-control-plane pass or full 117 credit is granted, and the final two
control-plane identities were not entered. Per the one-run authorization, no
retry was launched to seek a different schedule or recover the displaced
panic text.

The 894,133-byte output has SHA-256
`43b9ded0ad0a2d7e4cabc4e6e1c5623f55f2d43ef7278166ac0e97271d269385`;
its terminal record has SHA-256
`361b31d098b31c6285aee3f4420c673bd40ace8ddaf4af2933a7ae5416d64c01`.
Read-only cleanup receipt `7daa6185c045b7dafaf1d00d` used the same source
digest and exited 0 with no live Cloud Hypervisor, physical-host nft table or
STEP 14-01 workdir; only the documented pre-existing `ovd-ns-0000` remained.
Its output SHA-256 is
`bb01f03188e0c91bbeba9da62f435ca6d547b9de7347206265621f8cd5883f12`.
No Lima, mutation, DES, commit, production change or second run was performed
by DISTILL.

## Iteration 11 — full failure-console retention and CP-prefix diagnostic

Iteration 19's canonical run established that F-04's corrected third body
still panicked, but the canonical runner's timeout error retained only the
outer GuestKit console's final 16 KiB. The exact panic text and earlier client
allocation section were displaced by the later, larger Service probe section.
That output limit prevents honest primary-cause classification and does not
establish a third-body pass or the complete 117 result.

The private canonical runner correction is confined to its two existing test
EXEC timeout branches. Both now attach `GuestKitVm::console()`—the same full
console getter used by existing successful `GuestRun::finish` reporting—to
the original timeout error instead of `console_tail(16 * 1024)`. Nonzero child
exit, missing-summary and runtime-SKIP errors already retain their full
per-command console and remain unchanged.

The original peer error, exit code, per-cell deadline and fail-closed return
are preserved. Failure still returns before `finish`, count reconciliation or
success reporting; no original error is swallowed. The success path and its
single full-console rendering remain byte-for-byte unchanged, so successful
pass summaries are not duplicated and 117 accounting is unaffected. Raw
outer-console bytes—including newline-terminated events and any explicitly raw
partial fragment—are reported without parsing, sorting or priority rules.

The corrected canonical runner is
`285a49b1c1b8b0f2dc62e7c0056aa84cbed315759cb7df07eafa0bb14db13962`.
Scoped Metal format/check/Clippy gate `ceae28d65a65493fff211881` used source
digest `59323d89ff4032c85735f10a38e11c86dea05ad0b85b8532623dd15134ebd9c0`
and exited 0; its output SHA-256 is
`c849de45d9404d657d2cb5f20754ad255eba28328bb271da70fdca055f68e694`.
No test or VM ran.

The focused reproduction uses the new isolated increment required by the
spike discipline:
`spike-scratch/netns-density-295-cp-14-01/increment-b/`. Its generated private
entrypoint invokes the corrected canonical runner's existing
`run_nested_vm_test_binary` with exactly `STEP14_01_CONTROL_PLANE[..3]`, retaining
the first two prerequisite cells and the third cell in canonical order. It
uses the same slot 15, prepared control-plane integration binary, outer
GuestKit, nested payload, mounts, security/device boundary, exact libtest
filters, 600-second per-cell deadline, summary/SKIP checks and cleanup as the
canonical run. It adds no public selector, lane, test, body, expected value or
product mechanism and cannot contribute to 117 credit.

| Increment-B source | SHA-256 |
| --- | --- |
| Manifest | `bb2b7bbd63b0cfd9c156bf9c663f967142858adfbcf0936588d99111b1631332` |
| Lockfile | `f9bd75adb6c297d87558dee6ba709b4961e92f5a5106fa2069eb213f6049f600` |
| Runner generator/private prefix entrypoint | `5764df439e1d42b69795f84702e5b8359a70f2186a53e1154236df3fda80ea03` |
| Main | `0a6d5aa5b26a60b5f3be2acec6b969aacc0298090f8a407e2565fd6ce8850822` |
| Boundary and exact commands | `4bcc8c31f3ef94cdaafe06372f506794ac84b38557aed71a30f7c164425987c0` |

Compiler gate `1291d1d703631b838dcc7397` exited 0. Final locked
Clippy/build/staging gate `19c52e22eeac39c03c48780b` used source digest
`02aa72bb9d1b6bbf4481549c181efd7247c3b91ff3688b09306ea34209102053`
and exited 0. It installed executable SHA-256
`3660d37060ee014216105a88fa48d0e80482b0f1ca5bf3b21ef8afefffbb4dc6`
outside the rsync-managed workspace under that exact digest. No focused native
execution has run; independent source/boundary review precedes the one
authorized attempt.

### Focused CP-prefix execution

Independent review iteration 20 approved the full failure-console correction
and increment-B boundary, then authorized one matching-source focused run.
Run `1e232061b80e4a961e094624` used workspace source digest
`c367378501a3c3d6a3f87c0b87c47eb88f172314a8d93844931aae15928745e4`,
verified executable SHA-256
`3660d37060ee014216105a88fa48d0e80482b0f1ca5bf3b21ef8afefffbb4dc6`,
and exited 0.

The exact canonical prefix ran in order with three separate successful
libtest summaries:

| Existing identity | Result | Duration |
| --- | --- | ---: |
| `the_forwarder_inventory_counts_every_held_guest_flow_through_serve` | 1 passed, 0 failed, 0 ignored | 9.25 s |
| `a_killed_launchs_attachment_leaves_the_owner_and_the_next_launch_is_accepted` | 1 passed, 0 failed, 0 ignored | 6.23 s |
| `the_forwarder_inventory_counts_probe_and_relay_watches_through_serve` | 1 passed, 0 failed, 0 ignored | 14.66 s |

No recognized runtime-SKIP diagnostic occurred. The outer GuestKit completed
its normal shutdown and full success-console path. Because no failure occurred,
this receipt does not execute the corrected full failure-console branch.

The focused pass does not convert the canonical `8f6666…` result into a pass.
The same three cells ran in one fresh outer control-plane GuestKit guest in
both cases, but the focused diagnostic omitted the retained-111 and CLI
execution prefix. Under that focused context the third cell passed; under the
full canonical sequence it panicked and its original panic/client output was
lost by the old 16-KiB reporter. The evidence therefore establishes a
sequence/schedule-sensitive difference, not that the canonical failure was
fixed or that its cause is known. No retry of the full sequence is authorized
or claimed.

This isolated three-pass receipt is diagnostic only and supplies no 117-body
credit. Its 252,899-byte output SHA-256 is
`c570ad1908b41d66b42716abb880f59225dcbd505fdaa8e53818c25693da20c7`;
its terminal record SHA-256 is
`70f81968d3212ba31a28e60a83f675fad184b2af725e1031c0204546e638d168`.
The full raw record is retained under both the ordinary evidence root and
increment-B's `evidence/raw/` directory.

Read-only cleanup `c5841add0a5378055dfea2e4` exited 0 with no live Cloud
Hypervisor, physical-host nft table or STEP 14-01 workdir; only documented
pre-existing `ovd-ns-0000` remained. Its workspace digest
`51634cc8aff6a729ab68e357f8045f003ab9e731a6f77d6be849cfe45b4bbf01`
differs only because the completed raw receipt had been copied into retained
increment-B evidence before the audit. Its output SHA-256 is
`4e6a0605225125df50785555f9780c36a781ae94bbed6287a31fbfc7406f50d4`.
No second focused execution, production change, Lima, mutation, DES or commit
was performed by DISTILL.

### Full canonical run with reviewed failure reporting

After review iteration 21 approved the focused evidence, the orchestrator
authorized one matching-source full canonical run specifically because the
new full-console failure path had not yet been exercised. Run
`727f0a30d20dc6ae67b3d3b1` used source digest
`1238131a47429722724604cf47e29c0a9560fcec62593c44d3cd7bb364e07a06`
and exited 101.

The durable output contains 102 visible successful libtest summaries totaling
112 passed, zero failed and zero ignored: retained 111 plus the CLI body. It
contains no recognized runtime-SKIP diagnostic. This schedule stopped in the
first control-plane identity,
`the_forwarder_inventory_counts_every_held_guest_flow_through_serve`; no
control-plane pass or complete 117 result is credited.

The corrected runner retained the complete 108,346-byte outer GuestKit
console inside the original timeout error. It now proves the primary body
failure that the prior 16-KiB report could not:

```text
thread '...the_forwarder_inventory_counts_every_held_guest_flow_through_serve' panicked at ...:568:9:
allocation alloc-d17-inv-0-0: the guest flow never reported flowing
```

The ordered production evidence for that exact allocation is:

| Timestamp | Event |
| --- | --- |
| `21:32:39.428483` | VM READY accepted |
| `21:32:39.434878` | allocation mTLS intercept installed |
| `21:32:39.437912` | guest EXEC released |
| `21:32:39.489926` | guest-originated `tcp_connect` entered the owner |
| `21:32:39.491219` | kernel inventory reported one live pair, zero watches |
| `21:32:39.510982` | shared guest-network owner entered bounded recovery, component `Forwarder` |
| `21:32:39.522438` | the exact allocation's `tcp_connect` ended with cause `quiesced` |

The complete client console contains no `hold: flowing`. It contains
`observer guest-app: terminal=reset`, followed by production PID-1 owner
`ForwardId(1)` ending as `PeerAbort { side: B }`, matching handle-end with
`pending_end_closes=0`, and deferred close with `clear_linger=false`. Thus the
first CP body failed because its real guest flow reset during owner quiescence
after the Forwarder entered recovery. The evidence does not identify what
caused that recovery and does not authorize a production remedy.

The focused prefix's three passes remain valid only for that focused schedule.
They do not classify this full-sequence failure as fixed: the same first body
passes without the retained-111/CLI prefix and fails after it. No second full
run was launched to seek a passing schedule.

The earlier separate `af7d3395010cef474da2c000` CLI health-sequence failure
also remains unresolved. Its retained lifecycle evidence rules out a missing
client listener, startup probe and intentional stop: the client declared and
bound listener 18085, had no readiness/startup probe, reached `paths-ready`,
then failed its unhealthy-state send while the peer saw `ECONNRESET`; client
app exit, PID-1 EXIT/poweroff and owner quiesce followed. The generic
`PumpExit::TransportDeath` warning text mentions ETIMEDOUT for the whole exit
class and is not proof of the actual host-socket errno. This handoff therefore
does not reclassify that earlier failure as expected health withdrawal or a
proven timeout.

The canonical output is 1,035,654 bytes with SHA-256
`fc0b0cc1a8bd68a50580b5c0415ec0198527b997e1076b1bb8001c48d2c04c3c`;
its terminal record SHA-256 is
`c775388d43e4c808ad33f30081519af6523f2116a17325cc8e89713a140d4c4e`.
Read-only cleanup `476535f2cbe3a342e9224e55` used the same source digest
and exited 0 with no live Cloud Hypervisor, physical-host nft table or STEP
14-01 workdir; only pre-existing `ovd-ns-0000` remained. Its output SHA-256 is
`975058adf20ba1878cd3df10331d084995b39acee42b627c532156a374db0354`.

No retry, production edit, oracle/deadline/identity change, Lima, mutation,
DES or commit was performed by DISTILL.

## Iteration 12 — direct canonical exact-body selection

The earlier increment-A and increment-B launchers generated private copies of
the canonical runner solely to reach existing STEP 14-01 bodies without the
full cgroup-default prefix. The user explicitly rejected that workaround and
authorized direct filtering in the test-support runner.

The canonical `overdrive-fwd-guest-suite` now accepts the test-support-only
environment variable `OVERDRIVE_FWD_GUEST_EXACT` when
`OVERDRIVE_FWD_GUEST_GROUP=cgroup-default`. Its value must be valid UTF-8 and
must equal one exact existing identity from `STEP14_01_CLI` or
`STEP14_01_CONTROL_PLANE`; an empty, partial or unknown identity returns the
existing `GuestKitError::Staging` failure with a zero-match diagnostic.
Supplying the exact selector with any other group fails closed before group
dispatch.

A matched identity runs through the existing `run_nested_vm_test_binary` helper:

- the CLI identity keeps its existing prepared binary and registered slot 14;
- a control-plane identity keeps its existing prepared binary and registered
  slot 15; and
- both retain the ordinary outer GuestKit, complete nested payload, mounts,
  device/security boundary, nofile wrapper, exact libtest invocation,
  600-second bound, summary check, runtime-SKIP rejection and cleanup.

The focused path requires the selected body's exact existing expected count,
then emits one explicit result with `canonical_117_credit=0`. It does not run
or reconcile the retained 111, CLI-plus-five extension, or final 117 count.
With `OVERDRIVE_FWD_GUEST_EXACT` absent, `run_cgroup_default` enters the
original canonical body unchanged; every manifest, count assertion, order,
oracle and success report remains intact.

No production API, lane, scenario, test body, expected-value seam, generated
runner or copied runner source is involved. The historical increment-A/B
sources and raw receipts remain retained and their READMEs now mark them as
superseded evidence launchers; no earlier evidence was deleted or rewritten.

The corrected canonical runner source is
`75dac61241c3feaaaa2b4b4e165a710074de9eaef861e2d01bd179eed99d56e9`.
Final scoped Metal format/check/Clippy gate `f40221b865095b3427ad9be0`
used workspace digest
`4687074a55157fe9298d8eff7171bfeb563956f31e57b5ffe43093faf64d1b94`
and exited 0; output SHA-256 is
`f7d3db52e7e8291cbc66bf9f7b0a2f42a164e17c9fef9748081809bc3d9961e3`.
Earlier preparation attempts were source-only: `1002377…` did not start while
the crafter's authorized diagnostic audit held the Metal lease, `4b4064…`
reported neutral rustfmt layout, and `7a5127…` reported one documentation
backtick lint. No test or VM ran in those attempts.

The concurrently staged control-plane audit-outcome diagnostics were compiled
separately by their owner and distinguish timeout, typed error and
`PostconditionMismatch` without changing recovery or error mapping. A focused
execution will retain the exact combined source identity; no production
change was made by DISTILL.

Independent DISTILL review iteration 23 approved the direct selector and
authorized one exact focused execution of the first existing control-plane
identity. Run `b98b3333856892b4b3072f46` used combined workspace source
digest `b30d85377f4148d2e1d54485899183c4ef5f76fe4dbb64161b1cb3d569feb8d7`
and exited 0.

The output proves the canonical runner selected only
`the_forwarder_inventory_counts_every_held_guest_flow_through_serve`, ran it
through the ordinary outer GuestKit/nested workload boundary, and obtained
exactly one successful libtest summary: 1 passed, 0 failed, 0 ignored. No
recognized runtime-SKIP diagnostic occurred. The runner then emitted:

```text
STEP14-01-FOCUSED-RESULT identity=integration::guest_flow_through_serve::the_forwarder_inventory_counts_every_held_guest_flow_through_serve passed=1 canonical_117_credit=0
```

This validates direct filtering and its explicit zero-credit boundary. It
does not reclassify the matching body's full-sequence failure in
`727f0a30d20dc6ae67b3d3b1`: the exact body passes in a fresh focused GuestKit
and fails after the retained-111/CLI prefix. No full canonical retry was
performed.

The 72,366-byte focused output has SHA-256
`8fa25c9ab0a9ef72b3f84522c4ff8c936dadc6e6c0d09d5276d06ac3d3b70697`;
its terminal record has SHA-256
`f3a6b87a3eecdcd7d49b04ff9644ff2b86b6b39cfca850c3d18840faa71887e2`.
Read-only cleanup `dff177c589372cfb6c00a9df` used the same source digest
and exited 0 with no live Cloud Hypervisor, physical-host nft table or STEP
14-01 workdir; only the documented pre-existing `ovd-ns-0000` remained. Its
output SHA-256 is
`f0359a1509fdb798b5e6633052c693484d57552ee60398d48327d54df652c3a8`.
No retry, production edit, lane/API/test change, Lima, mutation, DES or commit
was performed by DISTILL.

## Iteration 13 — runner helper responsibility names

The user required removal of roadmap-step numbering from private helper names.
The runner's private implementation now names responsibilities directly:

| Previous private name | Current private name | Responsibility |
| --- | --- | --- |
| `step14_01_nested_support` | `nested_vm_test_support` | stage the outer-guest substrate and nested production-serve dependencies |
| `run_step14_01_binary` | `run_nested_vm_test_binary` | execute one prepared integration binary in a nested-VM-capable cgroup-default guest |
| `run_step14_01_focused_exact` | `run_focused_exact_nested_vm_body` | resolve and execute one exact existing nested-VM body with zero canonical credit |

All callers and current handoff references use the responsibility names.
Frozen manifest/count/deadline constants retain STEP 14-01 attribution because
they describe that accepted delivery inventory rather than a reusable
operation. Historical review text remains unchanged as a record of the source
names it reviewed at that time.

The increment-A/B evidence wrappers now reference
`run_nested_vm_test_binary`, so they do not depend on removed names. Their
READMEs explicitly mark the source-transforming launchers as historical,
superseded evidence; the canonical `OVERDRIVE_FWD_GUEST_EXACT` path is the
only documented route for new focused runs. No raw receipt or historical
source explanation was deleted.

The rename changes no branch, manifest, payload, slot, selector, exact filter,
deadline, assertion, count, output, cleanup or API. Current runner SHA-256 is
`6921a47007a208b9106727ab3b7302737c252b517f166874221626622db14a65`.
Scoped Metal format/check/Clippy gate `a239d7d9a06795bda7277c47` used source
digest `8f16c0d2ebe7b414db768c78b4e93615c6efe3e1100ee93fc27ab0dbf6f892f1`
and exited 0; its output SHA-256 is
`908bdac9ed2f3115395933247b132bec2cde390a59b620114e16181bfcbccc6d`.
No native execution was repeated for naming alone.

Independent DISTILL review iteration 24 approved the direct selector runtime
evidence: only the exact first CP identity ran, its one summary passed, no
runtime skip occurred and the marker granted zero canonical credit. The
focused pass does not alter the full-sequence `727f…` failure or establish
117.

Independent review iteration 25 approved the responsibility-name correction.
All private callers and current docs use the new names, frozen manifest/count
constants retain correct delivery attribution, and historical reviews/raw
evidence remain honest. No native replay was required for naming.
