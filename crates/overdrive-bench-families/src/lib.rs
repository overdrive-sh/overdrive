//! The benchmark families (ADR-0296). One module per family under `families/`.

pub mod families {
    pub mod cgroup_connect_hook;
    pub mod dns_responder;
    pub mod mtls_path;
    pub mod vm_lifecycle;
}
