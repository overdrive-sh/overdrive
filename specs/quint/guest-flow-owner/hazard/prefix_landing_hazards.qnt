// -*- mode: Bluespec; -*-
// Teeth for the prefix_landing state machine (../prefix_landing.qnt, module landing_machine): each
// instance is the design with ONE rule switched off (OFF). Verdicts are in ../checks.toml.

// U-4: route, steering and rules removed when serve goes down
module landing_bugVolatile { import landing_machine(OFF = Set("volatile")).* from "../prefix_landing" }
// D8a / K-D2: no constant divert / reject rules
module landing_bugNoRules { import landing_machine(OFF = Set("noRules")).* from "../prefix_landing" }
// D8a-LOAD-SWAP / D8a-ROUTE: the route is written without attached steering
module landing_bugRouteFirst { import landing_machine(OFF = Set("routeFirst")).* from "../prefix_landing" }
