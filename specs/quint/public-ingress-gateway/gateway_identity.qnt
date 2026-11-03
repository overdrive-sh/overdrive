// -*- mode: Bluespec; -*-
/**
 * Gateway Identity Slot and lifecycle (public-ingress-gateway DESIGN reconstruction, ADR-0225).
 *
 * ONE owner: the node's volatile Gateway Identity Slot, driven by `GatewaySvidLifecycle` (reconcile -> Issue
 * Actions) and the action-shim executor (`dispatch_issue`), controlled by
 * `GatewayIdentityLifecycleControl::{ensure_current, disable_after_drain}`.
 *
 * Rules modelled (feature-delta "Gateway identity and lifecycle integration", "Gateway Identity Slot" aggregate):
 *  - The slot holds at most ONE gateway SVID generation (ADR-0225). `hold` replaces atomically (reissue).
 *  - The slot owns a checked process-local desired epoch: the constructor is epoch 1 and enabled; `begin_disable`
 *    increments the epoch, clears enabled AND clears the volatile slot in the same call (a synchronous clear). Issue
 *    Actions carry the epoch; `hold_audited` rejects a stale epoch as a successful no-op, so an in-flight Issue cannot
 *    re-hold after a disable (ADR-0225 consequence). There is no waiting disable and no drop action: the identity
 *    owner clears at disable.
 *  - `issue_and_audit` writes the issuance audit row BEFORE the hold; an audit failure holds nothing (ADR-0225).
 *  - Enabled + empty or near expiry issues; a restart begins empty (volatile) and reissues.
 *  - `disable_after_drain` (= `begin_disable`) returns Ok at once (the clear is synchronous; there is no waiting disable).
 *
 * Time: `now` is the injected SimClock (model-only). TTL / RENEW abstract the existing workload SVID TTL and
 * half-life policy (reused by ADR-0225); only their ordering matters.
 *
 * Environment actions: audit_fail (a failing ObservationStore write), advance_time, restart.
 *
 * OFF names the rules switched off (hazards in hazard/):
 *   noEpochCheck     hold_audited ignores the epoch (ADR-0225): the one hazard of the decision
 *
 * Model-only: `now`, `next_serial`, `audit_armed`, `issues` (R-31: the `issue_complete` handler fails the step when no
 * Issue of that epoch is pending), ghost `g`, `conf_step`.
 * Conformance projection (accessors pinned by the feature-delta): desired_epoch, desired_enabled <-
 * GatewayIdentitySlot::desired(); held <- GatewayIdentitySlot::current(); audited <- issued_certificate_rows.
 *
 * NOT MODELLED (reported to DESIGN): `NoAllocationIdentity` — no action of the slot or lifecycle can take or produce
 * an AllocationId (the slot API is epoch-keyed, the audit subject is `SpiffeId::for_gateway`), so the property is
 * true by the interface and cannot fail in a behaviour model; it is a type-level property.
 *
 * ASSUMPTIONS: GI-1 `issue_and_audit` is the existing CA issue+audit path whose failure leaves no held material
 * (discharged by the sim/unit scenarios S-PIG-51 and the existing ca_issuance tests); GI-2 the injected clock is
 * monotonic (SimClock).
 */
module gateway_identity_conf {
  type ConfStep =
    | init
    | submit_evaluation
    | issue_complete({ epoch: int })
    | disable
    | advance_time
    | audit_fail
    | restart
}

module gateway_identity {
  import gateway_identity_conf.ConfStep from "./gateway_identity"
  const OFF: Set[str]
  const REC: bool

  val TTL: int = 4          // validity of an issued SVID
  val RENEW: int = 2        // reissue when remaining validity <= RENEW (half-life)
  val MAXT: int = 6         // bound on the injected clock
  val MAXSER: int = 4       // bound on issued serials
  val BUG_NOEPOCH: bool = OFF.contains("noEpochCheck")

  type Gen = { epoch: int, serial: int, na: int }

  var desired_epoch: int
  var desired_enabled: bool
  var held: Set[Gen]
  var audited: Set[int]
  var now: int
  var issues: Set[int]          // in-flight IssueGatewaySvid Actions, by epoch
  var next_serial: int
  var audit_armed: bool
  var g: { staleHold: bool }    // ghost: a hold whose epoch differed from the desired epoch
  var conf_step: ConfStep

  action init = all {
    desired_epoch' = 1, desired_enabled' = true, held' = Set(), audited' = Set(), now' = 0,
    issues' = Set(), next_serial' = 1, audit_armed' = false, g' = { staleHold: false },
    conf_step' = variant("init", ()),
  }

  // GatewaySvidLifecycle::reconcile over (desired, current, now): enabled + (empty or near expiry) => Issue, carrying
  // the desired epoch. (No drop action exists: the identity owner clears synchronously at disable.)
  action submit_evaluation = {
    val nearExpiry = held.forall(h => h.na - now <= RENEW)
    val needIssue = desired_enabled and nearExpiry
    all {
      issues' = if (needIssue) issues.union(Set(desired_epoch)) else issues,
      desired_epoch' = desired_epoch, desired_enabled' = desired_enabled, held' = held, audited' = audited,
      now' = now, next_serial' = next_serial, audit_armed' = audit_armed, g' = g,
      conf_step' = if (REC) variant("submit_evaluation", ()) else conf_step,
    }
  }

  // dispatch_issue: issue_and_audit (audit row before hold), then the epoch-checked hold_audited.
  action issue_complete(epoch: int): bool = {
    val ok = not(audit_armed)
    val gen = { epoch: epoch, serial: next_serial, na: now + TTL }
    val epochOk = BUG_NOEPOCH or epoch == desired_epoch
    val holds = ok and epochOk
    val newHeld = if (holds) Set(gen) else held
    all {
      issues.contains(epoch), next_serial <= MAXSER,
      issues' = issues.exclude(Set(epoch)),
      held' = newHeld,
      audited' = if (ok) audited.union(Set(next_serial)) else audited,
      next_serial' = next_serial + 1,
      audit_armed' = false,
      g' = { staleHold: g.staleHold or (holds and epoch != desired_epoch) },
      desired_epoch' = desired_epoch, desired_enabled' = desired_enabled, now' = now,
      conf_step' = if (REC) variant("issue_complete", { epoch: epoch }) else conf_step,
    }
  }

  // GatewayIdentityLifecycleControl::disable_after_drain = begin_disable: epoch + 1, disabled, and the volatile slot
  // cleared in the same call. Returns Ok at once; no evaluation, no wait. In-flight Issues stay queued and become stale.
  action disable = all {
    desired_enabled,
    desired_epoch' = desired_epoch + 1, desired_enabled' = false,
    held' = Set(),
    audited' = audited, now' = now, issues' = issues, next_serial' = next_serial,
    audit_armed' = audit_armed, g' = g,
    conf_step' = if (REC) variant("disable", ()) else conf_step,
  }

  action advance_time = all {
    now < MAXT,
    now' = now + 1,
    desired_epoch' = desired_epoch, desired_enabled' = desired_enabled, held' = held, audited' = audited,
    issues' = issues, next_serial' = next_serial, audit_armed' = audit_armed, g' = g,
    conf_step' = if (REC) variant("advance_time", ()) else conf_step,
  }

  // environment fault: the next issuance's audit write fails (injected through the Sim ObservationStore)
  action audit_fail = all {
    not(audit_armed),
    audit_armed' = true,
    desired_epoch' = desired_epoch, desired_enabled' = desired_enabled, held' = held, audited' = audited,
    now' = now, issues' = issues, next_serial' = next_serial, g' = g,
    conf_step' = if (REC) variant("audit_fail", ()) else conf_step,
  }

  // environment fault: process restart. The slot is volatile (starts empty, epoch 1, enabled); in-flight Actions
  // die with the process; the audit rows and the CA serial counter are durable.
  action restart = all {
    desired_epoch' = 1, desired_enabled' = true, held' = Set(), issues' = Set(),
    audit_armed' = false,
    audited' = audited, now' = now, next_serial' = next_serial, g' = g,
    conf_step' = if (REC) variant("restart", ()) else conf_step,
  }

  // stutter (QC-1, ruling 7): exhaustive main only; the conformance main (REC = true) never takes it, so no
  // conformance step maps to "no driving-port call".
  action idle = all {
    not(REC),
    desired_epoch' = desired_epoch, desired_enabled' = desired_enabled, held' = held, audited' = audited,
    now' = now, issues' = issues, next_serial' = next_serial, audit_armed' = audit_armed, g' = g,
    conf_step' = conf_step,
  }

  action step = {
    nondet e = 1.to(3).oneOf()
    any {
      submit_evaluation, issue_complete(e), disable, advance_time, audit_fail, restart, idle,
    }
  }

  // ------------------------------------------------------------------ invariants
  // ADR-0225: audit must persist before hold: every held serial has an issuance audit row
  val HoldImpliesAudit: bool = held.forall(h => audited.contains(h.serial))
  // ADR-0225: a stale-epoch Issue never holds; nothing is held for an epoch the slot has disabled
  val StaleEpochNeverHolds: bool = and {
    not(g.staleHold),
    held.forall(h => h.epoch <= desired_epoch),
  }
  // (one slot, one generation: the production slot is an Option, a type-level fact; `hold` replaces atomically)
  // ADR-0225 / D5(c): a disabled slot is empty (disable clears synchronously; a late Issue cannot re-hold)
  val DisabledSlotIsEmpty: bool = (not(desired_enabled)) implies held.size() == 0
  val Safety: bool = and { HoldImpliesAudit, StaleEpochNeverHolds, DisabledSlotIsEmpty }

  // ------------------------------------------------------------------ named runs
  // enabled + empty: one reconcile, one issue, the audited generation is held
  run issue_hold = init
    .then(submit_evaluation).expect(issues == Set(1))
    .then(issue_complete(1))
    .expect(held.size() == 1 and audited == Set(1) and held.forall(h => h.serial == 1 and h.epoch == 1) and Safety)
    .then(submit_evaluation).expect(issues.size() == 0 and Safety)

  // near expiry: a fresh audited generation replaces the old one atomically
  run reissue_near_expiry = init
    .then(submit_evaluation).then(issue_complete(1))
    .then(advance_time).expect(held.size() == 1)
    .then(submit_evaluation).expect(issues.size() == 0)
    .then(advance_time).then(submit_evaluation).expect(issues == Set(1))
    .then(issue_complete(1))
    .expect(held.size() == 1 and held.forall(h => h.serial == 2) and audited == Set(1, 2) and Safety)

  // an Issue is in flight when the node disables: disable clears at once; the stale-epoch completion is a no-op
  run disable_races_inflight_issue = init
    .then(submit_evaluation).expect(issues == Set(1))
    .then(disable).expect(desired_epoch == 2 and not(desired_enabled) and held.size() == 0)
    .then(submit_evaluation).expect(issues == Set(1))
    .then(issue_complete(1))
    .expect(held.size() == 0 and not(g.staleHold) and Safety)

  // an injected audit write failure: nothing is held; the retry audits and holds
  run audit_failure_no_hold = init
    .then(submit_evaluation).then(audit_fail).then(issue_complete(1))
    .expect(held.size() == 0 and audited.size() == 0 and Safety)
    .then(submit_evaluation).then(issue_complete(1))
    .expect(held.size() == 1 and audited == Set(2) and Safety)

  // restart begins empty (volatile), keeps the durable audit rows, reissues with a new serial; then disable clears
  run restart_reissue = init
    .then(submit_evaluation).then(issue_complete(1))
    .expect(held.size() == 1 and Safety)
    .then(restart).expect(held.size() == 0 and audited == Set(1) and desired_epoch == 1)
    .then(submit_evaluation).then(issue_complete(1))
    .expect(held.size() == 1 and held.forall(h => h.serial == 2) and audited == Set(1, 2) and Safety)
    .then(disable)
    .expect(held.size() == 0 and desired_epoch == 2 and not(desired_enabled) and audited == Set(1, 2) and Safety)
}

// Main instance: the approved rules. TLC checks Safety exhaustively.
module identity_ok { import gateway_identity(OFF = Set(), REC = false).* from "./gateway_identity" }
// Conformance main: records `conf_step`; only the named runs are checked on it.
module identity_conf { import gateway_identity(OFF = Set(), REC = true).* from "./gateway_identity" }
