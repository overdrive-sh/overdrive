# Debugging Discipline

Reasoning patterns for investigating failures the test tiers
(`.claude/rules/testing.md`) catch but do not explain. The four-tier
test stack is the gate; debugging is what happens when a test fails
and the gate did not predict where to look.

These rules are general-purpose. They apply equally to a flaky DST
seed, a kernel packet drop, a Raft election that fails to converge,
and a workflow journal that replays divergently. Tool-specific
recipes (e.g. `pwru` for kernel packet tracing) live at the end of
the file under § Real-kernel debugging.

The rules below were extracted from a single multi-day investigation
(S-2.2-17 length-N TCP drop) where every one of them was violated at
some point and the violation cost real time. Treat them as the
distillation of failure modes that *will* recur, not as abstract
advice.

---

## 1. Falsifications disprove interventions, not hypothesis classes

A test that disproves "applying fix X resolves the failure" is NOT
the same as a test that disproves "the failure mechanism is in
class C." Conflating them produces investigations where each round
discards a hypothesis class on the basis of an irrelevant
intervention failing.

**Symptoms during review:**

- Three or more rounds of "we tried X, it didn't work, so the bug
  isn't in that area" without any round actually testing the
  *recommended* fix for that area.
- A "falsification" annotation in research notes that names a
  finding (e.g. "the prologue revert was tried first") rather than
  an experiment (e.g. "fix Y was applied and the test still failed").
- The investigation cycle has length-of-fixes-tried but never tries
  the fix the original hypothesis actually proposed.

**Check:** before claiming a hypothesis class is dead, write down
*the specific test that would falsify it* — including the exact
intervention. If the prior test sequence didn't run that
intervention, the class is still live.

---

## 2. Error codes are taxonomy, not mechanism

`SKB_DROP_REASON_TC_EGRESS` does not mean "TC dropped this." It is
the kernel's generic egress-drop bucket; the kernel attributes
drops on a path to a *layer*, not a *cause*. The same shape applies
to `EINVAL` (`Os { code: 22 }`), HTTP 500, gRPC `INTERNAL`,
`process exited with status 1`, and `panic: index out of range`.
The code names *the layer that gave up*, not what went wrong.

**Failure mode:** reading a code as if it identifies the mechanism
sends the next probe in the wrong direction. "Reason 51 = TC ⇒
look at the TC program" is the silhouette.

**Check:** for any error code your investigation pivots on, find a
second source that confirms the mechanism — the kernel source line
that fires the code, the library function that constructs it, the
service that emits it. Confirm the *call site*, not the *name*. A
kfunc / kprobe at the call site is a stronger signal than the code's
symbolic name; reading the call-site source is stronger still.

---

## 3. Inspection-tool gaps look like negative evidence

`tc filter show` is legacy-only; TCX attachments are invisible to
it. `kretprobe:tcf_classify` doesn't fire when the kernel inlines
the call. `bpftool prog show` lists programs system-wide, but
attachment context is per-netns and requires `nsenter`. A probe
that returned "empty" may mean *the thing is absent* OR *the tool
can't see it on this kernel / runtime / scope*.

**Failure mode:** absence-of-evidence becomes evidence-of-absence.
The next round's hypothesis assumes the missing thing is missing
when in fact it was just unobserved.

**Check:** before "X is missing," ask:

- Would I see X with this tool, on this kernel/runtime, in this
  scope (netns, cgroup, container, namespace)?
- What's the canonical positive case — what does the same probe
  output when X *is* present? Run that case (a known-good fixture,
  a hand-attached toy, a separate test that exercises the same
  surface) to confirm the probe sees what you expect.

If both checks fail, the probe is uninformative regardless of what
it returned.

---

## 4. Predict the outcome before running the probe

A probe with a written prediction either confirms what you
expected (cheap to skip-ahead from) or contradicts it (expensive
to dismiss — forces re-modelling). A probe without a written
prediction silently calibrates to whatever it shows.

**Failure mode:** every probe "supports the current hypothesis"
because the hypothesis is updated post-hoc to match the result.
Confirmation bias dressed up as evidence.

**Check:** every diagnostic dispatch carries three lines:

```
Hypothesis:        <one sentence>
Predicted outcome: <specific values, ranges, or symbols>
Falsification:     <what observation would prove the hypothesis wrong>
```

If you cannot fill any of the three, the probe is *exploration*,
not *investigation*. Both are valid; label them honestly so the
reader knows what kind of evidence the result is.

---

## 5. Compare populations, not isolated failures

For any content-conditional, load-conditional, or shape-conditional
failure: capture both populations (failing and surviving), dump the
same metadata, diff. The diff is the diagnosis.

**Failure mode:** investigating "why does the failing case fail"
in isolation, when the answer is "the failing case differs from
the surviving case in field F" and one diff would have shown F.

**Check:** if your bug shape is "X happens for some inputs but not
others," your first probe captures *both* sides at the same
boundary. Same fields, same units, juxtaposed. The probe's first
job is the diff, not the failure.

The technique generalises:

- Failing test seed vs passing seed → diff the trajectory at the
  divergence step.
- Slow request vs fast request → diff the per-stage timing.
- Crashed pod vs healthy pod → diff the resource snapshot at the
  crash boundary, not the crash itself.

---

## 6. Refresh measurements when source changes

Measurements are derived from `(source × runtime × inputs)`. A
source change invalidates every prior measurement on the affected
path. "We measured this last week" is only valid if the source
hasn't shifted since.

**Failure mode:** a stale measurement becomes a load-bearing
premise. The investigation that follows is correct in form but
operates against a model that no longer matches reality. Sub-cases:

- A bpftrace drop-reason number captured pre-refactor, cited
  post-refactor.
- A latency baseline taken on the previous tag, treated as still
  valid after a perf-affecting change landed.
- A flaky-test seed that reproduced once, retained as "the
  reproduction" after the test body was rewritten.

**Check:** every diagnostic that cites a prior measurement names
the commit it was taken at. If the current HEAD differs on the
relevant path, re-run the measurement before relying on it.

This rule connects to `development.md` § "Persist inputs, not
derived state" — a measurement is a derived value of source, and
caching it across source changes is the same anti-pattern.

---

## 7. Probe at the right altitude for the question

| Question | Right altitude |
|---|---|
| Did the program run at all? | run-counter / hit-counter |
| Where in the path does this skb die? | per-skb tracer (`pwru`) |
| What's wrong with the skb at the drop site? | per-skb metadata dump at the kfree |
| Why does this RPC return 500? | server-side log + request ID (not retry counter) |
| Is consensus stuck? | leader log + per-replica state, not aggregate metrics |
| Why does this test flake? | seed + DST trajectory diff, not retry count |

**Failure mode:** wrong-altitude probe answers a different
question from the one you have. The result is informative but not
load-bearing — it tells you *something* fired but not *why*.

**Check:** before picking a probe, write the question. Match it
against the table above (or extend the table). If the probe
answers a different question, it's the wrong probe — even if it's
the easiest to run.

Probes also have a natural ordering: low-altitude (did it run?)
before mid-altitude (where did it die?) before high-altitude
(what was wrong with the data?). Skipping levels is allowed when a
prior probe has already established the lower-altitude fact;
reaching for high altitude on the first probe is exploration.

---

## 8. `let _` on fallible setup is a debt-bomb

Test fixtures that swallow errors with `let _ = …`,
`.unwrap_or_default()`, `.ok()`, or bare `;` on a `Result` create
silent environment drift: each test run may execute against a
different kernel / network / storage shape than the author thinks.
Root causes that depend on this shape survive years of
investigation because the actual environment is never queried.

This duplicates `development.md` § "Distinct failure modes get
distinct error variants" — extend the same discipline to test
setup. A fixture that *can* fail must either succeed (`?`) or
panic (`.expect("…")` with a message naming the precondition).
Bare `let _` on a fallible call is rejected at review time.

**Check:** during code review, grep changed test fixtures for
`let _ =`, `.unwrap_or_default()`, `.ok();`, `_ = `. Each
occurrence on a fallible call needs an explicit justification or
a rewrite.

The cost shape of this rule is asymmetric: enforcing it costs
seconds at review time; violating it can cost weeks of
investigation when the silently-degraded fixture becomes the
unstated premise of a debugging session.

---

## 9. Read symbols, don't reason about probable behavior

`__dev_queue_xmit+1276` resolves with `addr2line` / `faddr2line` /
`objdump` against the running kernel's debug symbols or vmlinux.
The resolved source line is evidence; "the probable kernel path
through that function" is a guess, even when the guess is informed.

The same holds for stack traces, panic backtraces, prog dumps,
disassembly. Resolve the symbol, read the source, then reason.

**Failure mode:** a chain of "probably …" inferences each shaves
1% off the next round's confidence; six rounds in, the
investigation is a stack of guesses about a function nobody read.

**Check:** when a trace cites `<symbol>+<offset>`, resolve the
offset to a source line *before* the next probe. If the source
isn't available (kernel without debug symbols, stripped binary,
external service without traceable artifacts), that's the work to
do — not a reason to keep guessing.

For Linux kernel symbols on the Lima VM:

```bash
cargo xtask lima run -- bash -c '
  apt-get install -y linux-image-$(uname -r)-dbgsym 2>/dev/null
  faddr2line /usr/lib/debug/boot/vmlinux-$(uname -r) __dev_queue_xmit+0x4fc
'
```

(`+1276` decimal is `+0x4fc` hex; both work depending on the tool.)

---

## 10. Each probe gets a hypothesis, prediction, and falsification path

Restating the form from § 4 explicitly because it's load-bearing:
every diagnostic dispatch (whether to a subagent, a teammate, or
yourself in 30 minutes) ships with:

- **Hypothesis** — what we believe is happening, in one sentence
- **Prediction** — what the probe will show if the hypothesis holds
- **Falsification** — what the probe will show if it doesn't

The triple is the unit of investigation. A probe without it
generates data that the investigator post-rationalises; a probe
with it generates evidence the investigator can trust.

The triple also provides a natural checkpoint for *escalating
invasiveness*: if rounds 1–3 (non-invasive probes) didn't disprove
the hypothesis class but didn't confirm it either, round 4 may
require source instrumentation, a code change, or a maintenance
window. The decision to escalate is defensible against the triple
("here's what the next probe must show") rather than against
momentum ("we're still stuck").

When dispatching to a subagent, the triple goes in the prompt
verbatim. When the agent reports back, its findings are scored
against the prediction and the falsification path — not against
the original hypothesis. This keeps the loop honest under
delegation.

---

## Real-kernel debugging — `pwru`

The pre-merge tiers gate regressions; they do not explain *why* a
packet died in the kernel when an integration test or manual repro
fails. For that, [`pwru`](https://github.com/cilium/pwru) (Cilium) is
the canonical inner-loop tool — it traces a single skb through every
kernel hook (XDP → TC → conntrack → routing → qdisc → kTLS) with
filter expressions, and gives the visibility the BPF ringbuf does not.

Installed in the Lima dev VM (`infra/lima/overdrive-dev.yaml`) as
`pwru` on the system PATH; `cargo xtask lima run --` runs as root, so
the standard wrapper invokes it without further escalation:

```bash
# In one shell — start the trace.
cargo xtask lima run -- pwru --filter-func 'kfree_skb*' \
  'host 10.0.0.5 and tcp port 8080'

# In another — reproduce the failure (run the integration test, send
# the packet via `tcpdump`-style tooling, etc.).
```

When to reach for it:

- A Tier 3 test fails with "packet did not arrive" and the BPF
  ringbuf shows no drop event — pwru identifies the kernel hook that
  consumed the skb.
- A new XDP / TC program ships and traffic seems to disappear; pwru
  confirms whether the verdict actually fired or the packet skipped
  the program entirely. Per § 3 above, the same answer cannot be
  derived from `tc filter show` alone — pwru sees TCX hooks, the
  legacy tooling does not.
- `bpf_l4_csum_replace` / `CHECKSUM_PARTIAL` length-N drops — exactly
  the failure shape that motivated the conntrack-INVALID falsification
  in Phase 2.16. pwru catches these pre-conntrack and points at the
  exact `kfree_skb_reason` site.
- BPF map lookup verification — `pwru --filter-track-bpf-helpers
  --output-bpfmap <pcap-filter>` (v1.0.11+) traces `bpf_map_lookup_elem`
  calls with the resolved map name, key bytes, and value bytes. Answers
  "did SERVICE_MAP / BACKEND_MAP actually hit, and what did it return?"
  for the Phase 2 HoM chained-lookup path without instrumenting the
  program.
- Per-skb tracking: `pwru --filter-track-skb '<pcap-filter>'` follows
  one skb through clones / linearisation / `pskb_expand_head` until
  it dies. This is the right altitude (per § 7) when the question is
  *where in the path does this die*; for *what's wrong with the
  data*, pair it with a `kfunc:vmlinux:kfree_skb_reason` skb-metadata
  dump at the death site.

What it is NOT:

- **Not a CI gate.** pwru is a developer debugging aid, not part of
  the merge envelope. The Tier 2 / Tier 3 suites remain authoritative
  — a passing pwru trace does not substitute for either.
- **Not a substitute for Tier 1 DST.** pwru explains *kernel* packet
  flow; it cannot reproduce concurrency or partition bugs in
  control-plane logic.
- **Not a substitute for source-line resolution.** pwru tells you the
  function name; the source line still needs § 9's symbol resolution
  for the actual mechanism.

---

## Leftover XDP attachments across runs (Lima / Linux integration tests)

Integration tests that load XDP onto a real interface — `crates/
overdrive-dataplane/tests/integration/{atomic_swap,maglev_real,
redirect_neigh_attach,reverse_nat_e2e,sanity_mixed_batch,
service_map_forward,veth_attach}.rs` running through `cargo xtask
lima run --` — and the `xdp-perf` gate (which loads xdp-trafficgen /
xdp-bench, both libxdp-based and using the `xdp_dispatcher`
program-array shape) attach an XDP program to an iface for the
duration of the test.

When a test crashes mid-run, gets cancelled by nextest's
slow-test killer, gets SIGKILL'd by the Bash tool's wall-clock cap,
or is interrupted by the user, the RAII guard that detaches the
program does NOT run — the program stays attached to the iface
until something explicitly removes it.

### Why this matters

Once a stale XDP program is attached to `lo` (or any iface a later
test plumbs traffic through), every TCP / UDP / ICMP packet on that
iface traverses the program before the kernel networking stack sees
it. The program was loaded against a *previous* test's map state and
expectations; against a fresh test's traffic it can drop packets
silently, mangle headers, or simply pass everything through with
side effects (counters bumped, ring buffers drained). The exact
shape depends on what the leftover program was, but the steady-state
symptom on `lo` is the same: **TCP connections to `127.0.0.1:<port>`
that should `ECONNREFUSED` immediately just hang until timeout**.

This bites adjacent tests that have nothing to do with the
dataplane. The 6 `overdrive-cli` integration tests that exercise the
in-process control-plane HTTPS server (`http_client::*`,
`cluster_and_node_commands::*`, `endpoint_from_config::*`,
`exec_spec_walking_skeleton::*`) all failed with the same Transport
timeout signature after a prior xdp-perf run left
`xdp_dispatcher` attached to `lo`. The control-plane code was not
broken; the loopback path was.

The failure looks like a regression in whichever test is under
audit; it isn't. It's leftover state from whatever ran (or crashed)
before.

### Detection — the canonical one-liner

```bash
cargo xtask lima run -- bash -lc '
  for i in $(ip -br link show | awk "{print \$1}"); do
    info=$(ip link show "$i")
    case "$info" in
      *xdpgeneric*|*xdpdrv*|*xdp\ *)
        echo "=== $i ==="
        echo "$info" | grep -E "xdp(generic|drv)?"
        ;;
    esac
  done
  echo "--- bpftool prog show (xdp/sched_cls only) ---"
  bpftool prog show 2>/dev/null | grep -E "(xdp|sched_cls)"
'
```

Output naming `xdp_dispatcher`, `xdp_service_map_lookup`,
`xdp_reverse_nat`, or any other Overdrive-side XDP program attached
to `lo` / `veth0` / `overdrive-veth-*` is the smoking gun.

A faster sanity probe when you suspect the loopback is the problem:

```bash
cargo xtask lima run -- bash -lc \
  'timeout 3 bash -c "echo > /dev/tcp/127.0.0.1/1" 2>&1 \
   || echo "loopback path is healthy (refused, did not hang)"'
```

`Connection refused` is the expected output (nothing listens on
:1). A 3 s timeout is the failure shape — proceed to the detection
one-liner above.

### Cleanup — before re-running the affected suite

```bash
cargo xtask lima run -- bash -lc '
  for i in $(ip -br link show | awk "{print \$1}"); do
    ip link set dev "$i" xdpgeneric off 2>/dev/null
    ip link set dev "$i" xdpdrv off 2>/dev/null
    ip link set dev "$i" xdp off 2>/dev/null
  done
'
```

`xdpgeneric off` / `xdpdrv off` / `xdp off` are the three detach
shapes; only one will fire per iface, the others are harmless
no-ops. Running all three covers attach-mode uncertainty (the
fallback in § "Attach mode" of `development.md` means we don't
always know which mode the leftover program is in).

Veth pairs created by the integration tests usually clean themselves
on `Drop` (one `ip link del <peer>` removes both sides), but when
the test was SIGKILLed mid-setup the pair survives. Sweeping them is
parallel to the XDP detach:

```bash
cargo xtask lima run -- bash -lc '
  for i in $(ip -br link show type veth | awk "{print \$1}"); do
    case "$i" in
      overdrive-veth-*) ip link del "$i" 2>/dev/null ;;
    esac
  done
'
```

### Prevention — what RAII already does, when it runs

`aya::programs::Xdp::attach()` returns an `XdpLinkId` whose `Drop`
detaches the program. As long as the test process exits via the
normal unwind path (panic OR clean return), the link is detached on
scope exit. That handles the common case.

`xdp-trafficgen` and `xdp-bench` (libxdp-based) detach on
`SIGINT` / `SIGTERM`. The `xdp-perf` gate driver in
`crates/overdrive-dataplane/bin/xdp_perf.rs` wraps these subprocesses
and signals them on shutdown.

Neither path fires when the parent process itself dies abnormally —
SIGKILL from nextest's `slow-timeout` (default 60 s test, 120 s
leak), SIGKILL from the Bash tool's wall-clock cap, SIGINT from the
user mid-run. After any such event, expect leftover XDP.

### Don't paper over it

If a previously-passing CLI / control-plane / non-dataplane test
suddenly starts timing out against a loopback HTTPS server, **run
the loopback sanity probe before assuming the recent changes broke
the test**. The failure shape — `request timed out` after the
client's full timeout — is identical to "the server crashed mid-bind"
and "we wired the wrong port"; the only way to distinguish is to
check the loopback path itself.

The fix is to detach the leftover program, not to add defensive
retries or longer client timeouts to the test under audit (the
production code path IS the loopback, and lengthening the timeout
just shifts the failure threshold without removing the cause).

The same discipline as the cgroup-leak section in `testing.md`
applies: tests that attach XDP must NOT silently reuse a
pre-existing attachment. Production code does not assume the iface
has a stale program ready to be replaced; tests should not paper
over a leak by replacing one stale program with another.
