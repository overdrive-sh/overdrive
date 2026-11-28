# Spike: the responder's TCP/53 listener serves one query per connection

Feature: guest-public-dns-resolution (GH #382, ADR-0360). Probe: `spike-scratch/guest-public-dns-tcp-pipelining/`
(sources, scripts, captured evidence under `evidence/`). Throwaway; nothing under `crates/` was touched.

## Verdict

**Hypothesis: CONFIRMED, and the effect is worse than a delay: the lookup FAILS.**
glibc's stub resolver, when it uses TCP and asks for both families, writes the A and the AAAA query back to
back in one `writev` on one connection and expects two replies. The production responder reads one framed
query, answers it, and closes; glibc gets the A reply, then EOF where the AAAA reply should be, and
`getaddrinfo` returns failure after ~16-25 ms. No AAAA-only loss, no retry delay: the whole name fails to
resolve (`getent ahosts` exit status 2, no output). Two further facts that decide how often a guest hits it:

- glibc suppresses the AAAA query when the guest has no non-loopback IPv6 address (`AI_ADDRCONFIG`, which
  `getent ahosts` uses); a v4-only guest sends only the A query and TCP works (population control below).
- musl (measured) opens one TCP connection per query, never pipelines, and works against the responder.

## Setup

- Kernel `7.0.0-38-generic` (`uname -r`, native metal box); glibc `Ubuntu GLIBC 2.43-2ubuntu2.7` (`ldd --version`);
  musl: `x86_64-unknown-linux-musl` static binary built by rustc (std's `ToSocketAddrs` -> musl `getaddrinfo`, AF_UNSPEC).
- Repo SHA at run time `0192774e0` (probe files were uncommitted/staged in the synced tree; production crates unmodified).
- The REAL responder: `HostGuestDnsFactory::host().responder(..)` with gateway `100.95.0.1`, built as in
  `dns_forwarding_real_wire.rs`, in a probe binary (`src/main.rs`) inside `unshare --net --mount`. Its host
  resolver file (`/etc/resolv.conf`, bind-mounted per mount namespace) names the upstream `127.0.0.2`. Clients run in
  a nested mount namespace whose `/etc/resolv.conf` is `nameserver 100.95.0.1`.
- Upstream: `ScriptedDnsUpstream` (A -> 93.184.216.34, AAAA -> empty NOERROR) for the main phase; for the TC phase a
  probe-local upstream that truncates every UDP reply (TC=1, no answers) and serves many queries per TCP connection.
- Wire captured with `tcpdump -i lo` (pcaps committed); client syscalls with `strace -f`.
- Runs (full logs in `evidence/`): `r1` = v4-only netns, run id `076cb2e2943dc223f96bf7b5`;
  `r3-v6` = netns plus a dummy interface with `2001:db8:1::2/64`, run id `369f369c275c820b9994b262`.
  Discarded runs: `1f853c8be4b1913da03e2af6` (never started, my pipe killed it), `3828570a8259980a8dd4ca58`
  (script argument not applied, duplicate of r1), `023d64c8b315b1fbfdd77161` (v6 run into a dir that already held
  the previous run's output; repeated cleanly as r3-v6).

## Hypothesis / prediction / falsification

```
Hypothesis:        glibc over TCP writes A and AAAA back to back on one connection; the responder answers one and closes.
Predicted outcome: one TCP connection carrying two framed queries; one reply; connection closed; lookup degrades
                   (AAAA lost, delay, or failure). UDP control and A-only/AAAA-only TCP controls succeed.
Falsification:     both replies arrive, or glibc opens one connection per query.
```

## Evidence (glibc, `RES_OPTIONS=use-vc getent ahosts pipe-test.example.com`)

Population table, 3 repetitions each (`evidence/*/run.log`), wall time of the client, `getent` exit status:

| Case | Guest has global IPv6 addr | Queries glibc sends | Result |
|---|---|---|---|
| UDP, no options | no (r1) | A only (AI_ADDRCONFIG) | exit 0, 12-13 ms |
| UDP, no options | yes (r3-v6) | A + AAAA, two datagrams | exit 0, 11-13 ms |
| `use-vc`, `ahosts` | no | A only, one conn | exit 0, 15-25 ms |
| `use-vc`, `ahosts` | **yes** | **A + AAAA pipelined on one conn** | **exit 2 x3, 16-25 ms, no output** |
| `use-vc`, `ahostsv4` (A only) | yes | A only | exit 0, 22-34 ms |
| `use-vc`, `ahostsv6` (AAAA only) | yes | AAAA only | exit 0, 37-54 ms |
| `options use-vc` in resolv.conf | yes | A + AAAA pipelined | exit 2, 25 ms |
| `use-vc single-request` | yes | A + AAAA pipelined (option has no effect over TCP) | exit 2, 34 ms |
| UDP reply truncated (TC) by upstream, default options | yes | UDP A+AAAA, then TCP fallback | **exit 2 x3, 10-12 ms** |
| UDP reply truncated (TC) | no | UDP A, then TCP A | exit 0 |

The two single-family TCP controls succeed with the same responder, so the failure is the second queued query,
not TCP or the forwarding path.

glibc's syscalls (`evidence/r3-v6/strace-usevc-ahosts.txt`): one connection, one `writev` of two framed queries
(`\0'` = 39 bytes each), one reply read, then close:

```
connect(3, {sa_family=AF_INET, sin_port=htons(53), sin_addr=inet_addr("100.95.0.1")}, 16) = 0
writev(3, [{iov_base="\0'", iov_len=2}, {iov_base="\263\31\1\0\0\1...pipe-test\7example\3c"..., iov_len=39},
           {iov_base="\0'", iov_len=2}, {iov_base="\275\33\1\0\0\1...pipe-test\7example\3c"..., iov_len=39}], 4) = 82
read(3, "\0007", 2) = 2
close(3)      = 0
+++ exited with 2 +++
```

Wire (`evidence/r3-v6/usevc-ahosts-1.pcap`; client side, checksum noise removed): one 82-byte segment
(two framed queries, tcpdump flags the second as "prefix length(39) != length(80)"), one 57-byte reply for the
A query only, then FIN from the responder:

```
100.95.0.1.58994 > 100.95.0.1.53: Flags [P.], seq 1:83, ack 1, length 82 [prefix length(39) != length(80)]
127.0.0.1.58088 > 127.0.0.2.53: ... length 41 46305+ A? pipe-test.example.com. (39)      <- responder forwards ONE query
100.95.0.1.53 > 100.95.0.1.58994: Flags [P.], seq 1:58, ack 83, length 57 46305 q: A? ... 1/0/0 A 93.184.216.34
100.95.0.1.53 > 100.95.0.1.58994: Flags [F.], seq 58, ack 83, length 0                   <- closes; AAAA never answered
```

The scripted upstream saw exactly one TCP A query per such client run and no AAAA (`evidence/r3-v6/upstream-seen-scripted.txt`).
The same names resolve with UDP on the same guest address configuration (rows 1-2), so the loss is TCP-specific.

Population control for the v4-only guest (`evidence/r1`): `getent ahosts` under `use-vc` sends only
`A? pipe-test.example.com.` (no AAAA, `AI_ADDRCONFIG`), gets its reply, exit 0.

## musl

Measured (`evidence/*/strace-tc-musl.txt`, `tc-musl-*.pcap`; upstream truncating its UDP replies so musl falls back to
TCP): musl sends A and AAAA as two UDP datagrams (it ignores `AI_ADDRCONFIG` unless asked), receives two TC replies,
then opens **two separate TCP connections** (fds 4 and 5), writes one framed query on each, reads one reply from each
and closes each. It works against the responder in every run (6/6, `musl getaddrinfo(...) -> Ok([93.184.216.34:80])`,
1.5-3 ms). musl has no `use-vc` option; its only TCP use is the TC fallback. (musl 1.2.x as linked by Rust's musl target;
the version string was not captured.)

## Design implications

- **Who is affected:** guests whose resolver is glibc (Debian/Ubuntu/RHEL-family images, anything not musl/Alpine)
  AND which have a global IPv6 address or call `getaddrinfo` without `AI_ADDRCONFIG` (Python, Node, Go's cgo resolver
  and other callers that do not set it send both families), AND which use TCP. A guest uses TCP when (a) `options use-vc`
  / `RES_OPTIONS=use-vc` is set, or (b) a UDP reply arrives truncated (TC) and glibc retries over TCP. (b) needs no
  configuration: any upstream answer larger than the client's UDP size (512 B without EDNS0; glibc sends no EDNS0 by default) makes the
  forwarded UDP reply carry TC (the responder relays the upstream's TC; the probe confirmed it relays it), and the guest's
  lookup then fails entirely with the current listener.
- **Does the guest have IPv6 today?** Not established here; if guests are IPv4-only and callers keep `AI_ADDRCONFIG`
  (getent, curl, most libc consumers), the A-only TCP path works and the defect is latent. It becomes live the moment
  a guest has a global IPv6 address, or for any caller that always sends both families.
- **User-visible degradation:** name resolution fails (`EAI_AGAIN`/not found; `getent` exit 2), fast, with no
  retry delay, not a lost AAAA. Not measured: the exact `EAI_*` code, and the behaviour of Go's pure-Go resolver
  and systemd-resolved-style stubs.
- A per-connection read loop would fix glibc; musl would be unaffected (one query per connection stays valid).

Recommendation: **serve multiple queries per connection (RFC 7766) instead of keeping ADR-0360's one query per
connection**, since the current listener turns every glibc dual-family TCP lookup, including the TC fallback, into a hard failure.

## Re-run against the fix (73aa25657) — 2026-11-28

**Verdict for the fix: FIXED.** The gateway TCP listener (commit `73aa25657`, "serves pipelined queries until the
client closes") now answers both queries of a glibc dual-family TCP lookup on one connection, and every case that
failed before exits 0.

```
Hypothesis:        the per-connection read loop in 73aa25657 lets glibc's pipelined A+AAAA TCP exchange complete.
Predicted outcome: every previously failing case exits 0 (use-vc ahosts, use-vc via resolv.conf, use-vc single-request,
                   TC fallback with global IPv6); the wire shows one connection carrying two queries and two replies,
                   closed by the client.
Falsification:     any of those exits 2, or the responder still closes after the first reply.
```

Setup: same probe, unchanged `run.sh` / `inner.sh` / harness / musl client (the harness builds against the workspace
crates by path, so it linked the fixed listener). Same invocation as the original runs,
`cargo xtask metal run -- spike-scratch/guest-public-dns-tcp-pipelining/run.sh <out-subdir> <v4|v6>`.
Binary sha `73aa256572df6eb099f29ae707669d0c473f231f`; kernel `7.0.0-38-generic`; glibc `2.43-2ubuntu2.7`; musl client
`x86_64-unknown-linux-musl`.

| Run | Mode | Run id | Evidence |
|---|---|---|---|
| fix-v6 | netns plus dummy interface with global IPv6 `2001:db8:1::2/64` | `b6235aeb1956adc5fac904e8` | `evidence/r4-fix-20261128-v6/` |
| fix-v4 | IPv4-only netns | `aad2d5f9b005592c6843687b` | `evidence/r4-fix-20261128-v4/` |

Earlier evidence (`r1`, `r3-v6`) is untouched.

### Results (3 repetitions each, `getent` exit status, same table as above)

| Case | Global IPv6 | Before the fix | With the fix |
|---|---|---|---|
| UDP control, `ahosts` | yes | exit 0 | exit 0 x3, 10-13 ms |
| `use-vc`, `ahosts` | yes | **exit 2 x3** | **exit 0 x3, 39-49 ms** |
| `use-vc`, `ahostsv4` | yes | exit 0 | exit 0 x3 |
| `use-vc`, `ahostsv6` | yes | exit 0 | exit 0 x3 |
| `options use-vc` in resolv.conf | yes | exit 2 | exit 0, 45 ms |
| `use-vc single-request` | yes | exit 2 | exit 0, 49 ms |
| TC fallback, default options, `ahosts` | yes | **exit 2 x3** | **exit 0 x3, 11-14 ms** |
| musl client, TC fallback | yes | exit 0 | exit 0 x3 (`Ok([93.184.216.34:80])`) |
| every case above, IPv4-only (fix-v4) | no | exit 0 except `ahostsv6` | exit 0 except `ahostsv6` (unchanged) |

fix-v6: 21 of 21 timed cases exit 0. fix-v4: 18 exit 0 and 3 exit 2; the three are `use-vc ahostsv6` on a host with
no IPv6 address, which `getent` answers with exit 2 because there is no address of the requested family. The original
IPv4-only run `r1` has the identical three (`evidence/r1/run.log`), so this is not a regression.

The scripted upstream returns an A record and an empty NOERROR for AAAA, so `getent` prints one address. "Both
families answered" is therefore shown on the wire and in the strace below, not as two printed addresses.

### One connection, two queries, two replies

glibc syscalls under `use-vc`, `ahosts`, global IPv6 (`evidence/r4-fix-20261128-v6/strace-usevc-ahosts.txt`). Before
the fix the first reply was followed by `close(3)` and a failed lookup; now both replies are read:

```
connect(3, {sa_family=AF_INET, sin_port=htons(53), sin_addr=inet_addr("100.95.0.1")}, 16) = 0
writev(3, [{iov_base="\0'", iov_len=2}, {iov_base="\307\0\1\0\0\1\0\0\0\0\0\0\tpipe-test\7example\3c"..., iov_len=39}, {iov_base="\0'", iov_len=2}, {iov_base="\350\2\1\0\0\1\0\0\0\0\0\0\tpipe-test\7example\3c"..., iov_len=39}], 4) = 8
read(3, "\0007", 2) = 2
read(3, "\307\0\201\200\0\1\0\1\0\0\0\0\tpipe-test\7example\3c"..., 55) = 55
read(3, "\0'", 2) = 2
read(3, "\350\2\201\200\0\1\0\0\0\0\0\0\tpipe-test\7example\3c"..., 39) = 39
close(3)                                = 0
write(1, "93.184.216.34   STREAM pipe-test"..., 93) = 93
+++ exited with 0 +++
```

Wire for the TC-fallback case, client side of the TCP connection (`evidence/r4-fix-20261128-v6/tc-glibc-ahosts-1.pcap`,
checksum noise removed): UDP A and AAAA are both answered with TC (`4189| 0/0/0`, `10819| 0/0/0`), then one TCP
connection `:43854 -> :53`:

```
Flags [S]                                         connect
Flags [P.], seq 1:83, length 82                   both framed queries in one segment (A 4189, AAAA 10819)
:53 > :43854 [P.], seq 1:58,  length 57           4189  1/0/0 A 93.184.216.34
:53 > :43854 [P.], seq 58:99, length 41           10819 0/0/0
:43854 > :53 [F.], seq 83                         client closes after both replies
:53 > :43854 [F.], seq 99, ack 84                 responder closes in answer to the client FIN
```

The same shape holds for the `use-vc` case (`evidence/r4-fix-20261128-v6/usevc-ahosts-1.pcap`): one 82-byte client
segment, reply 1 (57 bytes, A), reply 2 (41 bytes, empty AAAA), then the client's FIN first. The responder closes only
after the client does. Upstream log for the scripted phase shows both `qtype=1` and `qtype=28` arriving over TCP for
each such client run (`upstream-seen-scripted.txt`); previously only the A query arrived.

### Caveats

- Same probe scope as before: glibc 2.43 `getent` and a musl `getaddrinfo` client against the responder with a scripted
  upstream. Not measured: the behaviour of other resolvers (Go, systemd-resolved stubs), query counts above two per
  connection, or an idle-timeout/connection-cap path.
- The responder answers the two pipelined queries one after the other (replies about 0.4-20 ms apart in the TC case,
  about 20 ms apart under `use-vc`, set by the scripted upstream's latency), so reply order matches query order here.
  Out-of-order replies are not exercised.
