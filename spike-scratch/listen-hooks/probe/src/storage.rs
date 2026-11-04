//! Verbatim copy of the guest's ACTIVE_CORRELATION staging-map helpers
//! (crates/overdrive-init/src/transport/kernel.rs:523-648) so the spike loads the
//! copied guest object the way overdrive-init does.
use std::ffi::CString;
use std::io;
use std::os::fd::{FromRawFd, OwnedFd, AsRawFd};
use std::path::Path;

pub fn create_active_storage(pin: &Path) -> io::Result<()> {
    let btf = load_active_btf()?;
    let map = create_active_map(btf.as_raw_fd())?;
    pin_map(map.as_raw_fd(), pin)
}

fn load_active_btf() -> io::Result<OwnedFd> {
    let strings = b"\0int\0active\0";
    let mut types = Vec::new();
    types.extend_from_slice(&1_u32.to_ne_bytes());
    types.extend_from_slice(&(1_u32 << 24).to_ne_bytes());
    types.extend_from_slice(&4_u32.to_ne_bytes());
    types.extend_from_slice(&32_u32.to_ne_bytes());
    types.extend_from_slice(&5_u32.to_ne_bytes());
    types.extend_from_slice(&(4_u32 << 24).to_ne_bytes());
    types.extend_from_slice(&24_u32.to_ne_bytes());
    let mut btf = Vec::new();
    btf.extend_from_slice(&0xeb9f_u16.to_ne_bytes());
    btf.push(1);
    btf.push(0);
    btf.extend_from_slice(&24_u32.to_ne_bytes());
    btf.extend_from_slice(&0_u32.to_ne_bytes());
    let type_length = u32::try_from(types.len()).map_err(invalid_btf_length)?;
    let string_length = u32::try_from(strings.len()).map_err(invalid_btf_length)?;
    btf.extend_from_slice(&type_length.to_ne_bytes());
    btf.extend_from_slice(&type_length.to_ne_bytes());
    btf.extend_from_slice(&string_length.to_ne_bytes());
    btf.extend_from_slice(&types);
    btf.extend_from_slice(strings);
    let mut attr = [0_u8; 128];
    put64(&mut attr, 0, btf.as_ptr() as u64);
    put32(&mut attr, 16, u32::try_from(btf.len()).map_err(invalid_btf_length)?);
    bpf_syscall(18, &mut attr).and_then(owned_fd)
}

fn create_active_map(btf_fd: i32) -> io::Result<OwnedFd> {
    let mut attr = [0_u8; 128];
    put32(&mut attr, 0, 24);
    put32(&mut attr, 4, 4);
    put32(&mut attr, 8, 24);
    put32(&mut attr, 12, 0);
    put32(&mut attr, 16, 1);
    attr[28..43].copy_from_slice(b"ACTIVE_CORREL\0\0");
    put32(&mut attr, 48, btf_fd.cast_unsigned());
    put32(&mut attr, 52, 1);
    put32(&mut attr, 56, 2);
    bpf_syscall(0, &mut attr).and_then(owned_fd)
}

fn pin_map(fd: i32, path: &Path) -> io::Result<()> {
    let path = CString::new(path.as_os_str().as_encoded_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "bpffs path contains NUL"))?;
    let mut attr = [0_u8; 128];
    put64(&mut attr, 0, path.as_ptr() as u64);
    put32(&mut attr, 8, fd.cast_unsigned());
    bpf_syscall(6, &mut attr).map(|_| ())
}

fn invalid_btf_length(error: std::num::TryFromIntError) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, error)
}

fn owned_fd(raw: i64) -> io::Result<OwnedFd> {
    let raw =
        i32::try_from(raw).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    Ok(unsafe { OwnedFd::from_raw_fd(raw) })
}

fn bpf_syscall(command: libc::c_long, attr: &mut [u8; 128]) -> io::Result<i64> {
    let result = unsafe { libc::syscall(libc::SYS_bpf, command, attr.as_mut_ptr(), attr.len()) };
    if result < 0 { Err(io::Error::last_os_error()) } else { Ok(result as i64) }
}

fn put32(attr: &mut [u8; 128], offset: usize, value: u32) {
    attr[offset..offset + 4].copy_from_slice(&value.to_ne_bytes());
}

fn put64(attr: &mut [u8; 128], offset: usize, value: u64) {
    attr[offset..offset + 8].copy_from_slice(&value.to_ne_bytes());
}
