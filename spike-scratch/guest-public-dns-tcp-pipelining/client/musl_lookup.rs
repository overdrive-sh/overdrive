// musl-static getaddrinfo(AF_UNSPEC) client: Rust std's ToSocketAddrs calls libc getaddrinfo (musl here).
use std::net::ToSocketAddrs;
fn main() {
    let name = std::env::args().nth(1).expect("name");
    let t = std::time::Instant::now();
    let r: Result<Vec<_>, _> = (name.as_str(), 80).to_socket_addrs().map(|i| i.collect());
    println!("musl getaddrinfo({name}) -> {:?} in {:?}", r, t.elapsed());
}
