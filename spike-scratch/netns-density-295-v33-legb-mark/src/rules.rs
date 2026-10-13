//! S7: install the fwmark FIB rule and the table-100 local route through the REAL production
//! netlink client calls (`Client::add_fib_rule_fwmark`, `Client::add_local_route`), exactly as
//! `overdrive-worker::mtls_intercept::{ensure_fwmark_rule, ensure_local_route}` do.
use overdrive_netlink::Client;
#[tokio::main(flavor = "current_thread")]
async fn main() {
    let c = Client::new().expect("netlink client");
    // mtls_intercept.rs TPROXY_FWMARK = 0x1, TPROXY_RT_TABLE = 100
    if !c.fib_rule_fwmark_present(0x1, 100).await.expect("dump") {
        c.add_fib_rule_fwmark(0x1, 100).await.expect("rule add");
    }
    c.add_local_route(100, "lo").await.expect("local route");
    println!("production client: fwmark 0x1 -> table 100 rule + local default dev lo table 100 installed");
}
