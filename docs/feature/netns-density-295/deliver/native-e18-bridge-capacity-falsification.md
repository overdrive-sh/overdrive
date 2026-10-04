# Native E18 single-bridge population falsification

## Metadata and disposition

| Field | Value |
|---|---|
| Feature | `netns-density-295` |
| Date | 2026-10-04 |
| Role | Bounded DESIGN factual record and dependency/status reconciliation |
| Model | GPT 6.1 Sol, extra-high (`xhigh`) reasoning |
| Current validation | **Affected DESIGN PENDING; roadmap PENDING / NON-EXECUTABLE** |
| Stopped step | `08-04`, M-ND295-E18 capture at the recorded T1 populations |
| Replacement decision | None selected or implemented |
| Record review | Iteration 1 CHANGES_REQUESTED; documentary correction submitted, re-review pending |

The native `e18-t1-base` attempt could not provision the recorded
N=16,384 population through the real production owner on the one shared bridge.
The kernel refused a TAP's master attachment with netlink `-54` (`EXFULL`).
This contradicts the prerequisite for the T1/E18 measurement, so the repository's
Native falsification gate invalidates its dependent feasibility and measurement
validation immediately. The failure is not an audit-latency, quiescence, restore,
kill-loop, mutex-threshold, or recovery-window measurement.

This record selects no replacement topology or population. ADR-0117 explicitly
makes 16,384 a fixed placeholder cap, with no capacity basis, density promise, or
completion target. That framing is preserved. The falsified claim is that the
operative **T1 measurement profiles can reproducibly attach 16,384 live protected
guest network attachments to ADR-0114's sole node-local Linux bridge**.

## Completed native evidence

The preserved attempt is
[the raw directory](../../../../target/benchmarks/netns-density-295/E18/attempt-08-04-20261004/e18-t1-base/).
It contains nine original files. Its method records `receipt_id=M-ND295-E18`,
profile `e18-t1-base`, N=16,384, M=0, no Service TCP ports, and no VMM population.
The dispatched crafter used the canonical native metal execution path and
recorded native/lease checks and synchronized source metadata. Compiled benchmark
identity has the separate limitation under *Source correlation* below. This
DESIGN task ran no native command, population rerun, or synthetic reproduction.

| Surface | Recorded result | Limit |
|---|---|---|
| Substrate | Physical x86_64 host; `virtualization=none`; `uname -r=7.0.0-29-generic`; AMD EPYC 8024P; PowerEdge C6615; 16 logical CPUs; Cloud Hypervisor v53.0 | Actual method metadata, not an assumed kernel or host. No metal target address is recorded here. |
| Provisioning refusal | `Guest(Netlink { operation: TapAttachBridge, source: Link { op: "set-master", source: NetlinkError(ErrorMessage { code: Some(-54), ... }) } })` | Exact existing typed cause rendered in the raw completion row; no new error variant or caller interpretation is introduced. |
| Completion | `measurement_ok=false`, `cleanup_ok=true`, `cleanup_errors=[]` | A failed attempt with successful cleanup; not a completed E18 receipt. |
| Before state | Seven links; `ovd-gbr0` is present; none of the links is a bridge port or has a master | Direct independent pre-attempt inventory. The bridge is initially empty, rather than full of pre-existing allocation TAPs. |
| Preservation | Normalized `foreign-before.json` and `foreign-after.json` are equal, both SHA256 `361c090206ecb55b56df7c5de54ec50b1ee7ab2ca6200cce09158962bdcd3232` | Raw snapshots are separately retained. Equality uses the already recorded execution-counter and read-only bridge `gc_timer` normalization; it does not claim raw counters/countdowns stayed byte-equal. |
| Population/sample output | No `population-before.json`, `population-after.json`, or `receipt-complete.json`; `samples.jsonl` contains one failed completion row and no density samples | Full-population verification and sample capture were never reached. |

The first observed failure had been hidden by a separate endpoint cleanup
regression: removal of an absent endpoint returned `ENOENT`, overriding the
primary provisioning refusal during rollback. The independently executed
real-kernel regression, as recorded in the crafter's completed report,
`removing_an_absent_endpoint_preserves_the_real_kernel_complement` reproduced
that defect and passed after the existing absent-key classification was used.
The narrow correction and regression remain separate uncommitted work. Their
earlier native RED and support diagnosis are retained in
[the bounded harness report](../../../../.context/distill-08-04-harness.md),
including its *Isolated native regression (RED)* section. Fixing that already
approved absence behavior exposed the primary `TapAttachBridge` failure; it
does not repair or justify the single-bridge T1 premise.

### Source correlation

The raw method's base Git SHA is
`43a5ef753fac4dfe31547151476d1937ff287b31`. This is **not a claim that a clean
checkout of that commit was executed**. The method also records the canonical
dirty/untracked-source digest
`28efbbcf98d012531d7eea3d689553ab35e98931d6362c7267c94bac6858f092`, the raw
`executable_sha256` field
`48893b0fb21436b54619db80486e83ef39dfccaf1aefe83dfa00c02d6146e8c0`, and the
actual synchronized source-file manifest. The canonical editing-host marker and
active lease provide synchronized source correlation; remote Git state is not
its source. **The raw executable field is a utility fingerprint, not an
attestation of the benchmark executable. A benchmark executable digest is
unavailable in this receipt.**

Independent review reproduced the reason: the harness invokes the subprocess
`sha256sum /proc/self/exe`, so Linux resolves `self` in that child to the
`sha256sum` process. The utility hashes its own executable. The review's
[read-only Lima process-identity receipt](../../../../.context/review-e18-executable-digest-readonly.txt)
proves this mechanism; it does not attest the native utility's exact bytes or
recover the benchmark's missing digest. The raw field and all raw files remain
unchanged. Identical values in the earlier and new attempts do not establish
identical benchmark binaries, stale linkage, or a loaded-dependency mismatch.

The [independent DESIGN review](review-design-e18-native-falsification.md)
verified all **891 synchronized source-manifest entries** against current local
files. That proves synchronized file correspondence. It does not attest the
compiled benchmark or its loaded dependency bytes. The four relevant hashes
below were also checked by this record's author; the production-path audit
describes those synchronized sources rather than claiming recovered compiled
artifact identity.

The relevant synchronized hashes match the current files read for this bounded
path audit:

| Source file | SHA256 |
|---|---|
| `crates/overdrive-control-plane/bin/netns_density_benchmark.rs` | `c9784ab651ccb62e5ca8ab615b3ba890a8e84064e9ef4f870ecbf5da8daf7b0c` |
| `crates/overdrive-control-plane/src/guest_network.rs` | `148b08ce97fad1246c7b01ff9da318b3fd224b5c5eb893e841a467c81f521e5d` |
| `crates/overdrive-dataplane/src/guest_tcx.rs` | `f156611fb702f0568bc3d7d826cdd23df9b2c92db03c6a5dd36479768645b898` |
| `crates/overdrive-netlink/src/client.rs` | `939484fbe4069e763e7da12a7ff3f4e1d4e4148ba6b67a55de2c63a52686b453` |

| Preserved raw file | SHA256 |
|---|---|
| `method.json` | `2a2688bfae29c4cbe55c2f3e91406fa3392bfce868545e008aed32677845a664` |
| `samples.jsonl` | `cea34dc4faafb1775cabce357e0ddcb0afe4e625cca726106d2dd2f901ce320f` |
| `source-files.sha256` | `d6b1f16da614971e665f99165837ea2cb8beda8004b899e341bcace84283e6ca` |
| `prior-shared-state.json` | `8138313398bb5dd4fa2e09716a5da1cdbb5b237ccde874370631c6eba490d6c6` |
| `observations.redb` | `8e5d334372c040d3acd4ef8475dc3f71dea28e4a2ed60fa65cbf4d2719f9ccae` |
| `foreign-before.json` / `foreign-after.json` | `361c090206ecb55b56df7c5de54ec50b1ee7ab2ca6200cce09158962bdcd3232` |
| `foreign-before-raw.json` | `361ca2035e5e37a633e04bc3f6c52d35e0ed42253e4ba4ac6c72cbe0447aea6e` |
| `foreign-after-raw.json` | `3c3c06d1367fc539e1e01a9e43b3b15e4af398d9e21d6b3f3c5041d131abc37a` |

No raw file, earlier failed attempt, DES event, test, support source, or
production source was changed by this record.

## Production path and kernel interpretation

The measured support composition constructs the production
`HostSharedGuestNetworkOwner::new()` through
`e18_test_support::host_owner` (`guest_network.rs:119`). Its private
`HostGuestNetworkProvisioner` alias denotes the inherited provisioner role of
that same concrete owner (`guest_network.rs:2516`); it is not another owner.

| Current entry / owner path | Evidence and effect |
|---|---|
| Benchmark `run` → `Fixture::attach` | `netns_density_benchmark.rs:1290` first awaits attachment 0, performs its warmup quiesce/restore, then awaits each ascending index from 1 to N−1 at `:1293`. The loop has no parallel attachment or retry. |
| `Fixture::attach` → real address pool → owner `provision` | `netns_density_benchmark.rs:775` assigns an ordinary pool plan, retains it for cleanup before effects, and awaits `owner.provision` at `:781`. Each plan names the same `BRIDGE`; successful attachments then install intercept state and activate through the owner (`:800`, `:801`). |
| Owner `provision` → private allocation I/O | `guest_network.rs:3475` holds the allocation lifecycle lock, creates/observes the TAP, then awaits `allocation_io.attach_tap_to_bridge` at `:3545`; the approved mapping is `GuestNetworkError::Netlink { operation: TapAttachBridge, source }`. |
| `HostGuestNetworkAllocationIo::attach_tap_to_bridge` → `Client::set_link_master` | `guest_network.rs:2132` awaits the named TAP/master effect at `:2137`. `client.rs:584` resolves both ifindices, sets the controller, awaits the kernel result, and preserves it as `NetlinkError::Link` with operation `set-master` at `:592`. |
| Failure, rollback, and completion | `guest_network.rs:3795` awaits ordinary provisioning rollback and returns the primary error when rollback succeeds. The benchmark exits the population loop on that error, awaits fixture cleanup at `netns_density_benchmark.rs:1309`, restores prior shared state and compares the complement, and records the failed completion at `:1336`. It publishes a completed receipt only on full success at `:1346`. No forced abort, shutdown race, or fabricated production state is needed to reach the refusal. |

Linux v7.0 defines `BR_PORT_BITS=10` and `BR_MAX_PORTS=(1<<BR_PORT_BITS)`,
so there are 1,024 port-number slots. Its `find_portno` marks slot zero reserved,
marks existing ports, and returns `-EXFULL` when no slot remains; `new_nbp`
propagates that result and `br_add_if` propagates the port-creation failure.
These are exact-version primary sources:
[bridge constants](https://raw.githubusercontent.com/torvalds/linux/v7.0/net/bridge/br_private.h),
[port allocation and bridge addition](https://raw.githubusercontent.com/torvalds/linux/v7.0/net/bridge/br_if.c).
The Linux generic errno definition assigns `EXFULL` number 54, matching the
observed x86_64 netlink `-54`:
[errno definition](https://raw.githubusercontent.com/torvalds/linux/v7.0/include/uapi/asm-generic/errno.h).

**Inference, not direct inventory:** the initially empty bridge, exclusive
serial fixture, retained successful attachments, and this kernel path imply
1,023 completed protected attachments followed by failure of the 1,024th.
The raw attempt does not record the failing allocation index or an at-failure
bridge-port dump; its independent before/after inventories bracket cleanup.
The exact ordinal is therefore not presented as a directly sampled native
count, and is conditional on the synchronized source/path correspondence and
upstream interpretation; the receipt supplies no compiled benchmark digest.
The cited source is upstream v7.0, not a build-source attestation of
every downstream `7.0.0-29-generic` change. The directly recorded production
refusal alone proves this required N=16,384 attempt did not reach its population;
the primary kernel path explains the bridge-port exhaustion inference. No
new population run was used to strengthen or replace that evidence.

## Exact dependent validation scope

| Canonical clause | Falsified prerequisite and disposition |
|---|---|
| [ADR-0114](../../../product/architecture/adr-0114-node-local-shared-bridge-guest-network.md), *Decision* / *One bridge per workload* | The sole node-local bridge is not validated for the recorded T1 live population. Its density-based rejection of one bridge per workload is reopened. Bounded two-guest evidence and independent shared-owner, activation, VMM queue, and egress-security contracts are not disproved. |
| [ADR-0117](../../../product/architecture/adr-0117-initial-shared-bridge-density-target.md), *Decision* | The claim that the two N=16,384 receipts are reproducibly attachable under ADR-0114 is invalidated. The fixed placeholder's no-promise framing, cap, prefix/map sizing, and independent alternative grounds stay recorded. |
| [Feature delta](../feature-delta.md), *Requirements, Quality Attributes, and Capacity* / *Attachment-capacity contract and non-contractual connection limitation* / *CAP-295-A* | Exact T1 live TAP/TCX/guard/address/registry population, T1-BASE N/M=16,384/0 and T1-PORT4 N/M=16,384/65,536, and their cost/sweep receipts are pending on the contradicted single-bridge premise. The attachment-only boundary is preserved. |
| Feature delta, *Runtime shared-network supervisor*, *Full audit*, *Quiescence and restore latency at density*, *Evidence-lane matrix* E18, and *E18-derived bounds* downstream record | L/Q/W, kill-loop K, last-TAP-down/restore times, and the E18-derived source values cannot be obtained at the required population. The one-second audit threshold, both five-second fit checks, and the double-loss exposure restatement have no qualifying result. No value is inferred from partial population or clean cleanup. |
| Feature delta, *Runtime member audit (R15)* | Its `element_effects` hold-time validation at T1-PORT4 is pending. No hold-time sample exceeded 100 ms, so the epoch alternative's separate threshold condition has not been triggered; the mutex mechanism is not falsified by bridge-port exhaustion. |
| [ADR-0124](../../../product/architecture/adr-0124-bounded-shared-network-owner-recovery.md), *Consequences* / feature-delta bound rules | The placeholder-population measurements that restate exposure and support recovery fit are unavailable. The accepted cadence, Clock rules, outcomes, kill scope/order, restore condition, and typed failure taxonomy remain requirements. |
| [Roadmap](roadmap.json) `08-04` | Capture stopped before population verification/sample 0. Status is non-executable under pending overall validation; no E18 receipt or completed step is claimed. |
| Roadmap `09-01` | Depends directly on `08-04`; bound derivation, two fit checks, R15 threshold validation, source rustdoc values, and raw-sample research report are blocked. Failed raw attempts must not become the prescribed successful measurement report. |
| Roadmap `10-05` | Both capacity receipts require the same impossible recorded T1 attachment premise; they are pending. No replacement N/M or element-count criterion is introduced. |
| Later roadmap execution | Existing dependencies also prevent `09-02` and phase-10 advancement. No completed correct step or frozen phase 01–04 is blanket invalidated, and the append-only DES history is preserved. |

The independent R18 guard decision, retained mark → TPROXY → accept order,
R19 withdrawal, named-Service E14(e) result, and listener-loss classification
remain valid on their own completed native evidence. This failure provides no
new result about application-flow capacity, VMM resources, routing across
hosts, admission/retiring population timing, or future pending-state schedules.

## Reopened alternatives and next decision inputs

Only ADR-0114's **one bridge per workload** rejection is reopened here: its
stated ground included the one shared-switch density model. Its separate O(N)
switching-object cost remains a fact to assess. Reconsideration does not
recommend that alternative or choose another topology. Keeping per-workload
netns had separate structural-cost grounds; ADR-0117's 32,768 rejection cites
address overlap and resource pressure, and its 100,000 rejection cites prefix,
map, attachment resources, and VMM capacity. This evidence alone does not
invalidate those independent grounds. R15's epoch alternative remains subject
to its existing measured hold-time trigger, which did not run.

The next bounded DESIGN decision needs:

1. An explicit user decision on the contradiction between the recorded T1
   profiles and sole-bridge topology. No population decrease, topology change,
   kernel change, or capacity-policy expansion is authorized by this record.
2. An exact replacement contract naming its population/topology relationship
   and any necessarily changed ownership, lifecycle, public/cross-crate API,
   typed semantic errors, wire/persisted format, or configuration. Unchanged
   interfaces need no invented addition; an actual missing semantic error
   set/representation is `SPECIFICATION_AMBIGUITY`, returned to DESIGN.
3. Evidence for the replacement's production provisioning path and complete
   reachable held population, including any applicable admission/retiring
   state. Measurements at a smaller active population cannot silently certify
   unproved pending/retiring states or the unchanged T1 requirement. No new
   failure model or test seam is selected here.
4. Explicit approval for any new architecture or exact normative amendment,
   independent DESIGN review, then reconciliation and independent validation
   of the affected roadmap. DELIVER resumes only after those gates.

Likely fallout cannot be prescribed until that decision: a changed topology
would need its exact owner/interface and security proof effects recorded; a
changed measurement population would need its exact relationship to the held
cap and runtime-bound proof recorded. These are decision inputs, not mandates
for new mechanisms. The zero-frame, fail-closed, typed-error, effect-completion,
cleanup-complement, and lease-release-last requirements are not reduced.

## Documentary verification

The record author read only the affected canonical clauses and the necessary
synchronized-source attachment/rollback/cleanup path, verified the nine raw-file
digests and four relevant source-manifest hashes, and compared the normalized
foreign inventories. Independent review verified all 891 source-manifest entries
and reproduced the utility-hash mechanism read-only on Lima. The compiled
benchmark digest and loaded dependency identity remain unattested by this
receipt; the raw executable field is not used as that proof. Current validation
annotations were added to the feature delta and ADRs 0114/0117/0124; the
roadmap's pending validation and selected
step notes identify the same dependency scope. Exact N/M criteria and all
normative interface/security/lifecycle text remain unchanged. No staging or
commit was performed; independent review is required before committing this
bounded record.

**F1 documentary remediation (2026-10-04):** the executable qualification
overstatement identified by independent review iteration 1 is corrected above.
The raw utility fingerprint, synced-source evidence, ordinal inference, and
downstream-kernel limitation remain distinct. No support/production patch or
population rerun was performed. Re-review is pending; the reviewer owns the
finding's disposition, and DESIGN/roadmap execution remains pending.
