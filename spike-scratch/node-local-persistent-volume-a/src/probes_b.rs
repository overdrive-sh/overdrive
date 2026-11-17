//! Probe B: single writer. Throwaway.
use super::*;

pub fn disk_a(path: &str, extra: &str) -> String {
    let mut a = format!("path={path},image_type=raw");
    if !extra.is_empty() { a.push(','); a.push_str(extra); }
    a
}

/// Start a second VMM on `disk` while `first` runs; report whether it booted.
pub fn second_vmm(name: &str, disk: &str, hold_secs: u64) -> (bool, String) { second_vmm_ch(name, "stock", disk, hold_secs) }
pub fn second_vmm_ch(name: &str, ch: &str, disk: &str, hold_secs: u64) -> (bool, String) {
    let mut v = idle_vmm(name, ch, &[disk.to_string()]);
    let ready = wait_for_line(&mut v, "READY", Duration::from_secs(hold_secs));
    let exited = v.exited();
    let err = tail_ch(&v);
    v.cleanup();
    (ready, format!("ready={ready} exited_early={exited:?} stderr=[{err}]"))
}

pub fn lock_timeline(label: &str, path: &str, vmm: &mut Vmm, bound: Duration) -> (Option<u128>, Option<u128>) {
    // Poll the lock state and the process state every ~0.5ms; return (t_exit, t_unlocked) in us since start.
    let t0 = now_us();
    let (mut t_exit, mut t_unlock) = (None, None);
    let mut first_state = None;
    while now_us() - t0 < bound.as_micros() {
        if t_exit.is_none() && vmm.exited().is_some() { t_exit = Some(now_us() - t0); }
        let st = ofd_getlk(path);
        if first_state.is_none() { first_state = Some(st.clone()); }
        if st.is_none() && t_unlock.is_none() { t_unlock = Some(now_us() - t0); }
        if t_exit.is_some() && t_unlock.is_some() { break; }
        std::thread::sleep(Duration::from_micros(500));
    }
    ev(format!("LOCKTIMELINE {label}: first_lock_state={:?} t_exit_us={t_exit:?} t_unlock_us={t_unlock:?}", first_state.flatten()));
    (t_exit, t_unlock)
}
