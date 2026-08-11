# Slice 02 — A VM that fails to boot says why, and a VM service is honestly refused

> DISCUSS brief (2026-08-01). Feature: `microvm-driver-cloud-hypervisor` (GH #42).
> Stories: **US-VM-2**, **US-VM-6**. Job: **J-OPS-003**. Gated by Slice 01.

## Goal (one line)

Every distinct way a VM can fail before the guest runs produces a distinct, named,
actionable operator reason in `overdrive workload describe` — and a `[vm]` + `[service]`
spec is rejected at deploy time naming exactly what is missing.

## Learning hypothesis

`classify_driver_failure`'s already-present-but-unused `DriverType` parameter
(`action_shim/mod.rs:200`, documented at `:193-196` as *"accepted for
forward-compatibility"*) is the correct seam for a second driver's failure vocabulary —
no new mechanism, no change to exec classification.

**Predicted:** four distinct VM failure modes route to four distinct `TransitionReason`
variants through the existing seam, and zero exec test cases change.

> **`superseded-by-DESIGN` (2026-08-11, GH #42) — C-7.** The count is **five, not
> four**. The spike's measured P1 failure is a kernel that **is** found and is
> **not loadable**: CH silently reinterprets it as UEFI firmware and reports
> `VmBoot(UefiLoad(UefiTooBig))` — a firmware **size cap** for what is a
> **format** rejection. The unclassified-verbatim arm below catches it and
> reports CH's text faithfully, which is accurate reporting of a *misleading
> upstream term*: the operator reads a size cap and goes looking at file sizes.
> DESIGN adds **`TransitionReason::VmKernelFormatUnsupported`** (brief § 104,
> ADR-0083 § D5), fed by the **pure** pre-flight `KernelImage::validate(path,
> arch, header)` that runs **before** CH ever sees the file (ADR-0082 § D2); CH's
> verbatim text belongs in `detail`, never in the variant's meaning. The
> **twelve** `TransitionReason::Vm*` cause variants named in ADR-0083 § D5 are the
> governing list; this slice produces its share of them. US-VM-2 / K3's *"no two
> share a variant"* is unaffected and is exceeded.
>
> One further correction: *"no `cloud-hypervisor` on the host"* is a **host**
> property, not a spec property, and its better diagnosis is SD-5's boot probe
> plus an **admission** rejection naming the absent capability (brief § 104). The
> ACs below are unaffected — the deploy still fails, the message improves.

## Thinnest `serve` + `deploy` loop

`overdrive serve` + four deliberately-broken deploys (bad kernel path, bad rootfs path, no
`cloud-hypervisor` on the host, guest init that hangs) + one `[vm]` + `[service]` deploy.
Read `overdrive workload describe` / CLI output for each.

## Behavior (DESIGN owns the API)

- Distinct `TransitionReason` variants for: **kernel artifact not found**, **rootfs
  artifact not found**, **hypervisor binary absent**, **boot deadline exceeded**, and
  — **C-7, added by DESIGN** — **kernel present but not loadable by this hypervisor**
  (`VmKernelFormatUnsupported`, brief § 104 / ADR-0083 § D5). No two share a variant.
- `classify_driver_failure` gains a VM arm routed by its `DriverType` parameter; the exec
  prefix table is untouched.
- Genuinely unclassified failures carry the **verbatim** hypervisor text and are labelled
  unclassified — never dressed up as a known cause
  (`.claude/rules/development.md` § "Distinct failure modes get distinct error variants").
- **US-VM-6:** `[vm]` + `[service]` rejected at deploy time — no intent committed, no
  allocation created — with a message naming guest networking, guest-reachable probes, and
  guest-stack mTLS interception, citing GH
  [#257](https://github.com/overdrive-sh/overdrive/issues/257) (tap-in-netns provisioning +
  guest-reachable probes) and
  [#222](https://github.com/overdrive-sh/overdrive/issues/222) (guest-stack mTLS intercept).
  `[vm]` + `[job]` and `[vm]` + `[schedule]` are accepted.

## Carpaccio taste tests

- **Closes a real loop through production?** Yes — each failure is produced by a real
  `deploy` against a real `serve`, never by constructing a `DriverError` in a test.
- **Thinnest?** Yes — no new subsystem; one match arm plus variants, and one validation
  rejection.
- **Delivers operator-visible value alone?** Yes. This is Ana's stated frustration
  verbatim — *"a diagnosis that requires reading source instead of reading the CLI"* — and
  without it every VM failure is `DriverInternalError { detail: <raw string> }`.
- **Why before lifecycle (Slice 03)?** A workload that fails opaquely is worse than one
  that stops bluntly.

## Acceptance (= US-VM-2 + US-VM-6 ACs)

- [ ] **Five** distinct `TransitionReason` variants exist; none is a catch-all. *(Was
      "four" — **C-7** adds *kernel present but not loadable*; see the correction block
      above.)*
- [ ] **C-7 (added by DESIGN)** — a deploy whose `kernel` path exists but is not a loadable
      image for the target arch reports a **format** error naming the real problem, produced
      by the pure pre-flight validation **before** CH is invoked. It must **not** surface as
      `UefiTooBig`, and CH's verbatim text (when present) appears only in `detail`.
- [ ] VM failures route via the `DriverType` parameter; exec classification is unchanged
      (existing exec tests green, untouched).
- [ ] Each message names the artifact or resource **and** the actionable next step —
      verified by reading `workload describe` output, not by asserting on an enum.
- [ ] Unclassified failures carry verbatim cause text and are labelled unclassified.
- [ ] `[vm]` + `[service]` rejected at deploy time; **no intent committed, no allocation
      created**; message names all three missing capabilities and cites a real issue for
      each — **#257** (tap-in-netns + guest-reachable probes) and **#222** (guest-stack
      mTLS intercept).
- [ ] `[vm]` + `[job]` and `[vm]` + `[schedule]` accepted and run.
- [ ] Every case produced by a real `overdrive deploy`.

## Dependencies

- **Slice 01** (the `[vm]` surface and the VM driver must exist to fail).
- Mirrors the existing `ParseError::ProbesNotAllowedOnKind` precedent
  (`workload_spec.rs:827`) — a semantic rejection with guidance.

## Note on the rejection

The `[vm]` + `[service]` rejection is **removed, not relaxed**, when VM services become
supported. **All three cited gaps now resolve to a real issue** — tap-in-netns provisioning
and guest-reachable probes are GH
[#257](https://github.com/overdrive-sh/overdrive/issues/257) (filed 2026-08-02, closing the
former `[B1]`); guest-stack mTLS interception is
[#222](https://github.com/overdrive-sh/overdrive/issues/222). No named gap in the rejection
message is a hand-wavy forward pointer.

## Note on the failure vocabulary's second consumer

**US-VM-7 (Slice 03) reuses this vocabulary** for its fail-closed case — a host that cannot
supply the `[D7]` confinement produces a distinct, named `Failed` reason rather than a
hypervisor started with confinement silently degraded. That is why Slice 02 lands before
Slice 03: the vocabulary must exist for the confinement story to fail honestly into it.
