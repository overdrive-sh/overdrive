//! Throwaway network-active two-VM fixture; no production agent, identity or policy.
use std::ffi::{c_char, c_int, c_ulong, c_void, CString};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream, UdpSocket};
use std::time::{Duration, Instant};

unsafe extern "C" {
    fn socket(domain:c_int, kind:c_int, protocol:c_int)->c_int;
    fn ioctl(fd:c_int, request:c_ulong, ...)->c_int;
    fn close(fd:c_int)->c_int;
    fn mount(source:*const c_char,target:*const c_char,kind:*const c_char,flags:c_ulong,data:*const c_void)->c_int;
}
#[repr(C)] #[derive(Clone,Copy)]
struct Address { family:u16, data:[u8;14] }
#[repr(C)]
struct Route { pad1:c_ulong, dst:Address, gateway:Address, mask:Address, flags:u16,
    pad2:i16, pad3:c_ulong, pad4:*mut c_void, metric:i16, device:*mut c_char,
    mtu:c_ulong, window:c_ulong, irtt:u16 }
fn address(ip:&str)->Address {
    let mut out=Address{family:2,data:[0;14]};
    out.data[2..6].copy_from_slice(&ip.parse::<Ipv4Addr>().unwrap().octets());out
}
fn checked(rc:c_int,what:&str) { assert!(rc>=0,"{what}: {}",std::io::Error::last_os_error()); }
fn config(ip:&str) {
    unsafe {
        checked(mount(c"proc".as_ptr(),c"/proc".as_ptr(),c"proc".as_ptr(),0,std::ptr::null()),"mount proc");
        checked(mount(c"sysfs".as_ptr(),c"/sys".as_ptr(),c"sysfs".as_ptr(),0,std::ptr::null()),"mount sys");
        let fd=socket(2,2,0);checked(fd,"control socket");
        for (nic,ip,mask) in [("lo","127.0.0.1","255.0.0.0"),("eth0",ip,"255.255.0.0")] {
            let mut ifreq=[0u8;40];ifreq[..nic.len()].copy_from_slice(nic.as_bytes());
            let addr=address(ip);
            std::ptr::copy_nonoverlapping((&addr as *const Address).cast::<u8>(),ifreq[16..].as_mut_ptr(),16);
            checked(ioctl(fd,0x8916,&ifreq),"set guest IP");
            let mask=address(mask);
            std::ptr::copy_nonoverlapping((&mask as *const Address).cast::<u8>(),ifreq[16..].as_mut_ptr(),16);
            checked(ioctl(fd,0x891c,&ifreq),"set guest /16");
            checked(ioctl(fd,0x8913,&mut ifreq),"read guest flags");
            let flags=i16::from_ne_bytes([ifreq[16],ifreq[17]])|1;
            ifreq[16..18].copy_from_slice(&flags.to_ne_bytes());
            checked(ioctl(fd,0x8914,&ifreq),"raise guest NIC");
        }
        let dev=CString::new("eth0").unwrap();
        let mut route:Route=std::mem::zeroed();
        route.dst=address("0.0.0.0");route.gateway=address("100.95.0.1");route.mask=address("0.0.0.0");
        route.flags=3;route.device=dev.as_ptr().cast_mut();
        checked(ioctl(fd,0x890b,&route),"default route");checked(close(fd),"close control socket");
    }
}
fn exchange(destination:&str,label:&str,ip:&str) {
    let start=Instant::now();let request=format!("m295-real-vm:{ip}:{label}\n");
    let mut stream=TcpStream::connect_timeout(&destination.parse().unwrap(),Duration::from_secs(8)).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(8))).unwrap();
    stream.write_all(request.as_bytes()).unwrap();
    let mut reply=vec![0;request.len()];stream.read_exact(&mut reply).unwrap();assert_eq!(reply,request.as_bytes());
    println!("REAL_VM_TCP_PASS label={label} source={ip} destination={destination} bytes={} elapsed_us={}",reply.len(),start.elapsed().as_micros());
}
fn main() {
    // Mount first so the fixture's role is read from the real kernel cmdline.
    unsafe { checked(mount(c"proc".as_ptr(),c"/proc".as_ptr(),c"proc".as_ptr(),0,std::ptr::null()),"mount proc"); }
    let cmd=std::fs::read_to_string("/proc/cmdline").unwrap();
    let b=cmd.contains("spike_role=b");let (ip,peer)=if b {("100.95.4.0","100.95.0.2")} else {("100.95.0.2","100.95.4.0")};
    // config mounts proc again; EBUSY is avoided by leaving the proc mount once here.
    config_without_proc(ip);
    println!("REAL_VM_READY ip={ip}/16 gateway=100.95.0.1 mac={} route_size={}",std::fs::read_to_string("/sys/class/net/eth0/address").unwrap().trim(),std::mem::size_of::<Route>());
    let listener=TcpListener::bind("0.0.0.0:19000").unwrap();
    std::thread::spawn(move || {
        for incoming in listener.incoming() {
            let mut stream=incoming.unwrap();let mut data=[0;512];let n=stream.read(&mut data).unwrap();stream.write_all(&data[..n]).unwrap();
            println!("REAL_VM_PEER_ECHO source={} bytes={n}",stream.peer_addr().unwrap());
        }
    });
    std::thread::sleep(Duration::from_secs(2));
    exchange(&format!("{peer}:19000"),"peer",ip);
    exchange("100.95.0.1:18950","gateway",ip);
    exchange("10.98.0.1:18951","off-subnet-service",ip);
    let dns=UdpSocket::bind("0.0.0.0:0").unwrap();dns.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let query=[0x29,0x50,1,0,0,1,0,0,0,0,0,0,4,b'p',b'e',b'e',b'r',0,0,1,0,1];
    dns.send_to(&query,"100.95.0.1:53").unwrap();let mut reply=[0;512];let (n,from)=dns.recv_from(&mut reply).unwrap();
    assert!(n>=16);assert_eq!(&reply[..2],&query[..2]);assert_eq!(&reply[n-4..n],&peer.parse::<Ipv4Addr>().unwrap().octets());
    println!("REAL_VM_DNS_PASS gateway={from} resolved_peer={peer}");
    println!("REAL_VM_ALL_TRAFFIC_PASS ip={ip}");std::io::stdout().flush().unwrap();
    loop {std::thread::park();}
}
fn config_without_proc(ip:&str) {
    // Kept as fixture-only internal code. The first mount is already complete.
    unsafe extern "C" { fn umount(target:*const c_char)->c_int; }
    unsafe { checked(umount(c"/proc".as_ptr()),"unmount temporary proc"); }
    config(ip);
}
