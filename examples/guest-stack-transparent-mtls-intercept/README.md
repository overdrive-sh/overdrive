# One VM Job calls one Exec Service (E07)

This directory contains the feature's one checked-in operator journey and it
supports exactly one expectation: E07. `callee.toml` deploys one `[service]` +
`[exec]` callee; `caller.toml` deploys one `[job]` + `[vm]` caller. The VM
resolves `gti-e07-callee.svc.overdrive.local` and its Job can succeed only after
receiving the byte-exact reply.

The example is restricted to a native, non-virtualized x86_64 Linux host with
working KVM. Lima, nested KVM, and a compile-only run are not runtime evidence.
Use the canonical metal command so one host-global lease covers sync,
preflight, preparation, execution, public cleanup, and final owned-fixture
checks:

```sh
export OVERDRIVE_METAL_TARGET=user@native-metal-host
export OVERDRIVE_METAL_KERNEL=/srv/vm/overdrive-testing/kernel
export OVERDRIVE_METAL_ROOTFS=/srv/vm/overdrive-testing/rootfs.ext4
cargo xtask metal run -- \
  examples/guest-stack-transparent-mtls-intercept/run-example.sh run
```

The two `OVERDRIVE_METAL_*` artifact variables select real, readable appliance
inputs for the fail-closed metal preflight. On the metal host, the preparation
script defaults to those same canonical base paths. It can instead take
`GTI_E07_BASE_KERNEL` and `GTI_E07_BASE_ROOTFS` when the qualified inputs live
elsewhere.

## Materialization contract

`prepare.sh` is the sole preparation entry point used by the operator script
and the pending E07 runner's static check. It does not generate Rust source,
Cargo manifests, or TOML specs. On `prepare`, it:

- compiles both checked-in helpers for `x86_64-unknown-linux-musl` and rejects
  either binary if it has a dynamic interpreter;
- reflinks a private rootfs to
  `/srv/vm/overdrive-testing/gti-e07/rootfs.ext4`, installs the caller at
  `/opt/overdrive/examples/gti/e07-caller`, and always unmounts and detaches its
  loop device through bounded traps;
- copies the selected kernel to the exact `caller.toml` path and places the
  static callee at the exact `callee.toml` path;
- co-locates the private rootfs and explicit `serve --data-dir` beneath the
  same reflink-capable staging root, with root-owned mode `0711` traversal for
  the confined VMM identity; and
- writes a fresh 32-byte, mode-`0400` `overdrive-ca-root` credential under the
  isolated mode-`0700` credentials directory. `run-example.sh` supplies it
  only through the production `CREDENTIALS_DIRECTORY/<kek-id>` contract while
  `serve` runs in a fresh anonymous session keyring.

The fixed runtime tree is marker-owned. `prepare.sh cleanup` refuses an
unmarked or mounted tree, removes only
`/srv/vm/overdrive-testing/gti-e07`, and never purges or overwrites an ambient
session-keyring entry. Preparation arms its traps and records process-local
ownership before creating the fixed tree, so failure and signals execute the
same bounded unmount/detach cleanup even before the durable marker exists.
The operator script adds a per-invocation token to that marker and refuses to
remove a tree whose token it did not create.

## Runtime and cleanup contract

`run-example.sh` builds the default-feature `overdrive` binary, runs the shared
preparation entry point, starts one isolated `serve`, deploys the callee and
caller through public commands, and accepts E07 only when public `workload
describe` reports the reply-dependent caller Job as `Succeeded`. The explicit
`health_check.startup = []` in `callee.toml` is the supported opt-out for this
narrow journey: it prevents production from inferring the known-unreachable
host-namespace TCP startup probe. The Service describe surface can therefore
report its allocation as `Running` with replicas `1/1` while the VM calls it.

Every build, prepare, serve-readiness, deploy, describe, stop, unmount, loop
detach, and cleanup wait has a finite deadline. Traps are installed before the
first materialization. `serve` is launched through required `keyctl session -`
isolation after verifying that the new session does not already contain the
production KEK description; that session dies with the bounded serve
lifecycle. Before `keyctl` or `serve` exists, a `setsid` wrapper atomically
publishes its token-bound PID, process group, and Linux start time. The parent
tracks that direct child independently across the wrapper-to-serve `exec`
handoff; the serve identity is published atomically before `exec` as well.
Cleanup is already armed during both handoffs. On success, error, signal, or a
PID-handshake timeout it sends bounded TERM then KILL only to that proven
private launch group and reaps the direct wrapper only after exit is observed.
It never waits indefinitely or signals an unverified/reused PID. The script
also requires successful public stop results for the exact caller and callee
workload IDs and removes only its marker-owned materialization. It neither
inspects nor repairs product-private processes, run directories, cgroups,
namespaces, links, or capture state.

This journey proves only the stakeholder-visible successful call. Strict
nft/netlink framing, normalized rule programs, counter equality, packet
capture, TLS/kTLS state, generation stability, lifecycle action vectors, and
private kernel cleanup remain executable Rust integration-test obligations.
