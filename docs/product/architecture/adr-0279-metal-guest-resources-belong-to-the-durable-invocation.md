# ADR-0279 — Metal launchers use the kernel local-mode vsock namespace

## Status

**Accepted — 2026-11-08.** The user explicitly stated “Approve P1–P5”. Independent technical review is APPROVED; the exact accepted contract is in the feature delta.

## Context

Identical concurrent guest groups reuse the same numeric CID bands and listener
values. The inspected pinned Linux kernel already keys these resources by
namespace and holds that namespace with the actual VMM device references.

## Decision

Give each ordinary guest/native launcher an anonymous local-mode host vsock
network namespace and retain the existing numeric CID/listener contracts. This
is launcher confinement; selected kernel tests still run in their existing
disposable guest groups. No extra outer guest surrounds the whole invocation.

## Alternatives

A userspace CID-window allocator creates state and release protocols the kernel
namespace makes unnecessary, and does not alone isolate listeners. A hash is
not exclusive ownership. A resource allocator daemon or guest pool is not
needed.

## Consequences

The physical initial namespace is unchanged. An old VMM/socket reference keeps
its old context; a new invocation never guesses that its CID is globally free.
Required namespace capability refuses closed. Invocation-owned ephemeral roots
and the existing production VMM confinement remain explicit. Existing native
fixture prerequisites must be validated without substituting nested evidence.

## Links

- [Exact proposed resource/lifecycle contract](../../feature/metal-run-isolation/feature-delta.md#wave-design--ref-kernel-resource-boundary-and-finite-lifecycle--p3)
- [Pinned kernel sources and hashes](../../feature/metal-run-isolation/design/kernel-source-provenance.json)
- [Native paired-guest result](../../feature/metal-run-isolation/spike/g-paired-result.json)
