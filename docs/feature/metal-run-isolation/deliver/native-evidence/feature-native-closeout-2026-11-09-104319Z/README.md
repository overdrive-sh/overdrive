# Feature-specific native closeout — worktree/admission pass, login mode blocked

## Results

The workflows ran sequentially. No passed worktree or admission workflow was
rerun, and no Quint, mutation, or unrelated owner-burst campaign was added.

| Capture | Actual result |
|---|---|
| `initial/prepare-root/`, `initial/prepare-peer/` | Canonical guest-kit preparation 0 / 0 |
| `initial/worktree/` | Driver 0; K1–K3 overlap, survivor and final complements passed |
| `initial/admission/` | Driver 0; actual public timeout and controlled-owner E4 witnesses passed |
| `initial/placement/` | Driver 1 at the authored archive error-marker mismatch; preceding command statuses matched their original expectations; foreign fixture cleanup 0 |
| `invalidated-continuation/` | Driver 1: this author's evidence copy changed checkout identity inputs during the ordinary/no-sync sequence; no-sync correctly refused 78 |
| `login-mode-continuation/` | Driver 1 with stable source: ordinary and no-sync passed 0; no-sudo failed 75, expected 0 |

There is **no complete placement PASS**, no all-owner GREEN claim, and no final
placement `after.json`. The combined `--no-sync --no-sudo` case remains unrun.
No completion or finalization approval is implied.

## Source, preparation and substrate

The initial capture pins root source
`8914e5ee47da37689919399382b9304d75d18a21` and clean peer source
`06f25ce2d83af4578db9f1dcabad682e0cccb544`. These differ by evidence only and
have identical hashes for all nine recorded feature production/driver files.
Both contain the current runtime correction `47328fb88d21d5f3f57d25a935d1a17c505cbb9b`.
Every invocation retains exact command, times, source HEAD/status, configured
kernel/rootfs inputs and source hashes before/after. The root's unrelated
dirty work was preserved. `peer-update.json` records the clean detached peer
update without changing its canonical identity or ignored directory inodes,
ownership or modes; the root branch was not changed.

Preparation runs `341db4d3073b813e691eceba` and
`1c5f294f0f904f4f0654ec2b` both completed 0 and reported the lane prepared for
kernel `7.0.0-38-generic`, target `x86_64-unknown-linux-musl`. The read-only
substrate record identifies native x86_64 Linux `7.0.0-38-generic`, boot ID
`acdbd1e5-824c-4946-9aa7-e3b67c05d9a5`, Cargo 1.95.0, Nextest 0.9.149,
Bash 5.3.9, systemd 259 and util-linux flock 2.41.3. Their full queried output
and statuses are retained. Test commands used the canonical updated runner;
SSH was used only by the existing external drivers and scoped read-only
substrate/failed-owner observations.

## Worktree and admission outcomes

Worktree runs `531f46c36767a7f42293490d` and
`a5e15be51e13740a6687f1a2` held actual ordinary READ maintenance locks and
exclusive canonical-checkout locks, with distinct checkout keys. Live VMMs
2726976 and 2727106 shared CID 16785416 in distinct namespaces
`net:[4026532936]` and `net:[4026532989]`. After the first supervisor was
explicitly stopped through its real owner, its collection and repeat both
returned 70/`supervisor_lost`; the surviving VMM, roots, checkout and source
remained unchanged. The survivor completed its real core group with 86 passed,
0 failed and command 0. Repeated retained output was byte-identical. Both
owners finished without processes or roots, and raw physical before/after
snapshots were exactly equal: no RA-expiry projection adjustment was needed.

Admission retains the actual guest body-start console and timeout 100 for
`673d17e08eaf52c67c74577f`. Its E4 witness uses the explicitly approved
NativeUnit controlled prior owner, not an unchanged public guest stop window.
`unadmitted-waiter.json` brackets prior `2712e47b72f4831bb4381138` deactivating,
maintainer `0d0fd9dc1fd7804674408e1f` waiting with actual EX, and ordinary
`9fa4e01fb51a44353ba64214` waiting without SH. After the existing fixture stop
input, the prior's real finalizer completed cleanup; the maintainer progressed
while the ordinary waiter remained unadmitted. Final statuses are 100/0/0/0,
all four receipts complete, all units stopped, and all three roots absent for
each owner. The raw witness and completion observations remain intact.

## Exact unrelated baseline exclusion

The user instructed: **“stop focusing on failures that are not related to this
feature !!!!!”** Only this original driver command was omitted from the
temporary external execution copy:

```sh
run_case owner-cell 0 run -- cargo nextest run -p overdrive-init --bin overdrive-init --features integration-tests,kvm-tests -E 'test(guest_flow::real_kernel::a_burst_of_host_aborts_resets_every_application_connection)' --no-capture
```

The [preimplementation baseline](../owner-burst-preimplementation-baseline-2026-11-09/README.md)
at `0e8ae265c2d80d98c345b342a396057940837e3a` reproduced 78/1,000 ends within
the existing one-second bound, outer 100, with byte-identical body and fixture
blobs. This excludes that unrelated failure from feature closeout; it does not
establish that owner cell as passing. `initial/placement-original.sh`, its
one-line-deletion execution copy and `exclusion.json` preserve the exact scope.
Every other original command, expected status and assertion remained present.

## Archive oracle correction and continuation attribution

The initial placement capture completed attempts/retry/retained collection,
actual timeout/body-start/cleanup, guest reset, both profile cases, direct
native, named nested, operator cell, command 255/collection, and all negative
command statuses as expected. The archive command actually returned
`artifact_scope_mismatch`/64, as accepted P4 and bootstrap require; the old
driver incorrectly grouped it with execution/config replacement errors.

Commit `4bea1ebe3dc8da3cae9451c293ddbab4f39f4671` moves only that existing label
between the two marker groups. Bash syntax, whitespace and all negative marker
assertions replayed against the immutable actual capture passed. Production,
typed mappings, status 64, other oracles and the baseline exclusion are unchanged.

The continuations execute the corrected original tail only: marker assertions,
invalid-mode, four pure-mode commands and final physical/owner assertions.
They retain the real original invocation IDs and physical baseline, with an
explicit manifest for every copied prefix artifact. The earlier foreign fixture
already had actual cleanup 0 and is not recreated. The prefix is attributed to
8914e5ee; it is never claimed to have rerun at 4bea1ebe. Both continuation scripts
and actual commands are retained.

I copied the initial evidence into this checkout during the first continuation.
That changed its dirty-source identity between ordinary sync and no-sync; the
78 refusal was correct. Its changed start/end status and raw failure are retained
without credit. I then froze checkout writes and repeated only the same original
tail. Its source status stayed unchanged and no-sync passed 0, exposing the
independent login-mode failure below. No body, timeout or oracle was relaxed.

## Reproduced feature blocker: login temporary-root creation

At source 4bea1ebe, run `cbc4adb69839725766cd35be` used the exact existing
`run --no-sudo` host-pure command. Bootstrap failed before START while creating
its invocation temporary directory: `install` reported permission denied and
the canonical command returned **75**, `runner_preparation_failed`, instead
of the original expected 0.

The scoped parent stat observes the exact bound checkout's
`target/metal-run-tmp` as **root:root 0755**. Bootstrap's existing temporary-root
preparation (`infra/metal/bootstrap.sh:1081-1084`) creates login-mode children
as `sudo -n -u ubuntu install ...`, which cannot create a child in that parent.
The accepted contract retains login mode and requires matching access for the
invocation temporary root (`feature-delta.md:247,294,327`). No production fix
or new API is proposed here; this reachable failure returns to the original
crafter.

The existing read-only owner observer confirms a stopped/not-found unit,
retained runner preparation failure, complete cleanup receipt, no processes,
and all owned roots absent. Query output/errors and status are retained in
`login-mode-continuation/failed-login-*`. The final combined login/no-sync case
and final placement physical-complement assertion were never reached and are
not credited. All raw files were copied byte-for-byte from their named `/tmp`
captures; no unrelated native owner was interrupted and no diagnostic was filtered away.
