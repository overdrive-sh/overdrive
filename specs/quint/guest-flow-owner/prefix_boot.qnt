// -*- mode: Bluespec; -*-
/**
 * Guest-flow owner (netns-density-295) — steering concern 1: boot convergence of the pinned
 * guest-prefix steering and the prefix's tagged route (D8a-ROUTE, D8a-LOAD-SWAP, U-4).
 *
 * Design artifact and DISTILL oracle. Models the design text, not an implementation.
 * Source of truth: feature-delta.md § "vsock Attachment Replacement DESIGN" — D8a-LOAD-SWAP
 * (user-approved 2026-10-08: the runtime probe and boot `verify` removed), D8a-ROUTE, D8a-LOOKUP,
 * U-4; § Composition (boot step 9), § Owner and provisioner (`converge_shared`), § Lifecycle
 * ("Guest prefix"), § Typed error taxonomy, § Formal protocol model (concern 1, invariant 6);
 * ADR-0171, ADR-0152.
 *
 * The rules modelled:
 *   - boot step 9 runs load → swap → route. `load` loads this binary's program (the kernel verifier
 *     may reject it) and changes no attached kernel object. `swap` is the only step that changes the
 *     link's program: an adopted pinned link gets it atomically (BPF_LINK_UPDATE, K-L9) — never a
 *     detach; with no pin, the program is attached (link process-scoped) and then pinned (two
 *     sub-steps: `swap`, `pin`). `route` keeps or adds the tagged `local` route.
 *   - Any failure of load / swap / pin / the route write → startup refuses
 *     `GuestPrefixSteeringConverge`; the route is not written and the link keeps its earlier
 *     program (K-L9; nothing is attached after a failed load).
 *   - The shipped program is assumed correct (A-33, discharged by the Tier-3 steering scenarios R5-1,
 *     R5-2, R5-3, R5-25 in CI): a program that loads is a correct program, so the model has no defective-program state.
 *   - the route has two forms, `absent` and `local`; route, pinned link and map persist across a
 *     crash and graceful shutdown (U-4, K-D3, K-L4); nothing at runtime writes the route or the
 *     program (A-31), so this module has no runtime.
 *
 * Abstraction:
 *   - Boot steps 1–8 are one step (`start`); the nft rules are taken as ABSENT throughout (worst case:
 *     the design claims the steering alone keeps host services unreachable; concern 4 models the
 *     table). No intake listener exists (concern 2), so a lookup the program decides is refused.
 *   - Program versions BINS (v1, v2), so a boot can replace an earlier boot's program.
 *   - Faults (all Overdrive's own): a `serve` crash between any two steps and inside swap (between
 *     the attach and pin sub-steps of a first attach; a graceful shutdown or fail-stop has the same
 *     effect here); a scripted failure of load (verifier rejection), swap, pin or the route write.
 *
 * OFF names design rules switched off (hazard instances in hazard/prefix_boot_hazards.qnt):
 *   routeFirst     D8a-ROUTE: the route step runs before load / swap
 *   ignoreFailure  D8a-ROUTE / D8a-LOAD-SWAP: a load or swap failure is ignored and boot continues
 *                  to the route (no loaded program: swap keeps the link)
 *   detach         D8a-LOOKUP / D8a-LOAD-SWAP: swap detaches the pinned link and attaches anew
 *                  (a non-atomic swap)
 *   unpinned       D8a-LOOKUP / ADR-0159: the steering is process-owned (never pinned)
 * CFG selects the instance's fault bounds (see below).
 */
module prefix_boot {
  import prefix_landing.* from "./prefix_landing"

  const OFF: Set[str]
  const CFG: str   // "ok" | "faulty" | "live"

  def bug(r: str): bool = OFF.contains(r)
  // fault bounds (-1 = unbounded)
  val BINS: Set[str] = Set("v1", "v2")
  val CRASHES: int = if (CFG == "live") 1 else -1
  val FAILS: int = if (CFG == "faulty") -1 else 1
  val PORTS: Set[int] = Set(1)

  type Ghost = {
    refused: bool,          // a boot refused (witness)
    loadRefused: bool,      // a boot refused at load (verifier rejection) (witness)
    updated: bool,          // a boot replaced a pinned link's program by a different one (witness)
    detached: bool,         // a boot detached the pinned link
    routeUnswapped: bool,   // `local` written while this boot's swap had not returned Ok
    crashLocal: bool,       // serve went down with the route `local` (witness)
    crashAfterSwap: bool,   // serve went down right after a swap under `local`, before route (witness)
    crashMidAttach: bool,   // serve went down between a first attach and its pin (witness)
    swapFailKept: bool,     // a failed swap under `local` kept the earlier program (witness)
  }
  val g0: Ghost = { refused: false, loadRefused: false, updated: false, detached: false,
                    routeUnswapped: false, crashLocal: false, crashAfterSwap: false,
                    crashMidAttach: false, swapFailKept: false }

  var up: bool
  var phase: str      // start | load | swap | pin | route | open
  var stop: str       // init | crash | refused
  var bin: str        // this boot's program version
  var link: str       // none | attached (process-scoped, unpinned) | pinned
  var prog: str       // the program the link runs ("none" without a link)
  var loaded: bool    // this boot's program loaded
  var swapOk: bool    // this boot's swap returned Ok
  var adopted: bool   // this boot found a pinned link and updated it
  var route: str      // absent | local
  var estab: bool     // history: the tagged route was written at least once
  var failLeft: int
  var crashLeft: int
  var g: Ghost

  val open: bool = up and phase == "open"
  val net: Net = {
    marked: open, legC: false, divert: false, reject: false, route: route,
    steer: if (link == "none") "none" else "ours",
    entries: Set(), lis: Set(), assignRemote: false,
  }
  def dec(x: int): int = if (x < 0) x else x - 1
  def avail(x: int): bool = x != 0

  def nextPhase(ph: str): str =
    if (bug("routeFirst")) {
      if (ph == "start") "route" else if (ph == "route") "load" else if (ph == "load") "swap"
      else if (ph == "swap") "pin" else "open"
    } else {
      if (ph == "start") "load" else if (ph == "load") "swap" else if (ph == "swap") "pin"
      else if (ph == "pin") "route" else "open"
    }

  action keepCounters = all { failLeft' = failLeft, crashLeft' = crashLeft }
  action keepRoute = all { route' = route, estab' = estab }
  action keepSteer = all { link' = link, prog' = prog }
  action keepBoot = all { up' = true, stop' = stop, bin' = bin }
  // serve exits: process-scoped objects vanish (an unpinned link with them); node-global ones stay
  action exitSteer = all {
    link' = if (link == "attached") "none" else link,
    prog' = if (link == "attached") "none" else prog,
  }

  // a serve (re)start runs a binary; the first boot starts here too
  action Restart = {
    nondet b = BINS.oneOf()
    all {
      not(up),
      up' = true, phase' = "start", stop' = "none", bin' = b, loaded' = false, swapOk' = false,
      adopted' = false, keepSteer, keepRoute, keepCounters, g' = g,
    }
  }
  action init = all {
    up' = false, phase' = "start", stop' = "init", bin' = "none", link' = "none", prog' = "none",
    loaded' = false, swapOk' = false, adopted' = false, route' = "absent", estab' = false,
    failLeft' = FAILS, crashLeft' = CRASHES, g' = g0,
  }

  // the owner's boot progress (one step of boot step 9, or steps 1–8 as one)
  action BootStep = all {
    up, phase != "open", keepCounters,
    if (phase == "swap" and link != "none" and bug("detach") and not(adopted)) all {
      // hazard: detach the pinned link first (then attach anew)
      link' = "none", prog' = "none", adopted' = true, phase' = "swap", keepBoot, keepRoute,
      loaded' = loaded, swapOk' = swapOk, g' = { ...g, detached: true },
    } else if (phase == "swap" and loaded) all {
      // adopt + BPF_LINK_UPDATE (atomic program replace), or attach a new (unpinned) link
      link' = if (link == "none") "attached" else link,
      prog' = bin,
      adopted' = (adopted or link == "pinned"),
      phase' = nextPhase(phase), keepBoot, loaded' = loaded, swapOk' = true, keepRoute,
      g' = { ...g, updated: g.updated or (link == "pinned" and prog != bin) },
    } else if (phase == "swap") all {
      // no loaded program (reachable only when ignoreFailure ignored a load failure): the link keeps
      // its program
      keepSteer, adopted' = adopted, phase' = nextPhase(phase), keepBoot, loaded' = loaded,
      swapOk' = swapOk, keepRoute, g' = g,
    } else if (phase == "pin") all {
      // the second sub-step of a first attach; a no-op for an adopted link
      link' = if (link == "attached" and not(bug("unpinned"))) "pinned" else link,
      prog' = prog, adopted' = adopted, phase' = nextPhase(phase), keepBoot, loaded' = loaded,
      swapOk' = swapOk, keepRoute, g' = g,
    } else if (phase == "route") all {
      // keep / add the tagged `local` route
      route' = "local", estab' = true, keepSteer, adopted' = adopted, phase' = nextPhase(phase),
      keepBoot, loaded' = loaded, swapOk' = swapOk,
      g' = { ...g, routeUnswapped: g.routeUnswapped or not(swapOk) },
    } else if (phase == "load") all {
      // the verifier accepts this binary's program; no attached kernel object changes
      phase' = nextPhase(phase), keepBoot, keepSteer, adopted' = adopted, loaded' = true,
      swapOk' = swapOk, keepRoute, g' = g,
    } else all {
      // start (boot steps 1–8)
      phase' = nextPhase(phase), keepBoot, keepSteer, adopted' = adopted, loaded' = loaded,
      swapOk' = swapOk, keepRoute, g' = g,
    },
  }
  // a scripted failure of a boot-9 step: refuse, the route not written. load: the verifier rejects
  // the program (nothing attached); swap: the link keeps the earlier program (K-L9); pin: the new
  // link stays unpinned and vanishes with the process; route: the write fails (route unchanged).
  // ignoreFailure: a load or swap failure is ignored.
  action Fail = all {
    up, phase.in(Set("load", "swap", "pin", "route")), avail(failLeft),
    bin' = bin, adopted' = adopted, crashLeft' = crashLeft, failLeft' = dec(failLeft),
    if (bug("ignoreFailure") and phase.in(Set("load", "swap"))) all {
      phase' = nextPhase(phase), up' = true, stop' = stop, loaded' = loaded, swapOk' = swapOk,
      keepSteer, keepRoute, g' = g,
    } else all {
      up' = false, phase' = "start", stop' = "refused", exitSteer, keepRoute,
      loaded' = false, swapOk' = false,
      g' = { ...g, refused: true, loadRefused: g.loadRefused or phase == "load",
             swapFailKept: g.swapFailKept or (phase == "swap" and link == "pinned" and route == "local") },
    },
  }
  // serve dies (crash, graceful shutdown, a fail-stop): node-global objects persist (U-4)
  action Crash = all {
    up, avail(crashLeft),
    up' = false, phase' = "start", stop' = "crash", exitSteer, keepRoute, bin' = bin,
    loaded' = false, swapOk' = false, adopted' = false,
    failLeft' = failLeft, crashLeft' = dec(crashLeft),
    g' = { ...g, crashLocal: g.crashLocal or route == "local",
           crashAfterSwap: g.crashAfterSwap or
             (adopted and route == "local" and phase.in(Set("pin", "route"))),
           crashMidAttach: g.crashMidAttach or (phase == "pin" and link == "attached") },
  }

  action step = any { Restart, BootStep, Fail, Crash }

  // ------------------------------------------------------------------ invariants
  // (6, D8a-ROUTE, M-4) the route is `local` only while a PINNED steering link with a loaded program
  // exists (a first link attached and not yet pinned does not count)
  val LocalOnlyOverLink: bool = route == "local" implies (link == "pinned" and prog != "none")
  // (6) ... in particular whenever serve is down (crash, refusal, fail-stop)
  val LocalOnlyOverLinkWhileDown: bool = not(up) implies LocalOnlyOverLink
  // (D8a-ROUTE) a boot writes `local` only after its own swap returned Ok
  val RouteOnlyAfterSwap: bool = not(g.routeUnswapped)
  // (D8a-LOOKUP, D8a-ROUTE, K-L1) no connection to the prefix reaches a host service — with the nft
  // rules absent (the steering alone)
  val NoWildcardReached: bool = not(exposedOn(net, PORTS))
  // (6, K-L4) while serve is down nothing to the prefix is delivered to a host socket
  val NothingDeliveredWhileDown: bool = not(up) implies not(deliveredOn(net, PORTS))
  // (U-4) once Overdrive has written the tagged route, prefix traffic never leaves the node
  val NoElsewhere: bool = estab implies not(elsewhereOn(net, PORTS))
  // (D8a-LOOKUP, K-L9) swap never detaches the pinned link
  val NeverDetached: bool = not(g.detached)
  // (D8a-LOAD-SWAP, D8a-ROUTE) a boot opens only converged: its own program runs in the pinned link
  // and the route is `local` — so a crash at any step leaves a state the next boot converges
  val OpenConverged: bool = open implies (link == "pinned" and prog == bin and route == "local")

  val Safety: bool = and {
    LocalOnlyOverLink, RouteOnlyAfterSwap, NoWildcardReached, NothingDeliveredWhileDown,
    NoElsewhere, NeverDetached, OpenConverged,
  }

  // ------------------------------------------------------------------ witnesses (expected violated)
  val WitnessNoRefusal: bool = not(g.refused)
  val WitnessNoLoadRefused: bool = not(g.loadRefused)
  val WitnessNoUpdate: bool = not(g.updated)
  val WitnessNoCrashWithLocal: bool = not(g.crashLocal)
  val WitnessNoCrashAfterSwap: bool = not(g.crashAfterSwap)
  val WitnessNoCrashMidAttach: bool = not(g.crashMidAttach)
  val WitnessNoSwapFailKept: bool = not(g.swapFailKept)

  // ------------------------------------------------------------------ liveness (TLC)
  val bootVars = (up, phase, link, route)
  // boot opens once load and swap eventually succeed (bounded failures and crashes)
  temporal BootOpens =
    and { weakFair(BootStep, bootVars), weakFair(Restart, bootVars) }.implies(eventually(open))
}

// ------------------------------------------------------------------ design instances
module boot_ok { import prefix_boot(OFF = Set(), CFG = "ok").* from "./prefix_boot" }
module boot_faulty { import prefix_boot(OFF = Set(), CFG = "faulty").* from "./prefix_boot" }
module boot_live { import prefix_boot(OFF = Set(), CFG = "live").* from "./prefix_boot" }
