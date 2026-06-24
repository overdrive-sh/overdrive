# RCA — `unconnected_udp_roundtrip` cross-process concurrency flake (PR #245 CI)

**Analyst:** Rex (Toyota 5-Whys RCA)
**Date:** 2026-06-24
**Branch / commit:** `marcus-sa/path-a-inbound-tproxy` @ `d49ea7d4`
**CI surface:** `integration` job, profile `ci`, `-E binary(integration)`
**Kernel (repro):** `7.0.0-22-generic` (dev Lima VM)

---

## Binary verdict

**(a) host-kernel-shared serialization gap.** The four
`overdrive-dataplane::integration::unconnected_udp_roundtrip` tests fail
under cross-process concurrency because they — and a set of sibling
dataplane tests — attach the production `cgroup_connect4_service` /
`cgroup_sendmsg4_service` / `cgroup_recvmsg4_service`
`cgroup_sock_addr` programs to the **process-global root cgroup
`/sys/fs/cgroup`** with `CgroupAttachMode::Single`, yet are **NOT
members** of the `.config/nextest.toml` `[test-groups.host-kernel-shared]`
(`max-threads = 1`) cross-process single-writer group. nextest therefore
schedules them concurrently across separate processes; a sibling
process's cgroup attach displaces/contends the victim's `sendmsg4`
program on the shared root cgroup, and because each process's
`sendmsg4`/`recvmsg4` programs read that process's **own** per-process
`LOCAL_BACKEND_MAP` / `REVERSE_LOCAL_MAP` (declared `pinning = NONE`, so
created fresh per `EbpfDataplane::new_with_pin_dir`), the victim's
`sendto(10.96.0.10:53)` traverses a program bound to a foreign map with
no matching backend → no forward rewrite → the 2-second round-trip poll
exhausts and the test panics.

This is the **same root-cause class** as the mTLS flake fixed in
`d49ea7d4` ("serialize the full mTLS Tier-3 surface in
host-kernel-shared"): a shared-kernel-state Tier-3 test surface omitted
from the cross-process `max-threads = 1` group.

**(b) genuine 2s-budget timing bug — RULED OUT.** Every reproduced
failure consumed the **full ~2.2–2.9 s** poll budget and the round-trip
**never completed** (map MISS / no rewrite), not a map-HIT-with-late-echo.
A timing bug would show late completion, not non-completion.

**(c) real `sendmsg4` regression on this branch — RULED OUT.** `git diff
origin/main` shows **zero** changes to the production cgroup
`sendmsg4`/`recvmsg4` programs, the `LOCAL_BACKEND_MAP`/`REVERSE_LOCAL_MAP`
definitions, or the userspace attach/register path (`lib.rs`). The
roundtrip test pre-exists on `origin/main` unchanged. The same test
passes 4/4 every time in isolation and under `--test-threads 1`.

Serialization is the **correct fix, not a mask**: the failure is pure
cross-process contention on a genuinely process-global kernel resource
(the root-cgroup attach + per-process maps), reproducible only when a
sibling process is mid-attach. There is no latent timing or correctness
bug underneath that serialization would hide.

---

## Proven mechanism (producer → consumer chain)

The shared surface, the interferer, and the corrupted step, by evidence:

1. **The shared surface — the global root cgroup `/sys/fs/cgroup`.**
   Every cgroup-UDP dataplane test calls
   `EbpfDataplane::new_with_pin_dir(host, peer, pin_dir,
   Path::new("/sys/fs/cgroup"))`
   (`unconnected_udp_roundtrip.rs:110-116`). `EbpfDataplane::new_with_pin_dir`
   attaches all three `cgroup_sock_addr` programs to that cgroup FD with
   **`CgroupAttachMode::Single`** (`crates/overdrive-dataplane/src/lib.rs:710,
   744, 764`). The cgroup path is **hard-coded**, not parameterizable
   per-test.

2. **`Single` → kernel flag `0`** (aya 0.13.1
   `src/programs/links.rs`: `CgroupAttachMode::Single => 0`; the attach
   goes through `bpf_link_create(..., flags = 0, ...)` on kernel ≥ 5.7,
   `src/programs/cgroup_sock_addr.rs:80-95`). Neither `BPF_F_ALLOW_OVERRIDE`
   nor `BPF_F_ALLOW_MULTI`. Multiple concurrent processes each create a
   link of the same attach-type on the same root cgroup; the kernel does
   not isolate one process's program from another's traffic on a shared
   cgroup.

3. **The per-process maps are NOT shared (and that is what makes the
   override lethal).** `LOCAL_BACKEND_MAP` and `REVERSE_LOCAL_MAP` are
   declared `HashMap::with_max_entries(MAX_ENTRIES, 0)` with **`pinning =
   NONE`** (`crates/overdrive-bpf/src/maps/local_backend_map.rs:88-89`,
   `crates/overdrive-bpf/src/maps/reverse_local_map.rs:97-98`) — only the
   XDP `SERVICE_MAP` HoM is `pinning = ByName`. So each
   `EbpfDataplane::new_with_pin_dir` gets a **fresh, private**
   `LOCAL_BACKEND_MAP`/`REVERSE_LOCAL_MAP`. When process A's `sendto`
   traverses process B's `sendmsg4` program, that program reads **B's**
   `LOCAL_BACKEND_MAP`, which has no entry for A's `(10.96.0.10, 53, udp)
   → A's backend` (or rewrites toward B's backend). Either way A's
   datagram never reaches A's stub resolver.

4. **The interferers — same `VIP:53` + same root cgroup.** Population diff
   of the VIP/port constants across the candidate siblings:

   | Test | VIP | VIP_PORT | attaches cgroup progs to `/sys/fs/cgroup`? | same `10.96.0.10:53`? |
   |---|---|---|---|---|
   | `unconnected_udp_roundtrip` (victim) | `10.96.0.10` | 53 | yes | — |
   | `unconnected_udp_reply_hardening` | `10.96.0.10` | 53 | yes | **YES** |
   | `service_map_vip_port` | `10.96.0.10` | 53 | yes | **YES** |
   | `deregister_retry_safety` | `10.96.0.11` | 53 | yes | no (diff VIP) |
   | `local_backend_proto_connect` | `10.99.0.1` | 5353 | yes | no |
   | `reverse_nat_udp_e2e` (×3) | XDP `update_service` path | — | yes (cgroup attach) | n/a |
   | `multi_listener_tcp_udp_e2e` (×4) | XDP `update_service` path | — | yes (cgroup attach) | n/a |

   The exact-key interferers are `unconnected_udp_reply_hardening` and
   `service_map_vip_port`. But the **necessary condition is the shared
   root-cgroup attach**, not the map-key collision — ANY of these
   processes attaching its three `cgroup_sock_addr` programs to the same
   root cgroup mid-flight is sufficient to displace/contend the victim's
   `sendmsg4`. The cgroup-attach sampler proved multiple processes attach
   concurrently (root-cgroup overdrive attach count oscillated through
   `0, 3, 6, 9, 12, 15` — exactly 3 programs per concurrently-attached
   process).

5. **The corrupted step — the forward `sendmsg4` dest rewrite.** The
   victim panics at the forward-rewrite assertion
   (`unconnected_udp_roundtrip.rs:213`/`434`: "did not round-trip + echo
   within 2s — cgroup sendmsg4 forward rewrite (VIP→backend) regression")
   and at the second-query reuse assertion (`:341`/`:322`). All are the
   `poll_until(Duration::from_secs(2), …)` returning `None` — the forward
   rewrite never fired, so the datagram never reached the backend.

### Producer-before-consumer confirmation (debugging.md §11)

The panic text names "regression" (the author's **taxonomy**, §2 — the
layer that gave up), not the mechanism. The mechanism is **producer
displacement**: the victim's `sendmsg4` *producer* (its program +
populated map) was contended by a sibling on the shared cgroup, so the
*consumer* (`recvfrom`) saw an empty round-trip. Confirmed by:
the production code is byte-identical to `origin/main` (no regression),
the test passes 4/4 serialized (producer works when un-contended), and
fails only with a concurrent cgroup-attaching sibling present.

---

## Probe log — Hypothesis / Prediction / Falsification

### Probe 6a — serialized baseline (ground-truth re-confirm)

- **Hypothesis:** With cross-process serialization the four roundtrip
  tests always pass.
- **Prediction:** 4/4 pass, sub-second.
- **Falsification:** any failure → the bug is not concurrency.

```
$ cargo xtask lima run -- cargo nextest run ... --profile ci --test-threads 1 \
    -E 'binary(integration) & package(overdrive-dataplane) & test(unconnected_udp_roundtrip)'
 Starting 4 tests across 1 binary (53 tests and 73 binaries skipped)
     Summary [   0.514s] 4 tests run: 4 passed, 53 skipped
```

**Result: confirmed.** 4/4 pass in 0.514 s. Hypothesis held.

### Probe 6b — single concurrent batch (insufficient load)

- **Hypothesis:** One concurrent batch of victim + siblings reproduces.
- **Prediction:** ≥1 failure.
- **Falsification:** all pass → need the cross-process *loop*, not a
  single batch.

```
 Starting 17 tests across 1 binary (40 tests and 73 binaries skipped)
     Summary [  11.194s] 17 tests run: 17 passed, 40 skipped
```

**Result: no failure** — a single nextest batch (one process, 8 threads)
does not reliably reproduce; the race needs *separate processes* mid-attach.
This is exactly why `serial_test` (in-process) cannot defend it and why
the established Probe 5 used separate-process loops. Re-confirms the
cross-process nature.

### Probe 6c — first cross-process loop attempt (harness bug)

The inline `$!` PID capture was corrupted by the Bash tool's history-`!`
escaping (`INTERF=$'\!'`), so the background interferer loop never ran;
the victim looped essentially in isolation and passed 8/8. Discarded as a
harness artifact — NOT negative evidence (the interferer never started;
sampler saw mostly `root_ovd_attaches=0`). Re-run as a script file
(Probe 6d) to dodge the escaping.

### Probe 6d — cross-process reproduction (DECISIVE)

- **Hypothesis:** The victim roundtrip tests (separate nextest process,
  looped) fail when a background loop attaches same-root-cgroup siblings
  as separate processes; serialization (Probe 6a) is the only thing that
  prevents it.
- **Prediction:** ≥1 victim iteration panics with the exact CI sites
  (`:213`/`:341`/`:434`), full-2s-timeout (never completes); the
  root-cgroup overdrive attach count thrashes in multiples of 3.
- **Falsification:** all pass (→ not a serialization gap), OR failures
  show map-HIT-late-echo (→ timing bug).

```
=== PROBE 6d: VICTIM roundtrip looped vs background same-VIP interferers ===
iter1: ok --      Summary [   0.265s] 4 tests run: 4 passed, 53 skipped
iter2: FAILED --      Summary [   2.897s] 4 tests run: 2 passed, 2 failed, 53 skipped
        FAIL [   2.345s] (3/4) ...::kernel_reply_source_meets_tier1_reply_mirror_at_backend_identity
    panicked at crates/overdrive-dataplane/tests/integration/unconnected_udp_roundtrip.rs:434:24:
    unconnected sendto(VIP:53) did not round-trip + echo within 2s — cgroup sendmsg4 forward rewrite (VIP→backend) regression
        FAIL [   2.897s] (4/4) ...::second_unconnected_query_reuses_same_mapping_statelessly
    panicked at crates/overdrive-dataplane/tests/integration/unconnected_udp_roundtrip.rs:341:6:
    second unconnected query did NOT reuse the same mapping within 2s — ... the cgroup path is stateless point-lookup
iter3: ok --      Summary [   0.120s] 4 tests run: 4 passed, 53 skipped
iter4: FAILED --      Summary [   2.216s] 4 tests run: 3 passed, 1 failed, 53 skipped
    panicked at crates/overdrive-dataplane/tests/integration/unconnected_udp_roundtrip.rs:322:6:
iter5: ok ...
iter6: FAILED -- ...rs:341:6: second unconnected query did NOT reuse the same mapping within 2s ...
iter11: FAILED -- ...rs:434:24 + rs:341:6
iter16: FAILED -- ...rs:434:24: ... did not round-trip + echo within 2s — cgroup sendmsg4 forward rewrite ...
iter19: FAILED -- ...rs:341:6 + rs:213:24: unconnected sendto(VIP:53) did not round-trip + echo within 2s ...
iter20: FAILED -- ...rs:434:24 ...
iter1,3,5,7,8,9,10,12-15,17,18,21-24: ok (4 passed)
=== VICTIM failing iterations: 7 / 24 ===
=== cgroup ROOT overdrive-attach-count distribution ===
     69 root_ovd_attaches=0
     35 root_ovd_attaches=9
     32 root_ovd_attaches=3
     26 root_ovd_attaches=6
     26 root_ovd_attaches=12
      1 root_ovd_attaches=15
     (+ small counts of 1,2,4,5,8,10)
```

**Result: confirmed, 7/24 failures.** Every reported CI panic site
reproduced (`:213`, `:322`, `:341`, `:434`); every failure took
~2.2–2.9 s (full 2-s budget — round-trip never completed, NOT a late
echo); failing iterations interleave with passing iterations of the same
tests (pure concurrency, not a deterministic code bug). The cgroup
attach count is **always a multiple of 3** (0/3/6/9/12/15) — 3 programs
per concurrently-attached process — proving multiple processes attach to
the shared root cgroup simultaneously. Hypothesis held; (b) and (c)
falsified by the full-timeout shape and the zero production diff.

### Probe — rule out (b) timing / (c) regression

```
$ git diff --stat origin/main -- \
    crates/overdrive-bpf/src/programs/cgroup_sendmsg4_service.rs \
    crates/overdrive-bpf/src/programs/cgroup_recvmsg4_service.rs \
    crates/overdrive-bpf/src/maps/local_backend_map.rs \
    crates/overdrive-bpf/src/maps/reverse_local_map.rs
   (empty — no changes)
$ git diff --stat origin/main -- crates/overdrive-dataplane/src/lib.rs
   (empty — no changes)
$ git cat-file -e origin/main:crates/overdrive-dataplane/tests/integration/unconnected_udp_roundtrip.rs
   PRESENT on origin/main   # test pre-exists; not newly-added/flaky-by-construction
```

**Result:** production cgroup path and the test are unchanged from
`origin/main` → (c) falsified. Full-2s-timeout non-completion (not late
echo) → (b) falsified.

### Membership probe — who is / isn't in `host-kernel-shared`

`cargo nextest show-config test-groups --profile ci` resolves the
`host-kernel-shared` (`max-threads = 1`) group's dataplane members to
ONLY the XDP/atomic-swap/maglev/sanity/`service_map_forward`/`reverse_nat_e2e`
per-fn entries **plus** `package(overdrive-dataplane) & test(mtls)`
(by-module). **None** of `unconnected_udp_roundtrip`,
`unconnected_udp_reply_hardening`, `service_map_vip_port`,
`reverse_nat_udp_e2e`, `multi_listener_tcp_udp_e2e`,
`deregister_retry_safety`, or `local_backend_proto_connect` are members
— every one of them attaches the three `cgroup_sock_addr` programs to the
shared `/sys/fs/cgroup` root and runs **unserialized**.

---

## Toyota 5-Whys

```
PROBLEM: overdrive-dataplane::integration::unconnected_udp_roundtrip tests
pass in isolation but fail under concurrent CI load (PR #245), blocking merge.

WHY 1A: The roundtrip tests panic at unconnected_udp_roundtrip.rs:213 / 341 /
        434 (and :322) — "sendto(VIP:53) did not round-trip + echo within 2s".
  [Evidence: Probe 6d — 7/24 iterations fail at exactly these sites; each
   failure consumes the full ~2.2–2.9 s poll budget (poll_until(2s) → None).]

  WHY 2A: The cgroup `sendmsg4` forward rewrite (VIP→backend) does not fire
          for the victim's sendto — the datagram is delivered to 10.96.0.10:53
          (nothing bound) instead of the stub resolver, so recvfrom times out.
    [Evidence: round-trip never completes (full-2s timeout, not a late echo);
     production sendmsg4 program is byte-identical to origin/main (no regression).]

    WHY 3A: The victim's `cgroup_sendmsg4_service` program — bound to the
            victim process's OWN LOCAL_BACKEND_MAP (carrying the
            10.96.0.10:53 → victim-backend entry) — is displaced/contended on
            the shared global /sys/fs/cgroup root by a CONCURRENT sibling test
            process's cgroup attach. The sibling's sendmsg4 reads the SIBLING's
            LOCAL_BACKEND_MAP, which has no matching entry, so no rewrite fires.
      [Evidence: LOCAL_BACKEND_MAP / REVERSE_LOCAL_MAP declared pinning=NONE
       (kernel maps local_backend_map.rs:88, reverse_local_map.rs:97) → fresh,
       private per EbpfDataplane::new_with_pin_dir; cgroup attach sampler shows
       root-cgroup attach count oscillating 0/3/6/9/12/15 (3 progs × N concurrent
       procs); all three progs attach to the hard-coded "/sys/fs/cgroup"
       (lib.rs:684,710,744,764).]

      WHY 4A: All cgroup-UDP dataplane tests attach to the SAME process-global
              root cgroup "/sys/fs/cgroup" with CgroupAttachMode::Single (kernel
              flag 0 — no per-process isolation), and they rely on
              #[serial_test::serial(env)] for mutual exclusion — but serial_test
              only synchronises WITHIN one process. nextest runs each test in its
              OWN process.
        [Evidence: lib.rs:710/744/764 CgroupAttachMode::Single; aya links.rs
         CgroupAttachMode::Single => 0; every test file annotates
         #[serial_test::serial(env)] (e.g. roundtrip.rs:189,248,298,405); the
         cgroup attach path is not parameterizable per-test without changing
         production.]

        WHY 5A (ROOT CAUSE): These tests were never added to the
              .config/nextest.toml [test-groups.host-kernel-shared]
              (max-threads = 1) cross-process single-writer group — the ONLY
              level that can serialize separate nextest PROCESSES against a
              shared kernel resource. So nextest schedules them concurrently
              and the shared-root-cgroup attach + per-process map race fires.
          [Evidence: `nextest show-config test-groups --profile ci` lists the
           dataplane members of host-kernel-shared as ONLY the per-fn XDP/atomic-
           swap/etc. entries + `package(overdrive-dataplane) & test(mtls)`; NONE
           of the unconnected-UDP / cgroup-UDP tests appear. The group's own
           docstring (nextest.toml:130-177) explicitly names "the per-allocation
           cgroup hierarchy" and "by-name cgroup_connect4 program" as the shared
           state it guards — the cgroup_sock_addr-on-root surface is in-scope for
           the group but the membership filter omits these tests.]

      -> ROOT CAUSE A: A cross-process serialization gap — the cgroup-UDP
         dataplane Tier-3 tests share the process-global root-cgroup
         cgroup_sock_addr attach surface (+ per-process maps the override makes
         lethal) but are absent from the host-kernel-shared max-threads=1
         test-group. Identical class to the mtls flake fixed in d49ea7d4.

      -> SOLUTION A: Add the cgroup-UDP dataplane test modules to
         host-kernel-shared (see Fix below).
```

### Backwards-chain validation

"If the root cause exists, does it produce the observed symptoms?" —
**Yes, end to end.** Missing membership → nextest runs the tests
concurrently across processes → multiple processes attach
`cgroup_sock_addr` programs to the shared `/sys/fs/cgroup` (sampler:
attach count thrashes in multiples of 3) → a sibling's program (reading
its own empty/foreign `pinning=NONE` `LOCAL_BACKEND_MAP`) wins the
victim's `sendto` → no forward rewrite → 2-s poll exhausts → the exact
`:213`/`:322`/`:341`/`:434` panics. Removing the cause (serialize via
`--test-threads 1`, Probe 6a) removes the symptom (4/4 pass). No
contradiction with the production code (byte-identical to main) and no
competing branch.

---

## Minimal source-correct fix

Add the cgroup-UDP dataplane test surface to the existing
`host-kernel-shared` `max-threads = 1` group in `.config/nextest.toml`.
**Extend the existing by-MODULE mtls override block** (the one
`d49ea7d4` added at nextest.toml:259-281) rather than the by-fn block —
module-level membership is rename-proof and a new test in any of these
files joins the single-writer domain automatically (the lesson already
encoded in that block's own comment and the trybuild "match by
binary/module, not fn-name" precedent).

Add these `package(overdrive-dataplane) & test(<module>::)` disjuncts to
the same override filter that currently carries
`(package(overdrive-dataplane) & test(mtls))`:

```
| (package(overdrive-dataplane) & (
      test(unconnected_udp_roundtrip::)
    | test(unconnected_udp_reply_hardening::)
    | test(service_map_vip_port::)
    | test(reverse_nat_udp_e2e::)
    | test(multi_listener_tcp_udp_e2e::)
    | test(deregister_retry_safety::)
    | test(local_backend_proto_connect::)
  ))
```

(Keep the existing `(package(overdrive-dataplane) & test(mtls))`
disjunct; this adds the cgroup-`sock_addr`-on-root surface alongside it.)

**Why these seven modules and not fewer.** Every one calls
`EbpfDataplane::new_with_pin_dir(..., "/sys/fs/cgroup")` and so attaches
the three `cgroup_sock_addr` programs to the shared root cgroup — the
single shared surface that races. Restricting membership to only the
exact-`VIP:53`-key interferers (`reply_hardening`, `service_map_vip_port`)
would **under-serialize**: the proven necessary condition is the
shared-root-cgroup attach, not the map-key collision, so a
`reverse_nat_udp_e2e` / `multi_listener` / `local_backend_proto_connect`
process attaching concurrently still displaces the victim's program. All
seven modules must be in the same single-writer domain.

**Over-/under-serialization tradeoff.** This serializes ~17 dataplane
Tier-3 tests that already run in single-digit-second wall-clock each;
they join the existing single-writer group that already holds the
XDP/atomic-swap/mtls Tier-3 surface, so the marginal CI cost is bounded
and these tests CANNOT correctly run in parallel anyway (they contend the
one root cgroup). Under-serializing (a partial membership) re-opens the
exact race — reject it.

**By-module vs by-fn.** Extend the **by-module** block. The existing per-fn
block (nextest.toml:202-232) is where the staleness lesson was learned
(`d49ea7d4`'s comment: per-fn entries "went stale on the first refactor
and dropped the worker surface"); module-level keeps a future test in
these files serialized without a nextest.toml edit.

**Production code: NO change.** The hard-coded `/sys/fs/cgroup` attach and
`CgroupAttachMode::Single` are correct production behaviour (a single node
runs ONE dataplane; the root-cgroup ancestor attach is by design,
ADR-0053 §7). The defect is purely test-harness scheduling. Do NOT
parameterize the cgroup path or switch to `AllowMultiple` to dodge the
test race — that would shape production for the test double (a
development.md § "Production code is not shaped by simulation" violation).

**This fix RESOLVES, it does not MASK.** The serialization removes the
sole cause (concurrent root-cgroup attach across processes). There is no
underlying timing or correctness bug: the production path is unchanged
from main, the tests pass deterministically when serialized, and the
failure is a genuine "two single-node dataplanes cannot share one host
kernel's root cgroup" contention — exactly what the single-writer group
exists to prevent.

---

## Prevention (systemic)

1. **Structural rule the group's own comment already implies:** any test
   constructing a real `EbpfDataplane` (cgroup `sock_addr` attach to the
   shared root cgroup, or XDP attach, or by-name bpffs pin) belongs in
   `host-kernel-shared` by construction. The recurring failure mode is a
   *new* such test landing without group membership (this RCA + `d49ea7d4`
   are two instances in one PR's history). A lightweight guard — e.g. an
   xtask check that every `tests/integration/*.rs` in `overdrive-dataplane`
   /`overdrive-worker` referencing `new_with_pin_dir` /
   `/sys/fs/cgroup` resolves into the group — would catch the next one at
   PR time rather than as a CI flake. (Surface to the user before
   creating any tracking issue, per the deferral discipline.)

2. **Prefer module-level membership filters** for shared-kernel-state
   groups (already the stated lesson in the nextest.toml mtls block);
   per-fn filters silently drop renamed/added tests.

---

## Key files / lines

- `crates/overdrive-dataplane/tests/integration/unconnected_udp_roundtrip.rs`
  — VIP `10.96.0.10:53` (`:68`,`:73`); attach `/sys/fs/cgroup`
  (`:110-116`); panic sites `:213`, `:341`, `:434` (and `:322` second-query
  first-poll); all four tests `#[serial_test::serial(env)]`.
- `crates/overdrive-dataplane/src/lib.rs:684,710,744,764` — opens
  `cgroup_attach_path` and attaches `cgroup_connect4_service` /
  `cgroup_sendmsg4_service` / `cgroup_recvmsg4_service` with
  `CgroupAttachMode::Single`.
- `crates/overdrive-bpf/src/maps/local_backend_map.rs:88-89`,
  `crates/overdrive-bpf/src/maps/reverse_local_map.rs:97-98` — both
  `pinning = NONE` (private per process).
- `.config/nextest.toml:190-281` — `[test-groups.host-kernel-shared]`
  (`max-threads = 1`) + its by-fn (`:202-232`) and by-module mtls
  (`:259-281`) override filters. **The fix extends the by-module block.**
- aya 0.13.1 `src/programs/links.rs` (`CgroupAttachMode::Single => 0`) +
  `src/programs/cgroup_sock_addr.rs:74-95` (`bpf_link_create(..., flags =
  mode.into())`).
- Sibling interferers (all attach the three cgroup progs to
  `/sys/fs/cgroup`): `unconnected_udp_reply_hardening.rs` (`10.96.0.10:53`),
  `service_map_vip_port.rs` (`10.96.0.10:53`), `deregister_retry_safety.rs`
  (`10.96.0.11:53`), `local_backend_proto_connect.rs` (`10.99.0.1:5353`),
  `reverse_nat_udp_e2e.rs`, `multi_listener_tcp_udp_e2e.rs`.
```

---

## Addendum — completeness extension (post-RCA cross-crate audit)

This RCA's reproduction used the *dataplane* same-`VIP:53` interferers and so
named seven dataplane modules. A follow-up workspace-wide audit of the
necessary condition — *any* test that constructs a real `EbpfDataplane`, since
`EbpfDataplane::new` / `new_with_pin_dir` attaches `connect4`/`sendmsg4`/
`recvmsg4` at the root `/sys/fs/cgroup` **unconditionally** — found three more
root-attachers still outside the group that would re-open the same race against
the (now-serialised) UDP tests:

- `crates/overdrive-dataplane/.../redirect_neigh_attach.rs:131,135` —
  `new_with_pin_dir(..., Path::new("/sys/fs/cgroup"))`, runs (not `#[ignore]`).
- `crates/overdrive-control-plane/.../serve_boot_provisions_veth.rs:307` —
  `new_with_pin_dir(..., Path::new("/sys/fs/cgroup"))`.
- `crates/overdrive-control-plane/.../backend_discovery_bridge/test_server.rs:111`
  (the helper used by `boot_composition` + `walking_skeleton`) — attaches at
  `/sys/fs/cgroup`; `walking_skeleton.rs:319` documents the default IS root.

Audited and **excluded** (verified non-interfering): `veth_attach`
(`#[ignore]`), `veth_provision_idempotent` (no `EbpfDataplane` —
`VethProvisionPlan` only), `convergence_loop_spawned_in_production_boot`
(`dataplane_cgroup_attach_path: None`), and the `run_server` tests that wire no
dataplane or attach at the default `overdrive.slice` (`submit_round_trip`,
`server_lifecycle`, `serve_persistent_ca`, …) — a different cgroup node whose
programs do not fire for the dataplane tests' (root-cgroup) clients.

The landed fix therefore serialises **ten** modules (seven dataplane UDP +
`redirect_neigh_attach` + `serve_boot_provisions_veth` + `backend_discovery_bridge`),
growing `host-kernel-shared` from 49 → 92 members with nothing previously
serialised dropped. The agent's prevention recommendation — an xtask guard that
fails CI when a real-`EbpfDataplane` integration test lacks group membership —
would have caught all three structurally and remains the durable follow-up.
