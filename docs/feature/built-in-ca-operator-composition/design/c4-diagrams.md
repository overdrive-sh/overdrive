# C4 Diagrams — `built-in-ca-operator-composition`

Mermaid C4. System Context (L1) + Container (L2) are mandatory; a Component
diagram (L3) is included for the CA boot-composition seam because it crosses
four driven ports with Earned-Trust probes.

## L1 — System Context

```mermaid
C4Context
  title System Context — Built-in CA Operator Composition
  Person(operator, "Operator", "Runs overdrive serve; reads alloc status")
  System(overdrive, "Overdrive control plane", "Mints, holds, audits, and renders workload SVIDs")
  System_Ext(systemd, "systemd-creds / kernel keyring", "Supplies the operator KEK")
  System_Ext(workload, "Running workloads", "Consume the held SVID via the dataplane")

  Rel(operator, overdrive, "Starts via 'overdrive serve'; reads 'alloc status' from")
  Rel(overdrive, systemd, "Resolves KEK from")
  Rel(overdrive, workload, "Issues + holds chain-verifiable SVID for")
  Rel(operator, overdrive, "Reads current SVID summary (serial/spiffe/issuer/not_after) from")
```

## L2 — Container

```mermaid
C4Container
  title Container Diagram — Built-in CA Operator Composition
  Person(operator, "Operator")
  System_Ext(systemd, "systemd-creds / keyring", "KEK source")

  Container_Boundary(cp, "overdrive control plane (single binary)") {
    Container(cli, "overdrive-cli", "Rust", "serve + alloc-status verbs; renders SVID summary")
    Container(boot, "run_server boot root", "Rust", "Wires + probes the persistent CA before use")
    Container(svidlc, "SvidLifecycle reconciler", "Rust (core)", "running vs held; emits IssueSvid (first-issue, restart, near-expiry rotate)")
    Container(exec, "IssueSvid executor", "Rust (action-shim)", "Mints + audits leaf; holds it")
    ContainerDb(intent, "IntentStore (redb)", "redb", "KEK-sealed root-key envelope + public cert material")
    ContainerDb(obs, "ObservationStore", "Corrosion/CR-SQLite", "issued_certificates append-only audit rows")
  }

  Rel(operator, cli, "Invokes")
  Rel(cli, boot, "Starts")
  Rel(boot, systemd, "Resolves KEK from (probe a)")
  Rel(boot, intent, "Seals + persists / loads + decrypts root under KEK (probe b)")
  Rel(boot, svidlc, "Composes adopted CA + IdentityMgr into")
  Rel(svidlc, exec, "Emits IssueSvid action to")
  Rel(exec, obs, "Writes issued_certificates audit row to")
  Rel(cli, obs, "Aggregates max-issuance_ordinal row per running alloc from")
```

## L3 — Component (CA boot-composition seam)

Justified: the seam crosses four driven ports with two Earned-Trust probes and
a fail-closed adopt-or-refuse invariant — the highest-risk boundary in the
feature.

```mermaid
C4Component
  title Component Diagram — boot_ca / bootstrap_node_intermediate seam
  Container_Boundary(boot, "run_server boot root") {
    Component(bootca, "boot_ca", "fn", "Generate-or-load persistent root; probe KEK + envelope")
    Component(bootint, "bootstrap_node_intermediate", "fn", "Generate-or-load node intermediate; adopt on restart")
    Component(err, "ControlPlaneError::CaBoot", "enum variant", "Typed refuse-to-start; preserves distinct CaError cause")
  }
  Component_Ext(kek, "SystemdCredsKeyring : Kek", "adapter", "resolve(kek_id)")
  Component_Ext(codec, "RootKeyAeadCodec", "adapter", "seal / open AES-GCM envelope")
  Component_Ext(ca, "RcgenCa : Ca", "adapter", "root / issue_intermediate / adopt_persisted_*")
  ComponentDb(intent, "IntentStore", "redb", "root-key envelope + cert material")

  Rel(bootca, kek, "Probe (a): resolve KEK from; refuse if absent")
  Rel(bootca, intent, "Loads sealed envelope from / persists to")
  Rel(bootca, codec, "Probe (b): open envelope under KEK; refuse if fails")
  Rel(bootca, ca, "Generates root / adopts persisted root into")
  Rel(bootint, ca, "Signs / adopts node intermediate into")
  Rel(bootint, intent, "Loads / persists intermediate envelope + cert material")
  Rel(bootca, err, "Emits health.startup.refused + returns on probe failure")
  Rel(bootint, err, "Emits health.startup.refused + returns on probe failure")
```
