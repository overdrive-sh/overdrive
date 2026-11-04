//! Shared by guest and host (via #[path]): the deterministic LISTEN sequence both sides agree on.
pub fn expected(idx: u64) -> (&'static str, u16) {
    (if idx % 2 == 0 { "tcp" } else { "udp" }, 1 + (idx % 65_000) as u16)
}
pub fn listen_line(idx: u64) -> String {
    let (p, port) = expected(idx);
    format!("LISTEN {p} {port}\n")
}
