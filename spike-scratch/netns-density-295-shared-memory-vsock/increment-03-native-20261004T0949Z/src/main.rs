//! Throwaway PROBE ONLY: real AF_VSOCK in Linux guests, AF_UNIX in CH host backend.
use std::{fs, io::{self, Read, Write}, net::{TcpListener, TcpStream}, os::{fd::{AsRawFd, FromRawFd}, unix::net::{UnixListener, UnixStream}}, thread, time::{Duration, Instant}};

fn line(stream: &mut impl Read) -> io::Result<String> {
    let mut bytes=Vec::new();
    loop { let mut b=[0]; if stream.read(&mut b)?==0 {break;} bytes.push(b[0]); if b[0]==b'\n' {break;} if bytes.len()>8192 {return Err(io::Error::other("line bound"));} }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}
fn vsock(cid:u32,port:u32,listen:bool)->io::Result<fs::File> {
    unsafe {
        let fd=libc::socket(libc::AF_VSOCK,libc::SOCK_STREAM|libc::SOCK_CLOEXEC,0);
        if fd<0 {return Err(io::Error::last_os_error());}
        let file=fs::File::from_raw_fd(fd);
        let address=libc::sockaddr_vm{svm_family:libc::AF_VSOCK as _,svm_reserved1:0,svm_port:port,svm_cid:cid,svm_zero:[0;4]};
        if listen {
            if libc::bind(fd,&address as *const _ as *const _,size_of::<libc::sockaddr_vm>() as _)!=0 || libc::listen(fd,32)!=0 {return Err(io::Error::last_os_error());}
        } else {
            libc::fcntl(fd,libc::F_SETFL,libc::O_NONBLOCK);
            let rc=libc::connect(fd,&address as *const _ as *const _,size_of::<libc::sockaddr_vm>() as _);
            if rc!=0 && io::Error::last_os_error().raw_os_error()!=Some(libc::EINPROGRESS) {return Err(io::Error::last_os_error());}
            if rc!=0 {
                let mut p=libc::pollfd{fd,events:libc::POLLOUT,revents:0};
                if libc::poll(&mut p,1,2000)<=0 {return Err(io::Error::new(io::ErrorKind::TimedOut,"vsock connect 2s"));}
                let mut err=0i32;let mut len=size_of::<i32>() as libc::socklen_t;
                if libc::getsockopt(fd,libc::SOL_SOCKET,libc::SO_ERROR,&mut err as *mut _ as *mut _,&mut len)!=0 {return Err(io::Error::last_os_error());}
                if err!=0 {return Err(io::Error::from_raw_os_error(err));}
            }
            libc::fcntl(fd,libc::F_SETFL,0);
        }
        let timeout=libc::timeval{tv_sec:3,tv_usec:0};
        for opt in [libc::SO_RCVTIMEO,libc::SO_SNDTIMEO] {libc::setsockopt(fd,libc::SOL_SOCKET,opt,&timeout as *const _ as *const _,size_of::<libc::timeval>() as _);}
        Ok(file)
    }
}
fn accept(socket:&fs::File)->io::Result<fs::File> {
    let fd=unsafe{libc::accept4(socket.as_raw_fd(),std::ptr::null_mut(),std::ptr::null_mut(),libc::SOCK_CLOEXEC)};
    if fd<0 {Err(io::Error::last_os_error())} else {Ok(unsafe{fs::File::from_raw_fd(fd)})}
}
fn guest()->io::Result<()> {
    unsafe {for (src,dst,kind) in [(c"proc",c"/proc",c"proc"),(c"sysfs",c"/sys",c"sysfs"),(c"devtmpfs",c"/dev",c"devtmpfs")] {libc::mount(src.as_ptr(),dst.as_ptr(),kind.as_ptr(),0,std::ptr::null());}}
    let cmd=fs::read_to_string("/proc/cmdline")?;
    let role=cmd.split_whitespace().find_map(|s|s.strip_prefix("spike_role=")).unwrap_or("unknown").to_owned();
    let cid:u32=cmd.split_whitespace().find_map(|s|s.strip_prefix("spike_cid=")).unwrap().parse().unwrap();
    println!("GUEST_IDENTITY role={role} expected_cid={cid} pid={} source_sha={} exe={:?}",std::process::id(),option_env!("SPIKE_SOURCE_SHA").unwrap_or("missing"),fs::read_link("/proc/self/exe"));
    unsafe {
        let mut uts:libc::utsname=std::mem::zeroed();libc::uname(&mut uts);
        println!("GUEST_UNAME {} {} {}",std::ffi::CStr::from_ptr(uts.sysname.as_ptr()).to_string_lossy(),std::ffi::CStr::from_ptr(uts.release.as_ptr()).to_string_lossy(),std::ffi::CStr::from_ptr(uts.machine.as_ptr()).to_string_lossy());
        for path in ["/modules/vsock.ko","/modules/vmw_vsock_virtio_transport_common.ko","/modules/vmw_vsock_virtio_transport.ko"] {
            let file=fs::File::open(path)?;let rc=libc::syscall(libc::SYS_finit_module,file.as_raw_fd(),c"".as_ptr(),0);
            println!("GUEST_MODULE_LOAD path={path} rc={rc} errno={:?}",io::Error::last_os_error().raw_os_error());
            if rc!=0 && io::Error::last_os_error().raw_os_error()!=Some(libc::EEXIST){return Err(io::Error::last_os_error());}
        }
        let device=fs::File::open("/dev/vsock")?;let mut actual_cid=0u32;let rc=libc::ioctl(device.as_raw_fd(),0x7b9,&mut actual_cid);
        println!("GUEST_LOCAL_CID role={role} ioctl_rc={rc} expected={cid} actual={actual_cid} matched={}",actual_cid==cid);
        let socket=libc::socket(libc::AF_INET,libc::SOCK_DGRAM|libc::SOCK_CLOEXEC,0);let mut req:libc::ifreq=std::mem::zeroed();req.ifr_name[0]=b'l' as _;req.ifr_name[1]=b'o' as _;
        if libc::ioctl(socket,libc::SIOCGIFFLAGS,&mut req)!=0{return Err(io::Error::last_os_error());}
        req.ifr_ifru.ifru_flags|=libc::IFF_UP as i16;
        if libc::ioctl(socket,libc::SIOCSIFFLAGS,&req)!=0{return Err(io::Error::last_os_error());}libc::close(socket);
    }
    println!("GUEST_NET_INTERFACES {:?}",fs::read_dir("/sys/class/net")?.map(|e|e.unwrap().file_name()).collect::<Vec<_>>());
    println!("GUEST_MEMINFO {}",fs::read_to_string("/proc/meminfo")?);
    println!("GUEST_MODULES {}",fs::read_to_string("/proc/modules")?);
    let app=TcpListener::bind("127.0.0.1:8080")?;
    let app_role=role.clone();thread::spawn(move||{for incoming in app.incoming() {let mut s=incoming.unwrap();let mut b=[0;4096];let n=s.read(&mut b).unwrap();let body=format!("ordinary-http-{app_role}");let reply=format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());s.write_all(reply.as_bytes()).unwrap();println!("GUEST_TCP_HTTP role={app_role} request_bytes={n}");}});
    let incoming=vsock(u32::MAX,5001,true)?;
    let incoming_role=role.clone();thread::spawn(move||loop {match accept(&incoming) {Ok(mut channel)=>{let mut request=[0;4096];let n=channel.read(&mut request).unwrap();let mut app=TcpStream::connect("127.0.0.1:8080").unwrap();app.write_all(&request[..n]).unwrap();let mut reply=Vec::new();app.read_to_end(&mut reply).unwrap();channel.write_all(&reply).unwrap();println!("GUEST_HOST_TO_TCP role={incoming_role} request_bytes={n} reply_bytes={}",reply.len());},Err(e)=>println!("GUEST_ACCEPT_ERROR {e}")}});
    let wrapper=TcpListener::bind("127.0.0.1:8081")?;
    thread::spawn(move||for incoming in wrapper.incoming(){let mut app=incoming.unwrap();let request=line(&mut app).unwrap();let mut channel=vsock(2,5000,false).unwrap();channel.write_all(request.as_bytes()).unwrap();let reply=line(&mut channel).unwrap();app.write_all(reply.as_bytes()).unwrap();});
    println!("GUEST_LISTEN_READY role={role} cid={cid} vsock_port=5001 local_http=8080 local_wrapper=8081");
    let mut app=TcpStream::connect("127.0.0.1:8081")?;
    let request=format!("guest-to-host-{role}-cid-{cid}\n");let start=Instant::now();app.write_all(request.as_bytes())?;let reply=line(&mut app)?;
    let expected=format!("host-echo-{role}:{request}");println!("GUEST_TCP_WRAPPER_RESULT role={role} passed={} elapsed_us={} request={request:?} reply={reply:?}",reply==expected,start.elapsed().as_micros());
    let sibling=if role=="a"{39502}else{39501};
    for (label,dest,port) in [("unregistered-port",2,5009),("unregistered-cid",49999,5001),("sibling-cid",sibling,5001)] {
        let start=Instant::now();match vsock(dest,port,false){Ok(_)=>println!("GUEST_DENIAL role={role} label={label} cid={dest} port={port} denied=false elapsed_us={}",start.elapsed().as_micros()),Err(e)=>println!("GUEST_DENIAL role={role} label={label} cid={dest} port={port} denied=true errno={:?} error={e:?} elapsed_us={}",e.raw_os_error(),start.elapsed().as_micros())}
    }
    println!("GUEST_PROBE_READY role={role} transport_result={}",reply==expected);
    loop {thread::sleep(Duration::from_secs(1));}
}
fn host(paths:&[String])->io::Result<()> {
    let mut handles=Vec::new();
    for (index,base) in paths.iter().enumerate(){let role=if index==0{"a"}else{"b"};let path=format!("{base}_5000");let listener=UnixListener::bind(&path)?;println!("HOST_RECEIVER_ARMED role={role} path={path}");handles.push(thread::spawn(move||for incoming in listener.incoming(){let mut stream=incoming.unwrap();let start=Instant::now();let request=line(&mut stream).unwrap();let reply=format!("host-echo-{role}:{request}");stream.write_all(reply.as_bytes()).unwrap();println!("HOST_RECEIVE role={role} elapsed_us={} bytes={} payload={request:?}",start.elapsed().as_micros(),request.len());}));}
    println!("HOST_ALL_RECEIVERS_READY");
    for h in handles {h.join().unwrap();}Ok(())
}
fn main(){let args:Vec<_>=std::env::args().collect();let result=if args.get(1).map(String::as_str)==Some("host"){host(&args[2..])}else{guest()};if let Err(e)=result{println!("PROBE_ERROR {e:?}");loop{thread::sleep(Duration::from_secs(1));}}}
