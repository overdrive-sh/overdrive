# DELIVER Review — Step 08-02

## Metadata

| Field | Value |
|---|---|
| Feature | `netns-density-295` |
| Step | `08-02` — Boot member convergence and the killed-mode proof |
| Reviewer | `nw-software-crafter-reviewer` |
| Model | GPT-6 Luna, maximum reasoning (explicit user override) |
| Iteration | 1 |
| Commits reviewed | `c91a737c6326fa54e77235d262e5ca7b6f218f8c..cb150cf9ef340e440c6e29034a08ed86c6626831`; implementation `f21b9cf256e5158211987608fa2721fcd69b7f3b`; log-only commit `cb150cf9ef340e440c6e29034a08ed86c6626831` |
| Verdict | **APPROVED** |

The review applied `nw-sc-review-dimensions`, `nw-tdd-review-enforcement`, and `nw-tdd-methodology`.

## Scope and contract sources

Reviewed the implementation against roadmap step 08-02 (`roadmap.json:1287-1332`), the accepted R12 driven-port contract (`feature-delta.md:3332-3550`), the exact boot ordering (`feature-delta.md:5292-5340`), the typed boot/audit error mapping (`feature-delta.md:3888-3957`), ADR-0137, ADR-0139, and scenarios S-ND295-13A/B/C/D and 71 (`distill/test-scenarios.md:1133-1206, 1383-1417`). The new Service+Vm reclamation path was checked against brief §SD-1, which requires boot to reap every VM allocation with surviving host state and states that VM allocations are never adopted (`brief.md:125-155, 279-310`).

The reviewed boot path is the ordinary production composition: `run_server` calls `vm_reclamation_boot::converge`, then `sweep_stale`, then shared-network convergence, then `MtlsInterceptWorker::start_shared_owner` (`crates/overdrive-control-plane/src/lib.rs:7913-7991`). The worker's fresh-process owner path clears members before observing the prior IP program, converges fresh listeners, and reads the full state before creating listener tasks or publishing the owner (`crates/overdrive-worker/src/mtls_intercept_worker.rs:2143-2275`). The review did not evaluate later 08-03 audit work.

## Contract Shape Compliance

The public and cross-crate interface shape matches the selected design. This commit implements the already-approved `MtlsIntercept::converge_allocation_elements` and `observe_shared_state` methods and the already-approved doc-hidden `converge_shared_ip_intercept_members_atomically` operation. It removes the superseded `clear_shared_ip_intercept_elements_atomically` function and its private clear helper as the current R12 contract requires (`feature-delta.md:3481-3510`). No public method, type, enum variant, parameter, persisted field, wire format, or ownership boundary was added. The `vm_reclamation` change keeps the existing helper signature and changes only which existing workload intents feed its desired-side join.

The host's observed-identity fallback does not weaken the canonical identity proof. `observe_shared_ip_intercept_state` uses `collect_state`/`collect_once`; `SharedProgram::from_components` calls `validate_components`, which checks the exact owned table/chains/sets/rule count and order, per-rule ownership data, canonical expressions, and both nonzero listener targets (`crates/overdrive-netlink/src/nft.rs:3425-3447, 3628-3686, 3872-3975`). `converge_allocation_elements` takes that observed identity only when the process has no recorded identity, then `state_for` performs a second strict observation against it before the batch (`nft.rs:4463-4473, 4621-4674`). The program shape is therefore independently validated by the host observer before the identity is reused as the expected value; it is not an unchecked equality of a value with itself. The Sim adapter uses its own modeled program identity and state transition, as required for a separate simulation adapter (`crates/overdrive-sim/src/adapters/mtls_intercept.rs:471-484, 788-801`).

The designed semantic boot errors are pinned and implemented as follows. The adapter's underlying operational cause remains the approved cause-preserving `NetlinkError` source (`ADR-0122`; `InterceptError::NftRuleInstallFailed` carries its operation and typed source at `crates/overdrive-worker/src/mtls_intercept.rs:272-284`). Callers branch on the following approved worker-level distinctions:

| Designed failure | Approved representation | Implementation |
|---|---|---|
| Boot 6.2 member convergence returns `Err(e)` | `MtlsSharedOwnerError::BootMemberClear { source: e }` | `clear_boot_members` preserves the returned `InterceptError` as the source (`mtls_intercept_worker.rs:2222-2235`). The host adapter maps nft convergence failure to `InterceptError::NftRuleInstallFailed { op: "converge-shared-members", source }` (`mtls_intercept_port.rs:1222-1257`). |
| Boot 6.2 returns `Ok(Some(state))` with nonempty members | `BootMemberClear { source: InterceptError::MembersRemain { observed } }` | Exact observed members are retained (`mtls_intercept_worker.rs:2228-2234`). |
| Boot 6.2 returns `Ok(None)` because the owned table is absent | No failure; no member write occurs | Host and Sim both return `None`; the later 6.6 full read-back still requires the newly converged program (`mtls_intercept_port.rs:1227-1233`; `overdrive-sim/src/adapters/mtls_intercept.rs:792-801`). |
| Boot 6.6 `observe_shared_state` returns an error | `MtlsSharedOwnerError::Intercept { source: e }` | The worker preserves the source (`mtls_intercept_worker.rs:2242-2245`). Host program, route, and guard read failures use the accepted `NftRuleInstallFailed`, `IpRuleAddFailed`, or `IpRouteLocalAddFailed` leaves with their typed source (`mtls_intercept_port.rs:1194-1219`; `mtls_intercept.rs:1023-1033`). |
| Boot 6.6 program is absent or differs from the converged identity | `Intercept { source: PostconditionMismatch { expected, observed } }` | Expected and observed identities are retained (`mtls_intercept_worker.rs:2246-2258`). |
| Boot 6.6 policy route is absent | `Intercept { source: PolicyRouteAbsent }` | Exact source-less semantic cause (`mtls_intercept_worker.rs:2260-2263`). |
| Boot 6.6 R18 guard is absent | `Intercept { source: InterceptMarkGuardAbsent }` | Exact source-less semantic cause (`mtls_intercept_worker.rs:2265-2268`). |
| Boot 6.6 read-back has nonempty members | `BootMemberClear { source: MembersRemain { observed } }` | Exact observed members are retained (`mtls_intercept_worker.rs:2270-2274`). |

This matches the accepted error table (`feature-delta.md:3913-3957`). `MtlsSharedOwnerError::component()` remains the explicitly deferred 08-03 surface; this step does not assert or implement it (`distill/test-scenarios.md:1206, 1250`). No required semantic distinction is encoded only in a diagnostic string, and no generic carrier or new variant is substituted for a designed cause.

The whole-state member mutation is one netlink element transaction followed by generation-consistent state read-back. A read-back mismatch or failure takes the existing inverse transition and verifies restoration (`nft.rs:4475-4522, 4621-4674`). The strict collector rejects noncanonical or foreign owned state before mutation. This preserves the accepted program and outside-table complements.

## Test integrity, authored bodies, and coverage

All bodies named by the step are active. S-ND295-13B was already active at the reviewed base; the unchanged authored body drives ordinary `run_server` and asserts exactly the four boot phase events (`shared_guest_network_startup.rs:702-740`). S13A, S13C, the two worker-level S13D bodies, the three composed S13D refusal bodies, and the two member-aware S71 equivalence bodies were activated by removing only their pending markers. The source diff contains no deleted test, reduced case set, lowered timeout/window, skipped assertion, or reduced assertion. The new Service+Vm regression is also test-only evidence for the required boot recovery path.

Every changed or activated body has its accepted `Outcome anchor` and `CONTRACT_SHAPE: bounded-change.` declaration. The source-local S13A property exercises production composition, not a pure function; the new Service+Vm body is also a composed simulation invariant, so neither requires the special `CONTRACT_SHAPE: pure-function.` declaration. The contract-shape CLI path named by the reviewer skill is not present in this checkout; declarations were checked directly in every affected body. None of the changed test names matches the review skill's banned `test_.*` name patterns.

The step covers eleven required composed, worker-port, and native bodies across S13A/B/C/D, S71, and the additional Service+Vm regression; it adds no new unit-level test body, so the `2 × distinct unit behaviors` budget is not increased. L1/L2 review found no dead production code or speculative abstraction in the bounded source changes. The additional VM reclamation code is directly traced to the R12/S13C outcome below.

The changes to S13C's rule-order oracle correctly reconcile it with the accepted retained mark → TPROXY → accept order after R19's reorder was withdrawn. The matcher requires exactly two TPROXY rules, mark value numerically equal to `1`, then TPROXY, then accept; V4a retains the independent typed canonical-identity check. This corrects the stale R19 oracle without weakening the current invariant or changing the timing/sample windows. Capturing successful boot-two `describe` results adds diagnosis without changing the row predicate. The `OVERDRIVE_TEST_NFT_MONITOR_BIN` override is read only by `NftMonitor::start`; every other `nft` command still uses the host executable. M0's stderr, early-exit, and reported-loss conditions are unchanged (`serve_killed_restart_boot_clear.rs:631-685, 1585-1600`).

The worker source-local success fixture now reports the already-required R18 guard as present; it does not alter a failure script or assertion (`mtls_intercept_worker.rs:3580-3605`). The one existing worker call-count expectation changed from four to five because boot now makes the specified member-convergence call, strengthening the observable journal assertion (`netns_density_shared_owner.rs:83-91`). The composed S13D bodies use a recording port over `SimMtlsIntercept` and the real `run_server_with_obs_and_driver` path; the late-commit body changes only the test double after the production call has returned and proves that no later intercept call or guest attachment occurs (`boot_member_clear_refusal.rs:195-348, 551-605`). No test spawns the built Overdrive binary.

### Service+Vm reclamation was necessary to satisfy R12

The extra `crates/overdrive-reconcilers/src/vm_reclamation.rs` change is required by the accepted “every VM allocation with surviving host state” boot-reap outcome, not adjacent hardening. S13C's real killed-mode case is a Running Service VM. Before the change, `hydrate_vm_reclamation_desired` accepted only `WorkloadIntent::Job`, so the persisted Service VM produced no `VmAllocFacts`; the no-entry arm in `plan_reclamation` could discard VM-exclusive artifacts but could not author the predecessor's `PlatformReclaimed` ending or trigger replacement. A fresh Service lifecycle still sees the prior Running row and emits no replacement start.

The failure was reproduced in the bounded S13C native run and, as required for a control-plane restart/convergence claim, in the seeded `overdrive-sim` invariant through `run_server_with_obs_and_driver`, seed `0`, before the remedy. The RED receipt `7b40a7dd-4089-4d79-bd86-50176d580609` records the old row still Running, no `PlatformReclaimed` ending, and no replacement. After the join includes VM-backed `Service` alongside VM-backed `Job`, the same invariant passes (`106668af-80cd-405d-8d18-568ceb94dcaa`, 0.129 s). `Schedule` remains excluded because it has no direct driver. The implementation changes no public shape, and the existing join/plan/executor remain the only owner path (`vm_reclamation.rs:328-375`; `vm_reclamation_boot.rs:78-151`; `action_shim/reclamation.rs:168-278`).

## External monitor evidence and reproduction

The native S13C receipt is qualified by a reproduced external-tool defect. The physical host used stock nftables 1.1.6 (`nftables v1.1.6 (Commodore Bullmoose #7)`). On `DELRULE`, its monitor evicts referenced named sets from its local cache, so subsequent real set-element notifications cannot be rendered. The stock monitor emitted `Unable to cache set_elem. Set not found` / `Received event for an unknown set`, while the final kernel set contained `192.0.2.2`; the monitor lacked the corresponding add event. The exact complement was restored. This is an observer failure, not a missing kernel mutation.

The bounded monitor-only patch changes the named-set cache callback in upstream `src/monitor.c`; it does not filter or fabricate notifications. The callback after the patch is:

```c
static void netlink_events_cache_delset_cb(struct set *s,
                                           void *data)
{
    if (!set_is_anonymous(s->flags))
        return;

    set_cache_del(s);
    set_free(s);
}
```

The patch preserves anonymous-set cleanup and leaves the kernel, product binaries, test assertions, transcript parser, M0 stderr/loss gates, and sampling windows unchanged. The test selects this binary only for `nft monitor`; the test's other nft commands still resolve the system `nft`.

### Pinned source and private build

The source is the [official nftables 1.1.6 archive](https://www.netfilter.org/projects/nftables/files/nftables-1.1.6.tar.xz), SHA-256 `372931bda8556b310636a2f9020adc710f9bab66f47efe0ce90bff800ac2530c`. The exact three-line patch SHA-256 is `303e640c984f9faca6a65c4866e169f92b908894467f054ae8db427c22d06349`; patched `src/monitor.c` SHA-256 is `b11416f3d6b8ce4dfaf14dc2803589bd709a08e8de307fc287f839062ed32bc8`.

Development dependencies were privately downloaded and extracted under `/var/tmp/nd295-08-02-nft-monitor`; no system installation or global PATH change was made:

| Package | Version | SHA-256 |
|---|---|---|
| `libmnl-dev_1.0.5-3build1_amd64.deb` | `1.0.5-3build1` | `36cc184ecc61e043b6cfd8d3b6f9627f0c839e72b95b13b6ac9672ebf86f1f1e` |
| `libnftnl-dev_1.3.1-1_amd64.deb` | `1.3.1-1` | `11562c493f207fcc97e1d0df87d7ab5ddbecad511e8626e62547990728e4d6ef` |

Reproduction build, on qualified Linux x86_64 metal with the matching installed runtime packages (`libmnl0 1.0.5-3build1`, `libnftnl11 1.3.1-1`):

```sh
BASE=/var/tmp/nd295-08-02-nft-monitor
mkdir -p "$BASE/deps"
cd "$BASE"
curl -fLO https://www.netfilter.org/projects/nftables/files/nftables-1.1.6.tar.xz
printf '%s  %s\n' '372931bda8556b310636a2f9020adc710f9bab66f47efe0ce90bff800ac2530c' nftables-1.1.6.tar.xz | sha256sum -c -
apt-get download libmnl-dev=1.0.5-3build1 libnftnl-dev=1.3.1-1
printf '%s  %s\n' '36cc184ecc61e043b6cfd8d3b6f9627f0c839e72b95b13b6ac9672ebf86f1f1e' libmnl-dev_1.0.5-3build1_amd64.deb | sha256sum -c -
printf '%s  %s\n' '11562c493f207fcc97e1d0df87d7ab5ddbecad511e8626e62547990728e4d6ef' libnftnl-dev_1.3.1-1_amd64.deb | sha256sum -c -
dpkg-deb -x libmnl-dev_1.0.5-3build1_amd64.deb "$BASE/deps"
dpkg-deb -x libnftnl-dev_1.3.1-1_amd64.deb "$BASE/deps"
mkdir -p "$BASE/deps/usr/lib/x86_64-linux-gnu"
ln -sfn /usr/lib/x86_64-linux-gnu/libmnl.so.0 "$BASE/deps/usr/lib/x86_64-linux-gnu/libmnl.so"
ln -sfn /usr/lib/x86_64-linux-gnu/libnftnl.so.11 "$BASE/deps/usr/lib/x86_64-linux-gnu/libnftnl.so"
tar -xf nftables-1.1.6.tar.xz
cd "$BASE/nftables-1.1.6"
# Apply the three-line callback patch above to src/monitor.c.
PKG_CONFIG_LIBDIR="$BASE/deps/usr/lib/x86_64-linux-gnu/pkgconfig" \
PKG_CONFIG_SYSROOT_DIR="$BASE/deps" \
./configure --prefix="$BASE/prefix" --disable-man-doc --without-cli --without-json --with-mini-gmp
make -j4
make install
"$BASE/prefix/sbin/nft" --version
sha256sum "$BASE/prefix/sbin/nft" "$BASE/prefix/lib/libnftables.so.1.1.0"
ldd "$BASE/prefix/sbin/nft"
dpkg -V nftables libnftables1 libmnl0 libnftnl11
```

The private `prefix/sbin/nft` SHA-256 is `6e83c5a3448dba400328d2423731230ece0a1dc302344e5598c3566faba1678c`; the private `libnftables.so.1.1.0` SHA-256 is `1a03259f48c7b1ec5dc868a9465f8ea73c7bafc434a70b02235702da84deb59b`. Its loader uses the private `libnftables` and the original system `libmnl`/`libnftnl` runtime libraries. `dpkg -V nftables libnftables1 libmnl0 libnftnl11` returned 0 with empty output. The build host reported GCC 15.2.0 and GNU Make 4.4.1.

### Scratch spike and native receipt

The before/after scratch spike uses only an isolated `inet` table, a named IPv4 set, and a lookup rule. It snapshots `nft list tables`, starts the selected monitor, deletes and re-adds the referencing rule, adds `192.0.2.2`, reads the actual set, stops the monitor, and deletes both scratch tables in `finally`. The stock run proves `192.0.2.2` exists in the kernel set while `monitor_add_event_present=False`, prints the unknown-set warnings, and returns `table_complement_restored=True`. Repeating the identical stimulus with `OVERDRIVE_TEST_NFT_MONITOR_BIN=/var/tmp/nd295-08-02-nft-monitor/prefix/sbin/nft` prints the real `add element inet … ips { 192.0.2.2 }` notification, empty stderr, no loss report, and `table_complement_restored=True`.

The exact scratch stimulus is:

```python
import os
import re
import subprocess
import time

table = f"ovd_monitor_cache_spike_{os.getpid()}"
probe = table + "_ready"
before = subprocess.check_output(["nft", "list", "tables"], text=True)
monitor = None

def nft(*args, input=None):
    return subprocess.check_output(["nft", *args], input=input, text=True)

try:
    nft("-f", "-", input=f"table inet {table} {{\n"
        " set ips { type ipv4_addr; elements = { 192.0.2.1 }; }\n"
        " chain observation { ip saddr @ips accept; }\n"
        "}\n")
    monitor_binary = os.environ.get("OVERDRIVE_TEST_NFT_MONITOR_BIN", "nft")
    monitor = subprocess.Popen(
        ["stdbuf", "-oL", "-eL", monitor_binary, "monitor"],
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
    )
    time.sleep(0.3)
    nft("add", "table", "inet", probe)
    time.sleep(0.1)
    listing = nft("-a", "list", "chain", "inet", table, "observation")
    handle = re.search(r"ip saddr @ips accept # handle (\d+)", listing).group(1)
    nft("delete", "rule", "inet", table, "observation", "handle", handle)
    nft("add", "rule", "inet", table, "observation", "ip", "saddr", "@ips", "accept")
    nft("add", "element", "inet", table, "ips", "{ 192.0.2.2 }")
    time.sleep(0.3)
    final_set = nft("list", "set", "inet", table, "ips")
    monitor.terminate()
    stdout, stderr = monitor.communicate(timeout=5)
    print("FINAL_SET\n" + final_set)
    print("MONITOR_STDOUT\n" + stdout)
    print("MONITOR_STDERR\n" + stderr)
    print("kernel_new_member_present=" + str("192.0.2.2" in final_set))
    print("monitor_add_event_present=" + str(
        f"add element inet {table} ips {{ 192.0.2.2 }}" in stdout
    ))
finally:
    if monitor is not None and monitor.poll() is None:
        monitor.terminate()
        monitor.communicate(timeout=5)
    for name in (probe, table):
        subprocess.run(["nft", "delete", "table", "inet", name],
                       stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False)
    after = subprocess.check_output(["nft", "list", "tables"], text=True)
    print("table_complement_restored=" + str(before == after))
```

On the qualified native host, save the script as `/tmp/nd295-monitor-spike.py` and run it once with the stock monitor and once with the private monitor:

```sh
env -u OVERDRIVE_TEST_NFT_MONITOR_BIN python3 /tmp/nd295-monitor-spike.py
OVERDRIVE_TEST_NFT_MONITOR_BIN=/var/tmp/nd295-08-02-nft-monitor/prefix/sbin/nft \
  python3 /tmp/nd295-monitor-spike.py
```

Full native S13C command, with the test-only monitor selector set to the pinned private binary:

```sh
# Actual qualified run; RSYNC_BIN excludes only ignored .context scratch.
RSYNC_BIN=.context/distill-08-01-cleanup-rsync cargo xtask metal run -- env \
  OVERDRIVE_TEST_NFT_MONITOR_BIN=/var/tmp/nd295-08-02-nft-monitor/prefix/sbin/nft \
  cargo nextest run -p overdrive-cli --test integration \
  --features integration-tests,kvm-tests \
  -E 'test(serve_killed_restart_boot_clear)' --no-fail-fast

# Equivalent rerun when the local scratch-exclusion helper is unavailable:
cargo xtask metal run -- env \
  OVERDRIVE_TEST_NFT_MONITOR_BIN=/var/tmp/nd295-08-02-nft-monitor/prefix/sbin/nft \
  cargo nextest run -p overdrive-cli --test integration \
  --features integration-tests,kvm-tests \
  -E 'test(serve_killed_restart_boot_clear)' --no-fail-fast
```

Qualified final receipt `b059a266-00e1-44d4-a318-97d1f39b07fa`: 1 passed, 174 skipped; 7.790 s. All twelve S13C oracles are GREEN: M0 (empty stderr, no reported event loss, monitor alive until stop); V0 restart returns `Ok`; V1 Cloud Hypervisor PIDs and scope are gone before boot-two nft changes; V2 one batch deletes exactly the three stale members before any program mutation; V3 typed member read-back is empty after clear; V4a exact canonical identity/order read-back after the last program batch and before admission; V4b route present after clear; V4c R18 guard present after clear; V5 boot opens admission; V6 replay equals final kernel state; A1 first fresh lease is `100.95.0.2`; A2 the predecessor TAP, TCX link pins, endpoint entry, and bridge-guard member are absent before admission. The observed first admission batch is the real kernel batch adding the successor's managed/outbound `100.95.0.2` members. Cleanup stopped the replacement, interrupted boot two, and restored the exact host complement; proof processes/scopes/run directories/pins/dynamic members are absent.

The stock host `nft` monitor remains broken for this DELRULE/set-cache sequence; the reviewed native pass used the patched monitor selection above. This review does not claim that the default stock-monitor invocation passes. The patch changes only the external monitor's cache bookkeeping, not the kernel notification stream, product behavior, or the test's loss gate.

## Verification and DES

The retained run receipts report:

| Evidence | Result |
|---|---|
| S13A default proptest count | 256 cases; 28.026 s (`7b1b9aa1-d42b-484a-b890-cb97da8bf2cf`) |
| Service+Vm seed-0 invariant against prior implementation | RED: seed `0`, prior remains Running, no `PlatformReclaimed` ending or replacement (`7b40a7dd-4089-4d79-bd86-50176d580609`, 3.07 s) |
| Service+Vm invariant after implementation | PASS (`106668af-80cd-405d-8d18-568ceb94dcaa`, 0.129 s) |
| S13B plus three composed S13D bodies | 4/4 PASS (`ba8b0bdd-3a17-4d72-b215-8cd927c36d3d`) |
| Worker integration selectors | 24/24 PASS (`fbe2ee0c-d560-44b5-9c7f-41e250a195cb`) |
| Worker unit suite after the R18 success-fixture update | 80 passed, 1 skipped (`e0b1c372-67d6-42cb-8225-bc7b1cfc2bd4`) |
| Native S13C | 1 passed, 174 skipped; all 12 oracles GREEN (`b059a266-00e1-44d4-a318-97d1f39b07fa`) |
| Workspace gates | PASS as reported: `cargo xtask lima run -- cargo check --workspace --all-targets --features integration-tests`; `cargo xtask lima run -- cargo clippy --workspace --all-targets --features integration-tests -- -D warnings`; formatting and `git diff --check` |

`execution-log.json` records RED PASS at `2026-10-03T22:38:59Z`, GREEN PASS at `2026-10-03T23:57:46Z`, and COMMIT PASS at `2026-10-04T00:11:56Z`. The implementation commit retains Marcus as author, has exactly one `Co-Authored-By: Codex <codex@openai.com>` trailer, and carries `Step-Id: 08-02`. The log-only commit records COMMIT. No mutation test was run; step-level mutation testing is prohibited. This reviewer inspected the scoped code and receipts and did not rerun suites.

## Findings and remediation dispositions

No proven in-scope defect, API divergence, typed-error ambiguity, test-integrity violation, invariant weakening, or production-boundary violation was found. The Service+Vm reclamation change is necessary to make the accepted all-VM boot reap true for the exact Service VM used by S13C, and its failure was reproduced through both the existing simulated production composition and the native killed-mode path. No remediation was required or dispatched.

The external stock-monitor limitation is recorded as an evidence qualification, not a product finding: the native acceptance proof used a pinned monitor-only correction whose source, patch, build steps, hashes, scratch spike, and qualified run receipt are recorded above.

## Iteration 1 verdict

**APPROVED.** Boot member convergence and full boot read-back follow the accepted R12 order and typed error mapping. The canonical host observer remains the identity authority before member mutation, Service VM reclamation is covered through the existing boot owner and action path, and the activated acceptance bodies retain their observable contracts. Native S13C passes with complete kernel event evidence under the documented monitor-only correction. No later step or final-wave gate is part of this verdict.
