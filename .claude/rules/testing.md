# Testing Guidelines

Overdrive tests in four tiers. Each tier catches a class of bug the others
cannot. None of them substitutes for any other.

```
Tier 1  DST in-process            turmoil + Sim* traits        (§21)
Tier 2  BPF unit tests            BPF_PROG_TEST_RUN            (§22)
Tier 3  Real-kernel integration   QEMU + kernel matrix         (§22)
Tier 4  Verifier + perf gates     veristat, xdp-bench, PREVAIL (§22)
```

---

## Testing

**No `.feature` files anywhere.** All acceptance and integration tests are
written directly in Rust using `#[test]` / `#[tokio::test]` functions.
Gherkin-style scenarios may appear as GIVEN/WHEN/THEN blocks in
`docs/feature/{id}/distill/test-scenarios.md` for specification purposes
only — they are never parsed or executed. The crafter translates those
scenarios into Rust integration tests in
`crates/{crate}/tests/acceptance/*.rs` (or `tests/*.rs`) using
`ScenarioBuilder`. Do NOT introduce cucumber-rs, pytest-bdd, conftest.py,
or any `.feature` file consumer.

---

## Tier 1 — Deterministic Simulation Testing

### Nondeterminism must be injectable

Every source of nondeterminism in core logic is behind a trait. No exceptions:

| Trait | Real | Sim |
|---|---|---|
| `Clock` | `SystemClock` | `SimClock` (turmoil) |
| `Transport` | `TcpTransport` | `SimTransport` (turmoil) |
| `Entropy` | `OsEntropy` | `SeededEntropy` (StdRng) |
| `Dataplane` | `EbpfDataplane` | `SimDataplane` (HashMap) |
| `Driver` | `CloudHypervisorDriver` etc. | `SimDriver` |
| `IntentStore` | `LocalStore` / `RaftStore` | `LocalStore` |
| `ObservationStore` | `CorrosionStore` | `SimObservationStore` (in-memory LWW) |
| `Llm` | `RigLlm` | `SimLlm` (transcript replay) |

Rules:

- **Never call `Instant::now()`, `SystemTime::now()`, `Duration::from_secs` +
  `tokio::time::sleep`, `rand::random()`, `rand::thread_rng()`, or
  `std::thread::sleep` in core logic crates.** These are allowed only in
  wiring crates where the real implementations of the traits live.
- **Never spawn raw `TcpStream`, `TcpListener`, or `tokio::net::*` in core
  logic.** Go through `Transport`.
- **Never call the kernel or `aya-rs` directly from control-plane logic.**
  Go through `Dataplane`.
- A lint / grep CI gate enforces the above at the crate boundary. If the
  gate flags your code, the fix is a new method on the trait, not a
  bypass.

### What to write as DST

Every control-plane behaviour whose correctness depends on ordering, timing,
concurrency, or partition tolerance. If the behaviour is "single node,
single thread, no clock dependency," a plain `#[test]` is fine — DST is the
wrong tool.

Concrete must-haves:

- Leader election under partition, clean crash, clock skew.
- Scheduler placement under concurrent job submission.
- Certificate rotation across leader changes.
- Reconciler convergence after node rejoins.
- Corrosion gossip under peer event-loop stalls (the Fly contagion
  scenario is a *named* scenario, not a hypothetical).
- Investigation agent against seeded `SimLlm` transcripts — deviation in
  tool choice or parameter shape fails the test.

### Properties, not scenarios

Prefer invariants over scripted assertions. Three categories:

```rust
// Safety — nothing bad ever happens
assert_always!("single leader",
    cluster.nodes().filter(|n| n.is_leader()).count() <= 1);

assert_always!("intent never crosses into observation",
    corrosion.tables().all(|t| !t.contains_intent_class()));

// Liveness — good things eventually happen
assert_eventually!("job scheduled",
    submitted_jobs.iter().all(|j| j.has_allocation()));

// Convergence — reconcilers reach desired state
assert_eventually!("desired == actual",
    desired_state == actual_state);
```

Rules:

- Every built-in reconciler ships with ESR specifications (progress +
  stability) expressible as `assert_always!` / `assert_eventually!` pairs.
- A scripted scenario test without an invariant is a smell — ask what the
  scenario is actually defending.

### Seeding and reproducibility

- Every DST test takes a seed. On failure, the harness prints the seed.
- `cargo xtask dst --seed <N>` reproduces bit-for-bit.
- Flaky DST is a bug in the sim layer, never a "just rerun it." Fix or
  file.

### Store composition

Four store modes. Use the narrowest that exercises the behaviour:

- `LocalStore` + `SimObservationStore` — single-node, most tests.
- `RaftStore` + `SimObservationStore` — consensus tests only.
- `LocalStore` + real `CorrosionStore` — cross-region gossip tests (these
  are slower; reserve for the behaviour that requires real CR-SQLite LWW
  semantics).
- `RaftStore` + real `CorrosionStore` — full-stack integration, sparing.

---

## Tier 2 — BPF Unit Tests

### Triptych shape

Each eBPF program ships three companions in `crates/overdrive-bpf/tests/`:

- `PKTGEN` — synthetic packet or syscall context.
- `SETUP` — populates the BPF maps the program reads.
- `CHECK` — drives `BPF_PROG_TEST_RUN` via `aya::Program::test_run()`,
  asserts on output / verdict / map mutations.

### Rules

- **Map state is cleared between sub-tests by default.** Persistent state
  across sub-tests is opt-in via `#[test_chain]` for cases that genuinely
  need staged setup (atomic-swap semantics, etc.). Default-persist was the
  Cilium choice; we chose default-isolate to match idiomatic Rust `#[test]`
  and avoid phantom failures.
- **Only applicable where `BPF_PROG_TEST_RUN` is the right mechanism** —
  XDP, TC. Sockops and BPF LSM move entirely to Tier 3 (the kernel does
  not expose `PROG_TEST_RUN` meaningfully for these).
- **Tier 2 does not prove hook attachment.** It proves program-level
  correctness against curated input. Attachment and invocation are
  Tier 3.

---

## Tier 3 — Real-Kernel Integration

### Kernel matrix

Every merge runs against:

- 5.10 LTS (floor — BPF LSM + kTLS + sockops jointly stable)
- 5.15 LTS (Ubuntu 22.04, Debian 12 backports, RHEL 9)
- 6.1 LTS (Debian 13)
- 6.6 LTS (Ubuntu 24.04)
- Current LTS
- `bpf-next` (soft-fail; nightly gate)

Adding a kernel is one line of YAML against `little-vm-helper`. Dropping a
kernel requires an ADR.

### Harness

- CI: `little-vm-helper` (OCI kernel images; same tooling Cilium, Tetragon,
  pwru use).
- Dev laptops: `virtme-ng` (~1s boot from a kernel tree).
- Entry point: `cargo xtask integration-test vm --cache-dir <CACHE_DIR>
  <KERNEL>...` — reuses aya's existing flow, do not fork.
- GitHub Actions runners work with `--qemu-disable-kvm`; self-hosted
  KVM-capable runners optional for latency budget.

### Assertion rules

Assert on observable kernel side effects. Never on program internal
reachability ("the program took branch X").

Three observable layers:

- **Kernel-side state**
  - BPF maps: `bpftool map dump`
  - TLS ULP: `ss -K`
  - LSM decisions: BPF ringbuf event stream (the *event*, not "the program
    returned `EPERM` early")
- **Userspace state**
  - Structured flow events from the Overdrive telemetry ringbuf
- **Wire capture**
  - `tcpdump` on veth interfaces
  - Expected ciphertext (kTLS), expected forwarding (XDP SERVICE_MAP)

Counter-example (do not do this):

```rust
// WRONG — asserts the program reached a branch, not that the kernel
// invoked the hook.
assert_eq!(ebpf_program.last_action.load(Ordering::Relaxed), ACTION_DENY);
```

Correct form:

```rust
// The hook fires and the userspace observer sees the deny event.
let event = ringbuf.recv_within(Duration::from_secs(1))?;
assert_eq!(event.verdict, Verdict::Deny);
assert_eq!(event.lsm_hook, LsmHook::FileOpen);
```

### Mandatory test cases per hook

Every new eBPF program lands with the coverage below or it does not merge:

| Hook | Minimum coverage |
|---|---|
| XDP | Atomic map swap under load; zero-drop invariant across the update |
| TC | Egress redirection path through `SIDECAR_MAP` |
| sockops | ULP install verified via `ss -K`; handshake failure on wrong SVID |
| sockops + kTLS | Wire capture shows TLS 1.3 records |
| BPF LSM | Positive *and* negative case per policy bit (denied + allowed) |
| End-to-end | IntentStore write → Corrosion propagation → kernel verdict |

---

## Tier 4 — Verifier and Performance Gates

### Verifier complexity (`veristat`)

- Full BPF corpus compiled with worst-case feature flags, loaded into every
  matrix kernel.
- Baseline on `main`. PR fails if:
  - Any program exceeds its baseline instruction count by >5%.
  - Any program approaches the per-program complexity ceiling by >10%.
- Verifier behaviour changes across kernel releases. The only guard is
  loading the corpus into every kernel in the matrix. Do not rely on a
  single-kernel verifier-pass signal.

### XDP performance (`xdp-bench`)

- `xdp-trafficgen` → SUT → sink, two veth pairs inside an LVH VM.
- Baseline per-runner-class pps and p99 latency under
  `perf-baseline/main/`.
- PR fails if relative delta exceeds:
  - 5% pps regression
  - 10% p99 latency regression
- **Never gate on absolute numbers** — runner hardware varies enough to
  make absolute gates flaky. Deltas only.

### Second-opinion static analysis (PREVAIL)

- Nightly, non-blocking.
- Fails the build when PREVAIL disagrees with the kernel verifier's
  accept/reject decision.
- This defends against verifier bugs, not just program bugs.

---

## Fault injection catalogue

Every release exercises the fault classes below. The DST fault and its
real-kernel complement are written together — neither alone is sufficient.

| Class | DST (Tier 1) | Real kernel (Tier 3) |
|---|---|---|
| Network partition | `SimTransport.partition()` | `tc qdisc … netem loss 100%` on veth |
| Packet loss | `SimTransport` loss | `netem loss 5%` |
| Reordering | `SimTransport` reorder | `netem reorder 50% gap 3` |
| Latency | `SimTransport` delay | `netem delay 100ms 20ms` |
| Clock skew | `SimClock` offset | VM boot with offset `CLOCK_REALTIME` |
| Node crash | restart hook in turmoil host | `kill -9` the in-VM binary |
| Corrosion gossip stall | `SimObservationStore` stall | real Corrosion; pause peer event loop |
| Schema migration storm | `SimObservationStore` migration | additive migration against real Corrosion |
| Driver failure | `SimDriver` configured to fail | inject bad kernel image in CH |
| Policy eval timeout | inject `Llm`/`Regorus` hang | hang the real Regorus call |

The same catalogue drives the chaos engineering reconciler in production.
Tests and chaos share the fault definitions; a fault is specified once.

---

## CI topology

```
Per-PR (critical path ≈ 15 minutes):
  A  cargo test                          pure Rust, no BPF             (s)
  B  cargo xtask dst                     Tier 1                        (min)
  C  cargo xtask bpf-unit                Tier 2                        (min)
  D  cargo xtask integration-test vm     Tier 3, kernel matrix         (10 min)
  E  cargo xtask verifier-regress        Tier 4 — veristat             (min)
     cargo xtask xdp-perf                Tier 4 — xdp-bench            (min)

Nightly:
  F  Tier 3 + Tier 4 against bpf-next                                  soft-fail
  G  PREVAIL second-opinion analysis                                   soft-fail
  H  Long-run fault-injection soak with random netem profiles

Per-release:
  I  Full Tier 3 matrix on aarch64 (self-hosted Graviton runner)
```

---

## Scope boundaries

Explicitly out of scope:

- **Real hardware NIC drivers.** We run against virtio-net and veth in
  QEMU — the same envelope Cilium, Tetragon, and upstream BPF CI use. Real
  hardware validation lives in a per-release lab, not per-PR.
- **Kernel selftests.** We do not re-run `tools/testing/selftests/bpf`.
  That is the kernel's job. We rely on each supported kernel having passed
  its own selftests.
- **Production chaos as a CI substitute.** The chaos reconciler validates
  emergent production behaviour. It does not replace pre-merge gating.

---

## Adding a new test — which tier?

```
Logic bug under concurrency, timing, ordering, or partition?
    → Tier 1 (DST)

eBPF program-level correctness against curated input?
    → Tier 2 (BPF unit)

Does the program actually load, attach, and enforce on real kernels?
    → Tier 3 (integration)

Does a change bloat verifier complexity or regress XDP throughput?
    → Tier 4 (perf / verifier gates)
```

When in doubt, start with Tier 1 and promote upward. DST failures are the
cheapest to reproduce; real-kernel failures are the cheapest to trust.
