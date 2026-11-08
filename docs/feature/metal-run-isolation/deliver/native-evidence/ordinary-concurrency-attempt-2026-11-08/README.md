# Ordinary concurrency proof attempt — pending

This is the retained output from the authorized `worktree_journey.sh` attempt.
It is **not** a successful K1–K3 capture: both ordinary invocations timed out
before `START`, and the driver observed no running command pair, VMM pair, or
guest test body.

## Invocation and source attribution

The external driver was run with:

```sh
OVERDRIVE_METAL_TARGET=ubuntu@151.115.99.251 \
OVERDRIVE_METAL_KERNEL=/srv/vm/overdrive-testing/kernel \
OVERDRIVE_METAL_ROOTFS=/srv/vm/overdrive-testing/rootfs.ext4 \
OVERDRIVE_METAL_LEASE_TIMEOUT_SECONDS=5 \
bash infra/metal/tests/worktree_journey.sh \
  /Users/marcus/conductor/workspaces/helios/ouagadougou \
  /tmp/metal-run-isolation-peer.UKMMtE6B \
  /tmp/metal-run-isolation-journey-final
```

At capture, both input checkouts had `HEAD=a1f386fb644efad3a2a66547e7512eea39fe01a3`.
The first input was `ouagadougou` and its exact dirty-file list is retained in
[`first-source.status`](first-source.status); it includes pre-existing
`service-allocation-readiness-and-ending` work and `AGENTS.md`. The second was
the detached temporary worktree at `/tmp/metal-run-isolation-peer.UKMMtE6B`;
the step 01-02 source files were copied into it, and its source list is retained
in [`second-source.status`](second-source.status). The implementation commit
was created after this proof attempt as `2ba2232208ce9a5e91666f61b6b9845d9797c62a`.

The target was `ubuntu@151.115.99.251`; the prepared host reported kernel
`7.0.0-38-generic` and target `x86_64-unknown-linux-musl`.
`guest-kit-prepare` completed through ordinary
Metal admission for both inputs before this attempt. The final proof command
itself was an ordinary run of the existing explicit `core` guest-suite path;
it did not use `--exclusive`, direct SSH execution, or a physical-host
`kmod-load` step.

## First failure and owner evidence

The captured run IDs were `554fc291d793ef06d84a7928` and
`d12aee8a2ce2bf57507197d7`. Both durable records reported
`execution_class=ordinary`, `phase=waiting_for_lease`, and the same physical
maintenance owner. Both controller runs returned status 75 with
`OVERDRIVE_METAL_RUNNER_ERROR error=maintenance_timeout` after the explicitly
bounded five-second admission wait. Neither run reached `OVERDRIVE_METAL_RUN_STARTED`.

The existing owner reported by both callers was:

| Field | Observed value |
|---|---|
| PID | `1680367` |
| Unit | `overdrive-metal-run-2d050629d625a324fc113fd7.service` |
| Workspace | `/Users/marcus/conductor/workspaces/helios/bangalore-v1` |
| Commit | `036032935f101fc29e6d79d69d2796e9345a2b23` |
| Started at | `2026-11-08T20:42:33Z` |
| Source digest | `19e8a3fb8636bb72b2e7a3508d11c764a320767a5f9f382f98755d5143b5073c` |

The owner record does not include its selected command line; no command is
inferred for it. The two new invocation source digests and remote directories
from the real owner observations were:

| Checkout | Source digest | Remote directory |
|---|---|---|
| `ouagadougou` | `2bd2735a4e76e6afe1fcd4f65ba63cb1342bc706864dec338a764e8bb36b7639` | `/home/ubuntu/overdrive-worktrees/e3d831d7dafe621226ae6047ebcac3536f37470cafcf5dbb64eb9598886af410` |
| Detached peer | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` | `/home/ubuntu/overdrive-worktrees/052a646313948d416e08f7afb85c82411ecddc98659ed0825aa09bcde3be0f39` |
| Held boundary | `/run/lock/overdrive-metal-shared.lock` |

The driver's repeated observations saw both new owners waiting, then inactive;
the expected running-owner assertions failed. The first query observations also
retain the races where the not-yet-materialized run record was absent. Raw
stdout, stderr, source snapshots, physical baseline, owner observations, and
those first failures are preserved beside this note. The local observation
loop was stopped after the two bounded admission failures were established; no
remote owner was interrupted or replaced.

`admission_journey.sh` and `placement_journey.sh` were not run in this attempt.
Their existing scenarios include exclusive admissions; the authorized final
native proof for this step was limited to the ordinary concurrent pair.

## Lima evidence and remaining limits

The assigned ordinary-run acceptance suite passed in Lima (32 passed, 16
deliberate external-only ignores), as did the `overdrive-testing` library
tests (24 passed), both transitioned qualification tests, scoped Clippy,
all-target `overdrive-testing` check, Bash syntax checks, and scoped rustfmt.

The model-owner conformance selector cannot complete in ARM Lima because its
real `native-preflight.sh` path correctly refuses with
`preflight_failed probe=arch`; native-dependent cases remain pending. The full
workspace `cargo fmt --all -- --check` also reports formatting drift in the
untouched `crates/overdrive-testing/bin/guest_peer.rs` and
`xtask/tests/acceptance.rs`; every Rust file owned by step 01-02 passes scoped
rustfmt. No mutation run was performed.
