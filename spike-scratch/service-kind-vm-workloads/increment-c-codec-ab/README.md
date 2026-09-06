# Increment C: VM-Exec codec A/B

Throwaway, self-contained native-metal probe comparing length-prefixed JSON
and `rkyv` for one provisional VM-Exec message algebra. It is not production
code, a test tier, an accepted protocol, or an API commitment.

## Question

For the same bounded, versioned VM-Exec control algebra and the
`overdrive-init` production release/target settings, does JSON or `rkyv` have a
material binary-size, wire-size, validation-complexity, or evolution advantage?
There is no performance budget and no post-hoc materiality threshold.

The single source is compiled three ways:

- `baseline`: activates the existing Beacon-style `serde_json` argv path;
- `json`: baseline plus the provisional JSON VM-Exec codec;
- `rkyv`: baseline plus the provisional `rkyv` VM-Exec codec and bytecheck.

All variants use a four-byte big-endian frame length, a four-byte schema
version inside the bounded payload, and a 16 KiB pre-allocation cap. The
provisional logical algebra covers handshake/version, session generation,
request ID, direct argv, cancellation, overload, and seven result categories.

## Prerequisites

- Repository-root `.env` contains `OVERDRIVE_METAL_TARGET`.
- The target passes `cargo xtask metal`'s native x86_64/KVM/no-nesting preflight.
- The target has the Rust `x86_64-unknown-linux-musl` target and binutils.
- Local `/opt/homebrew/bin/rsync`, SSH, Cargo, and Perl are available.

## Run and capture

From the repository root, choose a new zero-padded run ID. Captures are
append-only and the script refuses overwrites.

```sh
bash spike-scratch/service-kind-vm-workloads/increment-c-codec-ab/capture.sh 0001
```

The canonical runner syncs the workspace, runs the fail-closed metal preflight,
builds actual `overdrive-init` and all three representative musl release
binaries, and executes all three. Build outputs remain ignored under `target/`
and `out/`. `runs/<id>.stdout` and `.stderr` are raw command streams except for
explicit metal-target/hostname redaction; `.meta` records timestamps, exit
status, sanitized command, and SHA-256 hashes.

## Interpretation limits

The representative binaries isolate incremental codec cost but are not the
full init binary, so their absolute sizes are not production deltas. The run
records actual current stripped `overdrive-init` size separately. Linker, LTO,
codegen, dependency-version, and message-shape changes can move all results.
