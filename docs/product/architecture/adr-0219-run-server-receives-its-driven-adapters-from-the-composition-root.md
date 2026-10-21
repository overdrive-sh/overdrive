# ADR-0219 — `run_server` receives its driven adapters from the composition root

## Status

**Accepted — user ruling NO-OVERRIDE-SEAMS, 2026-10-19** ("DISTILL rulings,
after round 10"), and its scope ruling of the same day ("ALL of it, in
#295"), with its STORES-IN ruling of the same day (the persistent stores
and the workload-identity CA come from the composition root too, with no
carve-out). GH #295. Not yet implemented. The bundle's shape
(`NodeAdapters` and its groups, `VmCapability`, the opaque
`GuestFlowAdapter`, the dataplane composer and the mTLS enforcement factory
ports, the store and CA groups, the host-bundle constructor) is an
architect elaboration under the rulings, pending adversarial review.
Contract: the #295 feature delta, § *[REF] Composition — production entry
points*, "Driven adapters of `run_server` (NO-OVERRIDE-SEAMS)".

This decision amends one implemented decision, ADR-0083 § D8 (the
`vmm_override` seam), which carries a dated amendment pointing here.
ADR-0138, which this decision completes, is #295's own and not yet
implemented; it states the result directly.

Related decisions, each separately reversible: the node-global vsock ports
are node configuration (ADR-0218); ADR-0083 § D8's probe-refusal semantics
for a VMM that is declared present.

## Context

`run_server` composed the node partly over adapters it built itself and
partly over adapters a test swapped in through `ServerConfig` fields:

- `vm_override` (ADR-0083 § D8) replaced the VMM and the beacon port; its
  presence also decided whether a failed VMM probe refused the boot.
- `dataplane_override` replaced the dataplane, skipped the dataplane's
  provisioning, construction and probe, and its presence decided the
  node's `host_ipv4` fallback to loopback.
- `mtls_identity_override` replaced the identity the mTLS enforcement
  reads; `dataplane_probe_fault` and `mtls_probe_fault` forced a probe
  failure inside the production boot.
- `clock` (defaulted to `SystemClock` by `ServerConfig::new`), `kek`,
  `mtls_intercept` and `guest_dns` (ADR-0138) were carried as adapter
  fields of the configuration.
- `run_server` constructed the TCP and HTTP probers, the cgroup accounting,
  the VM host state, the observation store and a second `SystemClock` that
  ignored `config.clock` itself; the boot it delegates to opened the intent
  store, the view store and the workflow journal from its data directory
  and constructed the workload-identity CA and its root-key codec.

Each override is a configuration field whose purpose is to change which
adapter the production boot composes, and the defaulted clock let a test
that forgot to choose one run on the wall clock. `.claude/rules/development.md`
§ "Port-trait dependencies" requires a port's implementation to be a
required argument, composed at the binary boundary.

## Decision

- **`run_server` takes every driven adapter it composes the node over as
  one required parameter**, built by the composition root. `overdrive
  serve` passes the host adapters; a test passes simulation adapters or
  host adapters it chose by name. No field has a default. Neither
  `run_server` nor the boot it delegates to constructs an adapter over a
  host resource or a store; the persistent stores and the workload-identity
  CA are included in that. They compose driver and owner objects (the VM
  driver, the driver registry, the guest-flow owner, the workflow engine)
  over the ports they are given.
- **Every boot opens its stores.** The persistent stores are opened from
  disk for each boot and are not shared between boots, so a restarted boot
  in a test reruns the intent store's boot-validation walks exactly as a
  new process does.
- **The composition root wires; the boot probes.** The root constructs and
  opens adapters (the stores are opened over the configured data
  directory); the boot runs each adapter's Earned-Trust probe before first
  use, as before — the view store's probe at reconciler registration, the
  CA's key-encryption-key and envelope probes and its generate-or-adopt of
  the persistent root — so every composition, host or test, runs them.
- **`ServerConfig` carries configuration only.** No field of it carries or
  swaps an adapter, and no field injects a fault. A fault a test needs is
  scripted on a simulation adapter it passes.
- **Adapters whose production construction needs boot state are passed as
  factories.** The dataplane is passed as a composer that provisions,
  constructs and probes it from the dataplane configuration (and reports the
  node address it resolved), because its effects must follow the boot's
  startup checks; the mTLS enforcement is passed as a factory that receives
  the boot's identity holder and guest dial, which exist only inside the
  boot. The guest-DNS responder was already such a factory (ADR-0138). The
  workload-identity CA is passed as a factory the boot calls over the boot's
  one entropy source, so the CA's key generation cannot draw from a
  different source than the rest of the boot; the boot then runs its
  generate-or-adopt.
- **VM capability is declared by the composition root.** The bundle states
  whether the node requires VM capability (`Required`) or may boot without
  it (`Optional`). That statement, not the presence of an override, decides
  whether a failed VMM or cgroup-accounting probe refuses the boot.
  `overdrive serve` declares `Optional`, today's production behaviour; a
  test whose VMM is a simulation declares `Required`.
- **The supplied-owner entry point takes the same boot-wide group** of the
  bundle, together with the driver registry and guest-flow owner it already
  took.

**Why `Required` exists though `overdrive serve` never selects it.** The
refusal it selects is ADR-0083 § D8's implemented contract: a probe failure
of a VMM the composition root knows is present is a substrate lie, not an
absent capability, and S-VM-13, S-VM-51 and the part of S-VM-67 that
§ D8 reaches depend on it. A root
knows its VMM is present only when it constructed the VMM as a double;
`overdrive serve` cannot know before the probe whether the host has a
hypervisor, so it declares `Optional`. The variant is a parameter of the
production refusal path, which runs in the binary's real composition; no
state is added that only a test configuration produces. A node-configuration
producer (an operator declaring that a node must run VMs) is a different
decision with a different outcome on a production node, and is not taken
here.

## Alternatives considered

- **Keep `vm_override` and add `guest_flow_io_override`.**
  Simulation substitutes inside the production configuration. Rejected by
  the user.
- **Move only the VM and guest-flow ports; leave the other override and
  adapter fields.** Leaves `dataplane_override` deciding the
  node address and `ServerConfig::new` defaulting the clock. Rejected by
  the user's scope ruling.
- **Pass the constructed dataplane instead of a composer.** The production
  dataplane's construction provisions host links and attaches programs;
  constructing it at the composition root would place those effects before
  the boot's startup checks and cgroup preflight. Rejected.
- **Pass the mTLS identity read alone** (an identity factory) and keep the
  enforcement composed inside the boot. Keeps the substitution the old
  `mtls_identity_override` made, but leaves the enforcement's probe
  refusal reachable only through a fault field, which the scope ruling
  removes. Rejected: the enforcement factory covers both.
- **Make `GuestFlowIo` and `SimGuestFlowIo` public.** Exposes the owner's
  crate-private socket vocabulary; under ADR-0218 no out-of-crate test needs
  a simulated owner seam. Rejected.
- **Classify a failed VMM probe by its error variant instead of a declared
  capability.** Changes ADR-0083 § D8's absence-versus-refusal semantics for
  production. Rejected.
- **Keep the persistent stores and the CA constructed inside the boot**,
  because no test substituted them. Leaves the boot
  constructing driven adapters, although simulation view and journal stores
  and a simulation CA exist, and splits the observation store (moved) from
  the stores opened the same way beside it. Rejected by the user (STORES-IN).
- **Run the CA's generate-or-adopt at the composition root** as part of
  building the bundle. A composition that supplied its own CA would then
  skip the CA's Earned-Trust probes, and the CA's boot would precede the
  boot's startup checks. Rejected: the root supplies the CA; the boot
  probes and adopts.
- **Pass a constructed CA and require a test that replaces the entropy to
  replace the CA as well.** This leaves a CA keyed to another entropy
  source representable, and nothing would detect it. Rejected in favour of
  the factory.
- **Let a restart test hand the previous boot's opened stores to the next
  boot.** The second boot would never reopen the files, so it would skip
  the reopen-time validation a real restart performs. Rejected.

## Consequences

- Every test that booted `run_server`, the supplied-owner path or the CLI's
  `serve` entry points migrates to the bundle. A test that wants the
  composition `overdrive serve` runs names the host-bundle constructor and
  replaces only what it simulates.
- One clock serves the whole boot; the second `SystemClock` is gone.
- The node address the boot uses comes from the dataplane composer that
  produced the dataplane, so no branch keys on whether a test substituted
  the dataplane.
- The mTLS worker, the DNS task owner and the shared-network supervisor
  stay composed unconditionally on every boot (ADR-0138).
- The persistent stores and the workload-identity CA come from the
  composition root like every other adapter. The node's durable files are
  therefore opened before the boot's first step: a boot refused at a later
  startup check has still created them, which is harmless (they are the
  node's own idempotent state under its data directory) and was already
  true of the observation store. One bundle serves one boot: each boot
  builds its own CA, which caches the root it adopted, and reopens its own
  stores.
- The test entry point of the command-line `serve` runs the cgroup probe
  before it builds the bundle, as production does, and builds the bundle
  from the same configuration the boot receives.
