# Ordinary concurrency proof attempt 2 — pending

This is a second authorized invocation of the existing `worktree_journey.sh`
driver. It also **did not complete the K1–K3 proof**. The two ordinary waits
were sequential: the first timed out before the second run registered, so the
capture never showed both ordinary runs waiting at once. Each run entered
`waiting_for_lease` and timed out before START. No test command or guest VMM
started. Raw output and every captured first failure are retained beside this
note; the earlier attempt remains in
[`ordinary-concurrency-attempt-2026-11-08`](../ordinary-concurrency-attempt-2026-11-08/README.md).

## Invocation and inputs

```sh
OVERDRIVE_METAL_TARGET=ubuntu@151.115.99.251 \
OVERDRIVE_METAL_KERNEL=/srv/vm/overdrive-testing/kernel \
OVERDRIVE_METAL_ROOTFS=/srv/vm/overdrive-testing/rootfs.ext4 \
OVERDRIVE_METAL_LEASE_TIMEOUT_SECONDS=5 \
bash infra/metal/tests/worktree_journey.sh \
  /Users/marcus/conductor/workspaces/helios/ouagadougou \
  /tmp/metal-run-isolation-peer.UKMMtE6B \
  /tmp/metal-run-isolation-journey-attempt-2
```

The first checkout was at `HEAD=59a9296d1195e2516683ff7a44c72f4b042a8c2a`
with unrelated service-feature and `AGENTS.md` dirty work. Its complete
`git status --porcelain=v1` capture is in `first-source.status`. The second
checkout was a detached worktree at implementation commit
`2ba2232208ce9a5e91666f61b6b9845d9797c62a`; its source status is in
`second-source.status`. The remote checkout keys were
`e3d831d7dafe621226ae6047ebcac3536f37470cafcf5dbb64eb9598886af410` and
`052a646313948d416e08f7afb85c82411ecddc98659ed0825aa09bcde3be0f39`,
respectively. The second checkout reused its prepared artifacts
only after its re-created worktree resolved to the same canonical git-dir key
as the prepared checkout. Both checkouts had completed ordinary
`guest-kit-prepare` against the target before this driver attempt.

## Observed admission boundary

The driver registered these two ordinary runs:

| Checkout | Run ID | Remote source digest | Remote directory | Result |
|---|---|---|---|---|
| `ouagadougou` | `e83202b4ffbbf9fb44c4a194` | `10ac14cf3c89e4f6fcfb531c3e7b31980731d021174d8444cb4050555237b297` | `/home/ubuntu/overdrive-worktrees/e3d831d7dafe621226ae6047ebcac3536f37470cafcf5dbb64eb9598886af410` | Captured stream: `maintenance_timeout` before START; command result reported status 75 |
| Detached implementation worktree | `be3b95671f4ab77148735ae2` | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` | `/home/ubuntu/overdrive-worktrees/052a646313948d416e08f7afb85c82411ecddc98659ed0825aa09bcde3be0f39` | Captured stream: `maintenance_timeout` before START; command result reported status 75 |

The retained controller streams contain the error name but not numeric process
exit codes; status 75 is the reported command result, not a value present in
those raw streams. The raw timestamps show
the first run registered at `21:24:03Z` and timed out at `21:24:08Z`; the
second registered at `21:24:22Z` and timed out at `21:24:27Z`.

Both controllers reported the same existing owner on
`/run/lock/overdrive-metal-shared.lock`:

| Field | Observed value |
|---|---|
| PID | `1758326` |
| Unit | `overdrive-metal-run-9e34103f0e184c634e5cd7af.service` |
| Workspace | `/Users/marcus/conductor/workspaces/helios/bangalore-v1` |
| Commit | `036032935f101fc29e6d79d69d2796e9345a2b23` |
| Started at | `2026-11-08T21:02:45Z` |
| Source digest | `19e8a3fb8636bb72b2e7a3508d11c764a320767a5f9f382f98755d5143b5073c` |

The owner fields identify a Bangalore unit different from the one recorded in
attempt 1. The retained owner record contains `action=run`, but does not contain
`execution_class` or the selected command; neither is inferred here. The
observed new run records were `ordinary` and `waiting_for_lease`; later snapshots
showed them inactive after the five-second timeout. `OVERDRIVE_METAL_RUN_STARTED`
was absent from both controller logs. The existing owner was not interrupted,
replaced, or otherwise touched. Once the two timeouts were captured, the local
driver was stopped instead of continuing its polling loop. Treat this as a
native attempt with two sequential admission timeouts, not as successful
concurrency evidence.

`admission_journey.sh` and `placement_journey.sh` remain unrun; this authorized
native proof was restricted to the ordinary concurrent pair. No mutation test,
exclusive test campaign, direct SSH command execution, or host `kmod-load` was
performed.
