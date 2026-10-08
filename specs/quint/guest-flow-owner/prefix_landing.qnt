// -*- mode: Bluespec; -*-
/**
 * Guest-flow owner (netns-density-295) — shared definition: where a connection to a workload
 * address in the guest prefix lands. Pure; no state. Used by the four steering modules
 * (prefix_boot, intake, quiescence, shared_table).
 *
 * Source of truth: feature-delta.md § "vsock Attachment Replacement DESIGN" (D8a-LOAD-SWAP):
 * § Driven port — guest-prefix steering ("Decision (the program, constant)"), § Formal protocol
 * model (assumption table); ADR-0171 (D8a-LOOKUP), ADR-0152 (D8a). The route has two forms,
 * `absent` and `local` (the fence form was removed in revision 10).
 *
 * Client classes: `marked` = Overdrive's own marked clients (leg-S, probes; exempt from the nft
 * divert / reject); `unmarked` = any other host-local client; `remote` = a packet arriving on a
 * non-loopback device. A wildcard host service is assumed bound on EVERY port (worst case).
 * Outcomes: intake | legC | refused | wildcard (a host service: exposure) | elsewhere (routed off
 * the node).
 *
 * ASSUMPTIONS encoded here (discharged in README.md):
 *   K-D1  `local` route and no lookup decision → a wildcard host listener unless a listener bound
 *         to workload_addr:p exists ............................................................ V-19
 *   K-D2  nft: non-exempt prefix traffic is diverted to leg-C, else rejected / dropped; a divert
 *         with no leg-C socket falls through to the reject ........................................ V-19
 *   K-D4  no tagged route → the prefix is not local: routed elsewhere unless nft diverts / rejects V-19
 *   K-L1  sk_lookup decides every loopback-ingress lookup to a `local` prefix address ......... V-26
 *   K-L2  a leg-C TPROXY-assigned packet bypasses sk_lookup ..................................... V-26
 *   K-L5  ingress is loopback for host-local connects, the receiving device otherwise ......... V-26
 */
module prefix_landing {
  type Net = {
    marked: bool,       // Overdrive's marked clients exist (serve is serving)
    legC: bool,         // the leg-C socket exists
    divert: bool,       // nft: prefix traffic diverted to leg-C (output + prerouting)
    reject: bool,       // nft: output reject / prerouting drop of non-exempt prefix traffic
    route: str,         // absent | local  (the prefix's Overdrive-tagged route)
    steer: str,         // none | ours  (the pinned sk_lookup program, if attached; the shipped program is correct, V-26)
    entries: Set[int],  // ports with a steering entry
    lis: Set[int],      // ports with an open intake listener
    assignRemote: bool, // hazard hook: the program also assigns non-loopback ingress
  }

  val CLIENTS: Set[str] = Set("marked", "unmarked", "remote")

  // the socket lookup for a connection that reached local delivery
  def lookup(n: Net, p: int, loopback: bool): str =
    if (n.steer == "ours") {
      if ((loopback or n.assignRemote) and n.entries.contains(p) and n.lis.contains(p)) "intake"
      else "refused"                                     // dropped; TCP answered with a reset
    } else if (n.lis.contains(p)) "intake" else "wildcard" // no decision: K-D1

  def lands(n: Net, c: str, p: int): str =
    if (c == "marked" and not(n.marked)) "refused"          // no such client
    else if (c != "marked" and n.divert and n.legC) "legC"   // K-D2 divert (K-L2: no lookup)
    else if (c != "marked" and n.reject) "refused"           // K-D2 reject / drop
    else if (n.route == "absent") "elsewhere"                // K-D4
    else lookup(n, p, c != "remote")

  def exposedOn(n: Net, ports: Set[int]): bool =
    CLIENTS.exists(c => ports.exists(p => lands(n, c, p) == "wildcard"))
  def elsewhereOn(n: Net, ports: Set[int]): bool =
    CLIENTS.exists(c => ports.exists(p => lands(n, c, p) == "elsewhere"))
  def deliveredOn(n: Net, ports: Set[int]): bool =
    CLIENTS.exists(c => ports.exists(p => lands(n, c, p).in(Set("intake", "wildcard"))))
}
