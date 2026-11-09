# Placement rerun — nested scripted-image dependency resolver is absent

This directory preserves the current placement capture copied from
`/tmp/metal-native-remaining-20261109T061627Z-placement`. Its foreground
invocation reports exit 1 at source HEAD
`261bf29cda77b5c04d2124367229d738d8dfb446`; the exact dirty source status is
retained in `foreground-invocation.json`.

## Captured results

The direct-native V1C test passed: run `c6ca1e50e6ac9843fc4d483b` returned 0.
Its retained receipt records kernel `7.0.0-38-generic`, uid/gid 4200,
supplementary device group 991, one vhost-vsock fd, launch confinement, and a
successful data round trip.

The nested run `f48ead25a2c73c5d67429677` returned 100 with the existing typed
`guest_execution_failed` result. It fails while the selected existing
`guest_boundary` test constructs its inner scripted image; the outer nested
guest is up, but no inner VMM is launched. The exact error is:

```text
SharedLibraries { binary: "/usr/sbin/tc", detail: "spawning ldd: No such file or directory (os error 2)" }
```

The foreground placement driver therefore exits 1 before later cases and
does not produce the final `after.json` complement. Foreign-fixture cleanup
returned 0. This attempt is separate from the earlier 054741Z capture: the
scheduler-module source fix is present here, and the failure has progressed to
the next inner-image prerequisite.

## Producer and consumer path

The selected `nested-boundary` plan stages the existing nested GuestKit
module sources and tool inputs into the outer guest. Inside that guest,
`crates/overdrive-fwd/tests/integration/guest_boundary.rs:86-95` calls
`GuestKit::provision` and then `GuestKit::image(..., GuestImageSpec::scripted())`.
The image consumer copies `/usr/sbin/tc` and calls `shared_libraries(tc)` for
the scripted image (`crates/overdrive-testing/src/guest_kit.rs:525-535`). That
helper invokes `Command::new("ldd")` (`guest_kit.rs:1092-1116`). The captured
failure is at that subprocess boundary: `/usr/sbin/tc` was found, but the
outer guest could not execute `ldd` to enumerate its dependencies.

The outer guest producer's explicit tool list in
`crates/overdrive-testing/bin/fwd_guest_suite.rs:1920-1953` stages `sh`, the
nested host tools, and the nested module sources, but not `ldd` or a `bash`
interpreter. Its scripted `GuestKit::image` also stages `tc` and its shared
libraries; it does not install `ldd` or a full host root. The nested
scripted-image consumer therefore has no `ldd` entry point in its environment.
This is private fixture-support incompleteness, not a failure of the physical
VMM or the KVM/vhost device setup.

The bounded owner route is to stage the existing `ldd` helper and only the
runtime interpreter/shared-library inputs required by that helper into the
outer guest through the existing `nested_vm_test_support` `GuestFile` list.
If the target's `ldd` is a script, preserve its actual shebang interpreter and
that interpreter's shared libraries; the current capture stops at `spawn ldd`
and does not record the target `ldd` file or its downstream helper closure.
The direct-native V1C success establishes the host's existing scripted-image
path, but does not establish that the outer guest has the same helper inputs.
No new API, fallback, product behavior, or test oracle is implicated.

## Canonical read-only target helper probe

After the placement capture, one read-only command ran through the existing
exclusive Metal entry point. Run `0d0e9d1b211cf7d68b933ff2` completed with
status 0. It reported target architecture `x86_64`; `/usr/bin/ldd` resolves to
itself, is mode 0755 root:root, and is a Bourne-Again shell script with
shebang `#!/bin/bash`. `/bin/bash` is an ELF executable using
`/lib64/ld-linux-x86-64.so.2`; `ldd /bin/bash` reports `libtinfo.so.6`,
`libc.so.6`, and that loader. `ldd /usr/sbin/tc` reports `libbpf.so.1`,
`libelf.so.1`, `libmnl.so.0`, `libcap.so.2`, `libm.so.6`, `libxtables.so.12`,
`libc.so.6`, `libz.so.1`, `libzstd.so.1`, and the same loader.

The bounded probe retained the ldd shebang and Bash runtime set; it printed
only the first twelve ldd script lines, so it does not claim an exhaustive list
of any further external helper commands in that script. This is read-only host
metadata, not evidence that the nested GuestKit image already has those files.
