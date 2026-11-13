# Findings — DESIGN validation items V-2, V-3, V-6, V-7, V-9

Probe sources and evidence:
`spike-scratch/metal-test-placement-validation/increment-a/` (`evidence/`),
commit `017db35e2`. Metal runs were ordinary `cargo xtask metal run --` on host
kernel `7.0.0-38-generic`, `rustc 1.95.0`. The V-1 macro was copied and
extended in the copy.

| Item | Verdict |
|---|---|
| V-2 stacking and coverage | WORKS, with three rules for DDD-1 |
| V-3 per-test variants | WORKS: 20 named environments over 8 groups |
| V-6 kernel command-line marker | WORKS |
| V-7 off-Metal lanes | inventory complete |
| V-9 dev-dependency edges | no gate rejects them; two rule-text findings |

## V-2 — stacking and which tests cannot take the attribute

Hypothesis: `#[overdrive_test(...)]` stacks over `tokio::test`, `proptest!`,
`quint_test`/`quint_run`, `serial` and trybuild tests with one libtest test and
one record each. Falsified by a stacking that registers a test twice, loses its
record, or does not compile.

**1. "Insert `#[test]` when absent" doubles tests.** Four stackings run twice
(`evidence/v2-running-counts.txt`):

```text
running 2  proptest_stack::p_test_then_ours     (#[test] above ours, inside proptest!)
running 2  tokio_stack::should_panic_expected   (#[test] #[should_panic] above ours)
running 2  quint_stack::quint_run_then_ours     (#[quint_run] above ours)
running 2  quint_stack::quint_test_then_ours    (#[quint_test] above ours)
running 1  every other stacking
```

A `#[test]` written above is expanded first and consumed; `quint_test` and
`quint_run` emit `#[test]` before the attributes below them. Each test still
has exactly one record (17 records for 17 tests).

**2. Ours above `#[quint_*]` cannot compile:** `error: tests must return ()`.
The quint macros take `-> impl Driver` and replace the fn with a `()` fn
(`quint-connect-macros-0.1.2/src/internal/test.rs:59-85`). A `Result`-returning
test is rejected by the same rule (`evidence/v2-neg-*-extract.txt`).

**3. Two forms stack in every order** (31/31 and 11/11 passed, `running 1`
everywhere): `record_only` (fn untouched, record is a sibling `static` in an
anonymous const) and `strict` (the body-wrapping guest gate, including the early
`return;` of the hand-off, with no `#[test]` insertion).

| Stacking | `record_only` | `strict` |
|---|---|---|
| ours above `#[quint_test]` / `#[quint_run]` | 1 test, passes | rejected: `()` rule |
| fn returning `Result` | 1 test | rejected: `()` rule |
| `#[ignore]` | record kept, test skipped | not run |
| trybuild `pass` test | not run in this form (PASS under V-1 behaviour) | not run |
| `serial` | not run in this form (1 test either order under V-1 behaviour) | not run |

**4. Workspace count** (text-level scan, `v2-count.py`): 4,672 test-introducing
sites.

| Site | Count |
|---|---|
| `#[test]` | 2,360 |
| `#[tokio::test]` | 1,560 |
| inside `proptest!` | 400 (398 write `#[test]` inside the block) |
| `#[quint_test]` | 328 |
| `#[quint_run]` | 24 |
| with `#[serial]` | 176 |
| with `#[ignore]` | 28 |
| with `#[cfg]` on the fn | 23 |
| with `#[should_panic]` | 7 |
| already `overdrive_test` | 1 |

Cannot take the attribute as the V-1 macro is written:

1. 6 host-pure tests returning `Result`:
   `overdrive-dataplane/tests/integration/build_rs_artifact_check.rs:29`,
   `overdrive-worker/src/probe_runner/tcp_prober.rs:210,219`,
   `overdrive-worker/src/probe_runner/http_prober.rs:528,537`,
   `xtask/src/metal_run_isolation_conformance.rs:1410`.
2. 352 `quint_*` tests; 5 are guest-placed (`overdrive-gateway`
   `application::quint_gateway_application::*`).
3. One `macro_rules!` template generating 18 tests
   (`gateway_relay_equivalence.rs:616`, `equivalence_cases!`): one attribute in
   the template places all 18, but DDD-15's "stop on a macro-generated test"
   fires.

Under `record_only` or `strict`, no test is unable to take the attribute. The
workspace uses no `rstest`, `test_case`, `libtest-mimic` or `harness = false`.

Side finding: 9 non-host-pure table entries match no `fn <name>` in the repo (6
in `overdrive-control-plane::integration` look stale, 3 belong to a fixture
crate).

**Rules for DDD-1:** (1) the macro never inserts `#[test]` — all 4,672 sites
already carry a test attribute, and order then stops mattering; (2) the
attribute goes below `#[quint_*]`; (3) the `()`-return rule applies to the guest
(body-wrapping) forms only; (4) the 18-test template needs a ruling.

## V-3 — the dispatcher's per-test variants

Method: `v3-inventory.py` re-implements the nextest per-test selection in
`guest_suite.rs` (`selected_cell` `:3788`, `nextest_guest_plan` `:4076`,
`nested_resources` `:4043`) over all 818 guest-group table entries. Every test
lands in exactly one environment.

Sources of variation: 13 `Cell` tables, `NESTED_GUEST_LAUNCH` (6 prefixes) and
`NESTED_GUEST_RUSTC`, the mtls-relay prefix split (`:196`), the literal
`DECLARED_LISTENER_TCP_CAPTURE_TEST` (`:379`), `focused_selection`, and
`QUINT_GUEST_MODULES` (`:180`, keyed by binary and module prefix; stages the
Quint runtime and a model file for 5 gateway tests). Two
`CONTROL_PLANE_NORMAL_NATIVE` rows match no placed test
(`workload_netns_provision::provision_creates_and_idempotently_converges_per_workload_netns`,
`…::provision_injects_node_local_responder_into_netns_resolv_conf`).

Group constants: CID slots probe 7, core 8, unload 9, ulp 10, stress 11,
nested-boundary 12, mtls-relay 13, cgroup-default 6. RAM 4096 MiB except
`nested-cohort` 12,821. vCPUs 2 (probe, core, unload, ulp, mtls-relay), 8
(nested-boundary, every cgroup-default profile), 16 (stress). Every
cgroup-default profile raises `RLIMIT_NOFILE`.

Proposed profile table (818 tests):

| Group | Profile | Tests | Settings |
|---|---|---|---|
| probe | `base` | 6 | tools cat cp echo ip nft sleep ss; no modules |
| core | `base` | 85 | probe tools; modules nfnetlink nf_tables nf_reject_ipv4/6 nft_reject nft_reject_inet tls |
| unload | `base` | 4 | forwarder module only |
| ulp | `base` | 1 | tls module loaded |
| stress | `base` | 1 | probe tools; 16 vCPU |
| nested-boundary | `base` | 7 | nested substrate (as `nested`), 8 vCPU |
| mtls-relay | `native` (default) | 124 | 20 dataplane tools, dataplane lib test binary, BPF object, python stdlib archive, tcpdump account files, tls + 17 native modules; boot loads the native modules then `DATAPLANE_NATIVE_SETUP` |
| mtls-relay | `relay` | 8 | same staging; only tls loaded; no native setup (`integration::mtls_inbound_guest_relay::`) |
| cgroup-default | `base` | 484 | 14 `CGROUP_DEFAULT_TOOLS` + sleep + forwarder module; cgroup-default module closure; setup loads modules, removes the auto `dummy0`, chmods `/dev/kvm` and `/dev/vhost-vsock`, mounts cgroup2 + bpf, sets controllers, links `/var/run` |
| cgroup-default | `dummy-link` | 16 | `base` + dummy module |
| cgroup-default | `host-route` | 2 | `dummy-link` + route setup before and cleanup after; cleanup failure fails a passing test |
| cgroup-default | `legb-tools` | 2 | `base` + sysctl |
| cgroup-default | `legb-audit` | 3 | `legb-tools` + LEGB setup and audit before, audit after |
| cgroup-default | `routed-route` | 3 | `base` + routed-route setup and cleanup |
| cgroup-default | `normal-gate` | 16 | `base` + ethtool setsid sysctl + empty `/etc/resolv.conf` + `CONTROL_PLANE_NORMAL_NATIVE_SETUP` before, `…_CLEANUP` after |
| cgroup-default | `nested` | 12 | 22 tools + ldd bash sleep systemd-detect-virt, forwarder module, BPF object, 10 guest-kit module sources, empty resolv.conf, reflink XFS volume, prebuilt init, kernel, cloud-hypervisor, xfs + cgroup closure modules; attempt prep loop-mounts XFS at `/srv/vm`, mounts cgroup2/bpf/securityfs, sets controllers, links `/var/run`, chmods device nodes |
| cgroup-default | `nested-cohort` | 2 | `nested` with 12,821 MiB |
| cgroup-default | `nested-launch` | 16 | `nested` + VMM at `/usr/local/bin/cloud-hypervisor`, which/umount/busybox/sh, `/etc/passwd`, `PATH=/usr/local/bin:/usr/bin:/bin`, node setup script |
| cgroup-default | `nested-launch-rustc` | 13 | `nested-launch` + Rust sysroot, `cc`, `collect2`, `ld`, 15 more tools, gateway example and guest sources; `PATH` starts with the sysroot `bin` |
| cgroup-default | `nested-launch-lane` | 12 | `nested-launch` + prepared `overdrive-init` test binary (`integration::in_guest_lane::`) |

The 1-test `declared-listener-diagnostic` variant only prints
`nft --version; uname -r`, and nft is staged in every cgroup-default guest, so
it folds into `base` (counted there). The lane kinds keep their own
`INIT_GUEST_LANE_PLACEMENTS`. The Quint model file is orthogonal to the
profile: declare it on the test (it already names its spec) or keep it as
library data.

## V-6 — the kernel command-line marker

Two guests booted through the production `GuestKit::boot_scripted_with`, run
`6b40f716fb94c4839f42ef9c`:

```text
V6 HOST_CMDLINE BOOT_IMAGE=/vmlinuz-7.0.0-38-generic root=UUID=… ro console=tty1 console=ttyS0
V6 HOST_HAS_TOKEN false
V6 [tok] V6_GUEST_CMDLINE console=ttyS0 panic=1 root=/dev/vda rw overdrive.testpeer=802776137 overdrive.test_guest=1
V6 [tok] V6_GUEST_HAS_TOKEN true
V6 [ctl] V6_GUEST_CMDLINE console=ttyS0 panic=1 root=/dev/vda rw overdrive.testpeer=802776138
V6 [ctl] V6_GUEST_HAS_TOKEN false
PASS [4.081s] v6_cmdline token_reaches_guest_and_not_host
```

The `FWD_GUEST_BOUNDARY` CID band has 16 slots (slots above 15 panic); the
probe used 13 and 14. The nested case was not run.

## V-7 — lanes that run guest-placed tests off Metal

Off Metal the wrapper `exec`s the original command, so a guest-placed body runs
on the host and passes. Feature gating (`v7-gating.py`, 807 of 834 matched to a
source fn):

| Features needed to compile | Guest-placed tests |
|---|---|
| none | 13: `overdrive-dataplane` lib 2, `overdrive-gateway` lib 5, `overdrive-sim::acceptance` 6 |
| `integration-tests` | 654 (+18 macro-generated in the dataplane integration binary) |
| `integration-tests` + `kvm-tests` | 140: cli 78, fwd 13, host 19, control-plane 7, init 23 |

| # | Lane | Runs on | Command | Guest-placed tests run today | Under fail-closed |
|---|---|---|---|---|---|
| 1 | CI `nextest + doctests` | runner → Lima | `cargo nextest run --workspace --locked --profile ci` (`ci.yml:290-311`) | 13 | fail |
| 2 | CI `nextest (integration-tests)` | runner → Lima | `… --features integration-tests --exclude overdrive-bpf -E 'binary(integration) \| …'` (`ci.yml:442-466`) | ≈614 | fail |
| 3 | CI `bpf-unit` | runner → Lima | `cargo xtask bpf-unit` → `cargo nextest run -p overdrive-bpf --features integration-tests --test integration` (`ci.yml:1171-1241`) | 23 | fail |
| 4 | pre-commit `nextest-affected` | Metal if `.env` sets `OVERDRIVE_METAL_TARGET`, else Lima | `infra/hooks/linux-run.sh cargo nextest run -E rdeps(…)` (`lefthook.yml:194-196`) | ≤13 | fail on Lima fallback only |
| 5 | pre-push `nextest` | same routing | `linux-run.sh cargo nextest run --workspace --features integration-tests` (`lefthook.yml:267-268`) | ≈685 | fine on Metal; fail on Lima fallback |
| 6 | `cargo xtask ci` | invoking host directly | `cargo nextest run --workspace --all-targets` (`xtask main.rs:1692-1693`) | 13 | fail |
| 7 | nightly `mutants-workspace` | runner → Lima | `cargo xtask mutants --workspace --features integration-tests` (`nightly.yml:79-83`) | ≈685 | baseline fails, lane yields nothing |
| 8 | per-PR `mutants` | commented out (`ci.yml:836-982`) | — | 0 | — |
| 9 | developer Lima inner loop | `cargo xtask lima run -- cargo nextest …` | as chosen | ≤≈685 | fail |
| 10 | Metal ordinary run | native host | `integration-tests,kvm-tests` | 834 | works |

No off-Metal lane enables `kvm-tests`. Under fail-closed, lanes 1, 2, 3, 6 and
7 turn red, and 4, 5 and 9 when they fall back to Lima. Lanes 1, 4 and 6 fail
only because of the 13 feature-free tests, which could be re-placed host-pure or
filtered. Lane 2 (≈614 tests) cannot be filtered (guest-placed and host-pure
tests share binaries); moving it to Metal matches the run-everything-on-Metal
direction. Running in place keeps every lane green but reports PASS for tests
that never ran in a guest; nextest has no runtime skip.

## V-9 — dev-dependency edges onto the macro crate

A throwaway worktree added `overdrive-testing-macros` as a dev-dependency of
`xtask` and `overdrive-core`; `cargo xtask` gates ran on Metal, `cargo tree` and
`cargo deny` locally (not in the ordinary grammar).

| Gate | Baseline | With the edges |
|---|---|---|
| `dst-lint` | exit 0 | exit 0 |
| `yaml-free-cli` | not run | exit 0 |
| `cloexec-lint` | exit 1 (`overdrive-dataplane/src/gateway_relay/sys.rs:87:31`) | same |
| `conformance-lint` | exit 1 (`overdrive-testing/src/guest_suite.rs:183:5 [spec-without-quint-connect]`) | same |
| `cargo tree -p xtask \| rg overdrive-` | empty | one hit: `overdrive-testing-macros v0.0.0 (proc-macro)` |
| `cargo tree -p xtask -e normal,build \| rg overdrive-` | empty | empty |
| `cargo tree -p overdrive-core -e normal,build \| rg overdrive-testing-macros` | empty | empty |
| `cargo deny check bans sources` | fails (17 wildcard errors, `anyhow` banned) | same class; wildcard count core 2→4, xtask 0→1 |

1. The rule's literal command (`development.md`: `cargo tree -p xtask | rg
   overdrive-` must be empty) includes dev edges and flags the edge; the
   automated graph gates read normal and build edges only. If approved, the
   documented command becomes `-e normal,build`.
2. `cargo deny` runs in no lane and already fails at baseline.
3. Guest forms of the macro expand to `::overdrive_testing::guest_suite::run_in_guest`,
   so xtask's 1 guest-group and 5 native-launcher tests also need
   `overdrive-testing` (which pulls `overdrive-core`, `overdrive-fwd`,
   `overdrive-host`; none has a `build.rs` and none reaches
   `overdrive-dataplane`, so the bootstrap deadlock does not recur). The other
   392 xtask tests need only the macro crate.
4. The macro crate has no `build.rs`, depends only on `proc-macro2`, `quote`,
   `syn`, and creates no cycle.

Also found: `cloexec-lint` and `conformance-lint` already fail on this branch's
base (lines above), independent of these edges.

## Design implications

1. DDD-1: no `#[test]` insertion; attribute below `#[quint_*]`; `()`-return rule
   for guest forms only; a ruling on the 18-test `macro_rules!` template.
2. DDD-5: adopt the 20-environment profile table; decide how the Quint model
   file is declared.
3. DDD-10: `overdrive.test_guest=1` works as designed.
4. DDD-11: the lane table above; fail-closed needs lanes 1, 2, 3, 6, 7 handled.
5. DDD-12: amend the rule text to `-e normal,build`; decide whether xtask also
   takes `overdrive-testing`.

## Not run

The V-6 nested case; `rstest`/`test_case` (unused); `cargo check` of the
`overdrive-core` and `xtask` test targets with the new edges.
