# V-GW-1..V-GW-5 — gateway connect path, selection hook, and intent key

Verdict summary: **V-GW-2 HELD (membership + proxy only), V-GW-3 HELD, V-GW-4 HELD (budget needs a user decision),
V-GW-5 HELD, V-GW-1 PARTLY HELD.** No item was falsified. One design rationale (V-GW-1 control) is wrong and must change.

- Date: 2026-11-01. Tree synced: `549d78fa65ebe5a9259fd9adcf6c0d7b694cf7f8` (taken from the progress log
  `.context/pig-reconstruct/01-progress.md`, section "V-GW-1..5 contract-shaping spikes"; the SHA is not printed in the
  per-run logs; `git rev-parse HEAD` in the worktree also reads `549d78fa6...`). No file under `crates/` was touched.
- Substrate: native metal `ubuntu@151.115.99.251`; every log line prints `uname=7.0.0-38-generic` (the stock Ubuntu 7.0
  kernel, x86_64, `systemd-detect-virt` = `none` per the D4 findings file on the same host).
- Runs: `cargo xtask metal run` sessions run1 (setup failure, no receipts), run2 (long pass, multiple sub-passes),
  run3, run4 (cleanest, quoted below unless stated). Raw logs: `spike-scratch/pig-vgw/out/run{1,2,3,4}/` (gitignored).
  Probe sources: `spike-scratch/pig-vgw/` (aya Rust eBPF `bpf/src/main.rs`, loader `src/main.rs`, dialer `src/dial.rs`,
  scripts `env.sh`, `pre-deploy.sh`, `post-deploy.sh`, `pass2.sh`, `pass3.sh`, `post.sh`, `common.sh`).
- Fixture: the same D4 VM Service (`service-vm-e08`, VIP `10.96.0.2`, listeners 18080/18081, guest `100.95.0.2`) as in
  `findings-vm-service-map-programming.md`, with the same environment bridges (metal kernel/rootfs env, `+io` in the root
  cgroup `subtree_control`, `ulimit -n`, `umask 022`, guest-peer PID 1, tolerant guest server).
  `env.sh run healthy` is invoked as `cargo xtask metal run -- bash -c 'umask 022; ulimit -n 1048576; echo +io >
  /sys/fs/cgroup/cgroup.subtree_control 2>/dev/null; bash spike-scratch/pig-vgw/env.sh run healthy'` (run4 `envhold4.log`).
- Probe design: two throwaway `cgroup_sock_addr/connect4` programs (aya Rust, no C) attached to
  `/sys/fs/cgroup/overdrive.slice` beside the production hook (multi-attach):
  `vgw_cookie` (records `bpf_get_socket_cookie` per dialed port, counts connects per port and per `comm`) and `vgw_select`
  (the registered arm: an `INTENT` map keyed by socket cookie -> `SERVICE_MAP` outer lookup -> inner Maglev slot ->
  `BACKEND_MAP`; `healthy == 0` denies, otherwise rewrites `user_ip4/user_port` and records a `Receipt` keyed by cookie).
  Unregistered cookies return 1 unchanged. The dialer `vgwdial` joins serve's exact cgroup before creating sockets.
  The probe maps are pinned under `/sys/fs/bpf/pigvgw`; the production `SERVICE_MAP`/`BACKEND_MAP` are pinned there by
  `bpftool map pin` and read by the probe.

## Kernel citations (read in the vendored source, tag `v7.0`)

`/Users/marcus/conductor/workspaces/helios/wellington-v2/vendors/linux`, `git describe --tags` = `v7.0`.

- `net/core/sock.c:2109-2113 @ v7.0` is the `getsockopt(SO_COOKIE)` case: `case SO_COOKIE: lv = sizeof(u64); if (len < lv)
  return -EINVAL; v.val64 = sock_gen_cookie(sk); break;`.
- `net/core/filter.c:5151-5154 @ v7.0` is `BPF_CALL_1(bpf_get_socket_cookie_sock_addr, struct bpf_sock_addr_kern *, ctx)
  { return __sock_gen_cookie(ctx->sk); }`, the helper behind `bpf_get_socket_cookie` for `cgroup_sock_addr` programs.
- Both reach the same generator: `include/linux/sock_diag.h:36-45 @ v7.0` `sock_gen_cookie()` wraps `__sock_gen_cookie(sk)`
  (declared `:34`). The source shows both callers use one per-socket generator; the equality itself is shown by the
  real-kernel evidence below, not by reading the generator body, which this probe did not open.

## V-GW-5 — userspace `SO_COOKIE` equals the hook's `bpf_get_socket_cookie`

Verdict: **HELD.**

- Hypothesis: the `SO_COOKIE` read in userspace before `connect(2)` equals the cookie `bpf_get_socket_cookie` returns inside
  the `connect4` hook for the same socket.
- Prediction: `pre_connect == hook` for every socket, for plain sockets and for sockets prepared per the contract.
- Falsification: any pair differs.
- Command (`pre-deploy.sh`): `$DIAL cookie --n 10 --cgroup $SERVE_CG` and `$DIAL cookie --n 10 --mark-bind --cgroup $SERVE_CG`
  (`--mark-bind` = `SO_MARK=1`, bind `127.0.0.1`, `IP_BIND_ADDRESS_NO_PORT`).

Evidence (run4 `pre-deploy.log`, kernel `7.0.0-38-generic`):

```
=== V-GW-5 cookie equality, plain sockets, inside serve's cgroup [23:48:38.967668459] uname=7.0.0-38-generic
port=41000 pre_connect=0xe003 pre_again=0xe003 post_connect=0xe003 hook=0xe003 equal=true
port=41009 pre_connect=0xe00c pre_again=0xe00c post_connect=0xe00c hook=0xe00c equal=true
COOKIE_RESULT 10/10 equal
=== V-GW-5 cookie equality, prepared sockets (SO_MARK=1 + bind lo + NO_PORT), serve's cgroup [23:48:38.983930691] ...
port=41000 pre_connect=0x1020 pre_again=0x1020 post_connect=0x1020 hook=0x1020 equal=true
port=41009 pre_connect=0x1029 pre_again=0x1029 post_connect=0x1029 hook=0x1029 equal=true
COOKIE_RESULT 10/10 equal
```

(Ports 41001-41008 elided here; every line in the log reads `equal=true`.) The progress log records the same 10/10 + 10/10
result for the earlier runs. The dial lines in later phases also carry a consistent cookie: the `DIAL cookie=0xb051`
printed by the dialer is the `sk:b051` seen by `ss` for the same socket (run4 `post-deploy.log`).

Edge cases: the cookie is stable across repeated pre-connect reads (`pre_again`) and across `connect()` (`post_connect`),
and is unchanged by `SO_MARK`/`bind`. Not exercised: reuse of a pooled socket for a second `connect` (not a legal
sequence for the intent key), `fork`/`dup`, IPv6, UDP.

Design implication: the intent key of ADR-0232 (`SO_COOKIE` read before `connect`, registered, matched by the hook on the
cookie) is sound on 7.0 for sockets prepared per the contract.

Gate recommendation: **PASS.**

## V-GW-2 — the attached hook fires for `serve`'s own threads

Verdict: **HELD, with a stated limit: cgroup membership of serve's threads and a proxy process placed in serve's cgroup;
not a socket created by a serve thread.**

- Hypothesis: `serve`'s tokio worker threads live inside `/sys/fs/cgroup/overdrive.slice`, so the slice-attached hook fires
  for sockets they create (the comment at `crates/overdrive-dataplane/src/mtls/mod.rs:928-931` says otherwise).
- Prediction: serve's threads are in a cgroup under the slice; a connect from a process in serve's exact cgroup is seen by
  the slice-attached hook, one from outside the slice is not.
- Falsification: serve is outside the attach subtree, or the hook does not count the in-cgroup connect.
- Command (`pre-deploy.sh`): `cat /proc/$SP/cgroup` and the per-task cgroup/comm table; `$DIAL conn --port 45001 --cgroup
  $SERVE_CG` vs `$DIAL conn --port 45002 --cgroup $NEG_CG` (`/sys/fs/cgroup/pigvgw-neg`, outside the slice).

Evidence (run4 `pre-deploy.log`):

```
0::/overdrive.slice/control-plane.slice
threads by (cgroup, comm):
      1 0::/overdrive.slice/control-plane.slice overdrive
     18 0::/overdrive.slice/control-plane.slice tokio-rt-worker
...
conn 127.0.0.1:45001 -> Err(111)
conn 127.0.0.1:45002 -> Err(111)
SEEN[port 45001] = 1   (hook counted connects)
SEEN[port 45002] = None   (hook counted connects)
```

The production hooks are attached at the same path (`bpftool cgroup tree`): `/sys/fs/cgroup/overdrive.slice`:
`842 cgroup_inet4_connect multi cgroup_connect4`, `843 ... cgroup_sendmsg4`, `844 ... cgroup_recvmsg4`. The connect errors
`111` are expected (no listener on those ports); only the hook's per-port counter matters. `post.log` (run2, ~11 min of
serve activity): `comm=vgwdial connects=3918`; the counter is by `comm` and only `vgwdial` appears (see caveat 1). Run2
`cmd-2.log` (about 3 min): `comm=vgwdial connects=3403`.

Honest caveats:

1. **Membership and a proxy process, not a serve-originated socket.** All 19 threads of serve (1 main + 18
   `tokio-rt-worker`) are in `/overdrive.slice/control-plane.slice`, a child of the attach path, and the hook counted a
   probe process that was moved into that same cgroup but not one outside. No `AF_INET connect` originated by a serve
   thread was observed: `SEEN_COMM` lists only `vgwdial`, because serve made no outbound TCP connect during the window.
   Kernel semantics (a connect hook runs for any process in the cgroup subtree) make the inference sound, but it was not
   observed end to end for a serve-created socket.
2. The check was against the hooks' attach path of this tree, which a future change to cgroup layout would invalidate.

Design implication: ADR-0224 alternative B (a dedicated gateway cgroup) is not required for the hook to fire for the
gateway; the stale comment in `mtls/mod.rs:928-931` is wrong about the attach subtree and should be corrected when that
file is next touched.

Gate recommendation: **PASS**, with the membership-only caveat recorded in the ADR-0224 consequences. If the reviewers
want direct observation, it falls out of V-GW-1's DISTILL real-kernel test (the production gateway socket against the
production hook), which exists anyway.

## V-GW-3 — healthy byte honoured; stopped backend never yields a wrong-peer success

Verdict: **HELD.** Two observations the design must carry (healthy flaps; entries persist after stop).

- Hypothesis: the registered arm returns `NoBackend` for `healthy == 0`, `Selected` after readiness `pass`, and after
  `job stop` a still-present entry yields `Selected` then a connect reset.
- Prediction: sequence `NoBackend` -> `Selected` -> (after stop) `Selected` + `ECONNREFUSED`.
- Falsification: healthy byte ignored, or a stopped backend connects as a different peer.
- Command: `$DIAL poll --secs 170 --cgroup $SERVE_CG` (registered dial every ~0.5 s; prints on change) started before the
  deploy; then `overdrive job stop service-vm-e08` from `post-deploy.sh`. The poller never touches the maps.

Evidence (run4 `poll.log`; first column is epoch ms, `+N` ms since poller start; the deploy returned Stable at
`MARK stable_returned 1793576921572805056` ns, the stop at `MARK job_stopped 1793576926460676204` ns):

```
1793576919055 +     0ms connect=Err(1) NoBackend/no-outer-entry be=0.0.0.0:0/h0
1793576920512 +  1456ms connect=Err(1) NoBackend/healthy==0 be=100.95.0.2:18080/h0
1793576921567 +  2512ms connect=Ok(()) Selected be=100.95.0.2:18080/h1
1793576926551 +  7495ms connect=Err(111) Selected be=100.95.0.2:18080/h1
```

NoBackend (healthy==0) at 20.512 s to Selected at 21.567 s is 1.055 s, with the startup probe passing about the same
time (`startup probe[0] ... last_observed_at=1793576921442`, readiness `1793576921443` in `describe-running.txt`). After
the stop (`Stopped workload 'service-vm-e08'.`, 26.46 s) the next change line is `connect=Err(111) Selected ...h1` at
26.551 s, 91 ms after `job_stopped` was marked: the entry is still present with healthy 1, the hook selects it, and the
connect to the dead guest is refused. Final map after the 12 s wait (run4 `post-deploy.log`) still carries both entries,
`healthy` = 1: `key: 01 00 00 00  value: 02 00 5f 64 a0 46 01 00  01 00 00 00`.

Run2 `poll.log` shows the same ordering and the flap:

```
1793575989358 +     0ms connect=Err(1) NoBackend/no-outer-entry be=0.0.0.0:0/h0
1793575990814 +  1456ms connect=Err(1) NoBackend/healthy==0 be=100.95.0.2:18080/h0
1793575991870 +  2511ms connect=Ok(()) Selected be=100.95.0.2:18080/h1
1793575994840 +  5482ms connect=Err(1) NoBackend/healthy==0 be=100.95.0.2:18080/h0
1793575996851 +  7493ms connect=Ok(()) Selected be=100.95.0.2:18080/h1
1793575998714 +  9356ms connect=Err(111) Selected be=100.95.0.2:18080/h1
```

Run2 `poll2.log` (about 5 min after the stop, with the entry still in the map): `connect=Err(111) Selected
be=100.95.0.2:18080/h1`. Run3 shows `Selected` -> `NoBackend/healthy==0` at +7493 ms (flap) and a second dial with
`healthy: 0` in `post-deploy.log` just before the stop (`key 1 value=... 0x01 0x00 0x00 0x00 0x00 0x00`, i.e. healthy = 0
at stop time).

Edge cases and open observations:

1. **`healthy` flapped 1 -> 0 in 2 of 3 deploys (run2, run3; not run4), cause not established.** The readiness probe
   stayed `last=pass` in `describe`. Candidates (not tested): the guest example's tolerant workload and its probe
   connections; the readiness-sampling interval; a hydrator rewrite of the backend row. Do not read map `healthy` as a
   monotone readiness signal until the cause is known. This is also why `post-deploy.sh` re-asserts `healthy=1` with
   `bpftool map update` before the V-GW-1 dial groups (the poller never does). **Run4's V-GW-1 dials therefore ran on a
   forced healthy byte.** The V-GW-3 conclusions come from the poller only.
2. **Entries persist after stop.** SERVICE_MAP/BACKEND_MAP stayed populated with healthy 1 for at least ~12 s in run4,
   +5/+10/+23 s in the D4 probe, and ~5 min in run2 (`poll2.log`). The result is `Selected` + `ECONNREFUSED` (111), never
   a wrong-peer success, because the backend address `100.95.0.2` has no listener once the guest is gone. A guest address
   re-lease (D4: the next alloc got `100.95.0.3`) was not observed to alias a stale entry within these runs, but is the
   scenario that would turn a stale entry into a wrong-peer connect; the exact-peer mTLS check (ADR-0234) is the
   defence and was not exercised here.
3. `NoBackend/no-outer-entry` before the Service exists; `NoBackend/healthy==0` between map programming and readiness.
   The receipt `reason` values observed: 1 (no outer entry), 4 (healthy == 0); reasons 2 and 3 were not triggered.

Design implication for ADR-0242: the consuming-port outcome table holds: no outer entry or `healthy==0` is `NoBackend`
(503); `Selected` followed by a refused connect is the 502 branch. The gateway must not treat `Selected` as liveness, and
must treat `healthy` as potentially flapping.

Gate recommendation: **PASS**; record the flap and the persisted-after-stop entries as inputs to ADR-0242 and open a
separate investigation for the flap (not a gateway issue).

## V-GW-4 — chained `SERVICE_MAP` -> inner -> `BACKEND_MAP` lookup verifies; instruction budget grows

Verdict: **HELD** for verification. **The Tier-4 budget rule is not satisfied if the arm is folded into the existing hook;
needs a user decision.**

- Hypothesis: the hash-of-maps -> inner -> `BACKEND_MAP` chain verifies in a `cgroup_sock_addr` program and its verified
  instruction count stays within the Tier-4 policy.
- Prediction: the program loads; `verified_insns` stays within the ">5% over baseline" rule
  (`.claude/rules/testing.md` § Tier 4).
- Falsification: verifier rejection, or budget growth the policy does not authorize.
- Command: `$VGW $ELF vgw_select /sys/fs/cgroup/overdrive.slice $PIN` (aya `ProgramInfo::verified_instruction_count`).

Evidence (run4 `loader-select.log`, `pre-deploy.log`):

```
LOADED prog=vgw_select id=867 verified_insns=Some(246) jited_len=704 uname=7.0.0-38-generic
ATTACHED prog=vgw_select cgroup=/sys/fs/cgroup/overdrive.slice mode=Single
...
PROG id=842 name=cgroup_connect4 verified_insns=Some(28) jited=186
PROG id=843 name=cgroup_sendmsg4 verified_insns=Some(28) jited=186
PROG id=844 name=cgroup_recvmsg4 verified_insns=Some(37) jited=272
PROG id=859 name=vgw_cookie verified_insns=Some(193) jited=484
PROG id=867 name=vgw_select verified_insns=Some(246) jited=704
```

`bpftool cgroup tree` after load shows production `842 cgroup_connect4`, `859 vgw_cookie`, `867 vgw_select` all
`cgroup_inet4_connect multi` under the same slice: production plus two probe programs coexist (the attach flag is `multi`
in the listing; the loader reports `mode=Single` for aya's own attach call, so a design must state the attach mode
explicitly rather than rely on this).

Edge cases and caveats:

1. **246 vs 28.** The probe verifies in 246 instructions; the production `cgroup_connect4` verified 28 in the same run
   (the baseline file in `perf-baseline/main/verifier-budget/` was named in the progress log, not re-read for this
   artifact). 246/28 is about 8.8x, far above the ">5% over baseline" gate in `testing.md` Tier 4. The probe also includes
   a hash-of-maps inner lookup, a `Receipt` write, and the intent lookup that production code might not need to store.
2. `vgw_select` is far below the 1 M privileged ceiling, so the question is policy, not feasibility.
3. Not exercised: the real production hook body combined with this arm (the probe replaced it), a tail-call split, worst
   case verifier states.

Design implication: selection can stay inside the existing hook mechanically; the cost is a budget re-baseline. The
alternative is a second `cgroup_sock_addr/connect4` program beside `cgroup_connect4` (multi-attach as shown) so the
production program's count is untouched.

Gate recommendation: **PASS for feasibility; user decision required** between (a) fold into `cgroup_connect4` and
re-baseline `perf-baseline/main/verifier-budget/` with user authorization, or (b) a second program beside the existing
hook. This is a design choice that changes ADR-0224's text ("existing hook"); do not decide it in DELIVER.

## V-GW-1 — upstream path: selection -> marked loopback dial -> leg-C -> TLS server flight

Verdict: **PARTLY HELD.** Proven up to and including leg-C's TLS server flight with the allocation SVID. Not proven:
a client-SVID handshake or any byte delivered to the guest.

- Hypothesis: a socket prepared per the contract (mark `1`, loopback bind, cookie registered) and connected to the Service
  VIP gets a `Selected` receipt, the rewritten dial enters leg-C through table 100 and TPROXY, and a TLS handshake with
  the gateway's client SVID completes and carries bytes to the guest.
- Prediction: `connect` OK with a `Selected` receipt; the socket shows `fwmark:0x1` on the client side and an accepted
  socket on leg-C whose original destination is the guest; the TLS server flight arrives.
- Falsification: `EPERM` without a `NoBackend` receipt, `ENETUNREACH`, a reset at the output rule, a handshake failure,
  or no bytes at the guest.
- Commands (`post-deploy.sh`): `$DIAL dial --vip 10.96.0.2 --port 18080 --hold 4 --cgroup $SERVE_CG` (no TLS) with
  `ss -tanpe state established` taken at +1.5 s; `$DIAL dial ... --tls` (TLS 1.3 client with a verifier that records the
  server leaf); the four controls below.

Evidence, prepared dial without TLS (run4 `post-deploy.log`, `ss -tanpe`, kernel `7.0.0-38-generic`):

```
0  0  127.0.0.1:47956   100.95.0.2:18080 users:(("vgwdial",pid=3449358,fd=3)) ino:18815755 sk:b051 fwmark:0x1 cgroup:/overdrive.slice/control-plane.slice <->
0  0  100.95.0.2:18080   127.0.0.1:47956  users:(("overdrive",pid=3448842,fd=42)) ino:18812617 sk:b053 fwmark:0x2 cgroup:/overdrive.slice/control-plane.slice <->
DIAL cookie=0xb051 connect=Ok(()) receipt=Some(Receipt { verdict: 1, reason: 0, backend_id: 1, ip_host: 1683947522, port_host: 18080, healthy: 1 }) Selected
sock local=127.0.0.1:47956 peer=100.95.0.2:18080
```

`ip_host: 1683947522` = `0x645f0002` = `100.95.0.2`. The first line is the client (source `127.0.0.1`, `fwmark:0x1`);
the second is the socket accepted by `overdrive` (pid 3448842 = serve) on leg-C, whose local address is the rewritten
destination `100.95.0.2:18080` (the TPROXY original destination), `fwmark:0x2` (the agent's own mark). Route state at the
time: `32765: from all fwmark 0x1 lookup 100` and `local default dev lo proto static scope host` in table 100; nft
`prerouting`: `ip daddr . tcp dport @inbound_destinations meta mark set 0x00000001 tproxy to 127.0.0.1:35559 accept`;
the leg-C listener is `127.0.0.1:35559 users:(("overdrive",pid=3448842,fd=35))`.

Prepared dial with TLS 1.3 (run4):

```
DIAL cookie=0x102d connect=Ok(()) receipt=Some(Receipt { verdict: 1, ... backend_id: 1, ip_host: 1683947522, port_host: 18080, healthy: 1 }) Selected
sock local=127.0.0.1:48716 peer=100.95.0.2:18080
TLS server leaf received (497 bytes), 1 intermediates
TLS write_all: Ok(())
TLS read: Err(... "peer closed connection without sending TLS close_notify" ...)
TLS negotiated version=Some(TLSv1_3) suite=Some(TLS13_AES_256_GCM_SHA384)
```

and the serve log in the same window:

```
2026-11-01T23:48:46.251534Z  WARN shared mTLS enforcement refused the connection error=peer verification failed against the trust bundle: peer sent no certificates
2026-11-01T23:48:46.323566Z  WARN shared mTLS enforcement refused the connection error=peer verification failed against the trust bundle: peer sent no certificates
```

The server leaf (497 bytes, 496 in run2) plus one intermediate was dumped to `/tmp/vgw-leaf.der` / `/tmp/vgw-inter0.der` on
the metal host by the verifier in `src/dial.rs:166`. The claim that the leaf carries the allocation SVID
(`spiffe://overdrive.local/workload/service-vm-e08/alloc/alloc-service-vm-e08-0`, the SPIFFE ID in
`describe-running.txt` "Issued certificates") is from the spike agent's reading of that DER on the host. **The URI SAN is
not printed in any retained log**, so it is not independently reproducible from `spike-scratch/pig-vgw/out/`; the retained
facts are leaf length, intermediate count, TLS 1.3 / `TLS13_AES_256_GCM_SHA384`, and the server's refusal text. Re-dump
the SAN (`openssl x509 -inform der -in /tmp/vgw-leaf.der -noout -ext subjectAltName`) if the review needs it on file.

Controls (run4, all four after the prepared dial; `common.sh` host has a default route via `eno12399np0`,
`151.115.99.1`):

```
(a) no mark, no bind, TLS: DIAL cookie=0x102e connect=Ok(()) ... Selected
    sock local=151.115.99.251:34520 peer=100.95.0.2:18080
    TLS server leaf received (497 bytes), 1 intermediates ... TLS negotiated version=Some(TLSv1_3)
(b) mark only:            DIAL cookie=0x102f connect=Err(101) ... Selected     # ENETUNREACH
(c) bind only:            DIAL cookie=0x1030 connect=Err(22)  ... Selected     # EINVAL
(d) unregistered:         DIAL cookie=0x1031 connect=Err(111) receipt=None none
```

Edge cases and honest findings:

1. **Not proven: the client-SVID handshake and bytes at the guest.** The dialer has no client certificate. There is no
   in-repo way to mint a node-CA client SVID without running `ca_boot` against a copy of the node's redb, which the spike
   did not do. Both leg-C refusals (`peer sent no certificates`) show the server demanded a client certificate, i.e. the
   gateway client entry (ADR-0234 exact-peer gateway client mTLS) is the part still unproven on real hardware. "Carries
   bytes to the guest" is not established.
2. **The design statement "a socket without mark and loopback bind cannot reach leg-C" is false on a host with a default
   route.** Control (a), an unprepared socket connected to the registered VIP, still reached leg-C: after the hook rewrote
   the destination to `100.95.0.2:18080`, the kernel's route lookup (local source `151.115.99.251` via the default route)
   hit the nft `output` chain rule 23, `ip daddr . tcp dport @inbound_destinations meta mark != 0x2 meta mark set 0x1
   accept`, which sets the mark `1` itself, so the re-route goes to table 100 and into TPROXY. This is the ADR-0190
   premise (the production output rule already marks any dial to a registered inbound destination), not a gateway
   feature. Mark-only fails with `ENETUNREACH` (101) and bind-only with `EINVAL` (22) because the default-route source
   and the loopback source then disagree with the mark or the bind (the exact kernel reason was not traced; no `strace`
   or `pwru` was run).
3. Consequence for the preparation contract (ADR-0243): the three-step preparation (mark `1`, loopback bind, register)
   still stands as the way to make the dial *deterministic* and independent of the host's default route. The rationale
   must change from "otherwise the dial cannot reach leg-C" to "otherwise the source address and route depend on host
   routing state, and mark-only or bind-only fail". The *safety* argument (an unregistered socket never gets the new arm)
   is carried by the cookie registration, which control (d) shows (`receipt=None`, ordinary behaviour), not by the
   marking.
4. The prepared dial's `ss` table shows `fwmark:0x1` on the client socket exactly as predicted; control (a)'s `ss` was not
   captured, so the mark on that socket is inferred from the rule, not observed.
5. Run4's dial groups ran on a **forced `healthy=1`** (`forcehealthy()` in `post-deploy.sh`), see V-GW-3 caveat 1. Run2/3
   logs show `NoBackend/healthy==0` at the same dials when it was not forced (run3 `post-deploy.log` lines under
   "prepared dial + TLS": `connect=Err(1) ... NoBackend/healthy==0`), consistent with the hook honouring the byte.
6. Run1 produced no receipt (`connect=Err(111) none`; cause not investigated, treated as a failed setup pass); run2's later
   sub-passes (`dial-tls2.log`, `dial-tls3.log`) after `job stop`/`workload restart` returned `Err(111) Selected` against
   the stale entry (V-GW-3) and the new alloc `alloc-service-vm-e08-1` at `100.95.0.3` was not selected because
   `BACKEND_MAP` kept the old `100.95.0.2` (see run2 `cmd-4.log`/`cmd-5.log`: after 120 s the map still carried
   `100.95.0.2`). This is a second persisted-map observation: after a restart the backend row did not move to the new
   guest address within 2 min on this tree (cause not investigated). It is outside this slice's scope but affects any end-to-end gateway test that restarts a Service.

Design implication: the upstream path through the existing nft/route/TPROXY machinery to leg-C holds for VM backends
with the probe's `connect4` rewrite, and the server presents TLS 1.3 with the allocation's SVID chain. The gateway client
entry must still be shown (DISTILL real-kernel test): an SVID-bearing client against the same path, with bytes and the
exact-peer check.

Gate recommendation: **PARTLY PASS.** Carry "client-SVID handshake and bytes at guest" into DISTILL as a required real-kernel
scenario that exists regardless; correct the preparation-contract rationale in ADR-0243 and the feature-delta
(the "cannot reach leg-C" premise); no design decision depends on the unproven half, because ADR-0234 already pins the
client entry under the existing mTLS owner, so this is not a contract-shaping gap.

## Incidental and host state

- **serve fail-stopped twice** during the campaign on a Forwarder audit `postcondition mismatch` whose log line shows
  `expected == observed`. Unrelated to the gateway (the run recovered by restarting the session). The line is not in the
  retained `out/` logs (only the progress log records it), so it cannot be quoted here. Recommend a separate issue
  (user approval required per CLAUDE.md before `gh issue create`).
- **Host state left:** the `ovd_fwd` kernel module is loaded; the root cgroup `subtree_control` includes `+io`
  (runtime-only, not persistent); staged `target/` under `/home/ubuntu/overdrive`. The probe maps were removed by `post.sh`
  (`rm -rf $PIN`); the probe cgroups `pigvgw-sel` and `pigvgw-neg` were `rmdir`ed.
- The environment bridges needed to get the D4 Service to Running are listed in
  `findings-vm-service-map-programming.md`; none touches the code paths under test.

## Gate recommendations

| Item | Verdict | Gate |
|---|---|---|
| V-GW-1 | PARTLY HELD | Proceed; DISTILL carries the client-SVID handshake + bytes at guest; change the ADR-0243 rationale (the nft output rule already marks registered tuples on a host with a default route) |
| V-GW-2 | HELD (membership + proxy) | Proceed; ADR-0224 alternative B not needed; fix the stale comment at `mtls/mod.rs:928-931` |
| V-GW-3 | HELD | Proceed; ADR-0242 outcomes stand; record `healthy` flap and persisted-after-stop entries; open a separate flap investigation |
| V-GW-4 | HELD (verifies) | **User decision**: re-baseline the Tier-4 budget for a folded arm (28 -> 246 verified instructions) or add a second program beside `cgroup_connect4` |
| V-GW-5 | HELD | Proceed; ADR-0232 intent key stands |

Overall: **PASS with one user decision (V-GW-4) and one corrected rationale (V-GW-1).** No contract-shaping item is left
Open for DISTILL; the unproven V-GW-1 client handshake is a DISTILL real-kernel scenario against a contract (ADR-0234)
that this spike did not contradict.
