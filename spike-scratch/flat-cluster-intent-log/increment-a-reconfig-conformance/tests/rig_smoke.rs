//! Smoke: learn viewstamp's real message flow under the rig (printed with --no-capture).
use core::time::Duration;
use vs_reconfig_conformance::rig::Rig;

#[test]
fn normal_op_and_view_change_flow() {
  let mut r = Rig::new(3, &[0, 1, 2], &[], 4);
  let mark = r.net.len();
  println!("--- genesis emitted:\n{}", r.net_trace(0));
  r.client_request(0, 11);
  println!("--- after Request to n0:\n{}", r.net_trace(mark));
  let mark = r.net.len();
  r.deliver_all(50);
  println!("--- after deliver_all:\n{}{}", r.net_trace(mark), r.status_line());
  // heartbeat
  let mark = r.net.len();
  r.advance(0, Duration::from_millis(60));
  println!("--- after primary +60ms:\n{}", r.net_trace(mark));
  r.deliver_all(50);
  println!("{}", r.status_line());
  // crash primary, let backup 1 time out
  r.crash(0);
  let mark = r.net.len();
  for _ in 0..3 {
    r.fire_next_timer(1);
  }
  println!("--- n1 timers after primary crash:\n{}{}", r.net_trace(mark), r.status_line());
  let mark = r.net.len();
  for _ in 0..40 {
    r.deliver_all(50);
    r.fire_next_timer(1);
    r.fire_next_timer(2);
  }
  println!("--- view change settle:\n{}{}", r.net_trace(mark), r.status_line());
  assert!(r.panics.is_empty(), "{:?}", r.panics);
}
