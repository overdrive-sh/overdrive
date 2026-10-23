// -*- mode: Bluespec; -*-
// Teeth for TCP-CORRELATION: weaken the combined pending-bound rule by allowing the correlation collection to
// refuse a correlated child while a shared pending place is free. The exhaustive Safety check and the existing
// ready-at-accept run must both expose the weakening.
module pending_bugCapacityRefusal {
  import tcp_capture_pending(OFF = Set("capacityRefusal")).* from "../tcp_capture_pending"
}
