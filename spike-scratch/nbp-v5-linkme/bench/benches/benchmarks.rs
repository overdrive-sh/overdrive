// harness = false bench main. Counts registry entries. Variant selected by feature-free
// cfg: the "referenced" variant is the default (extern crate); build with
// RUSTFLAGS cfg `nbp_unreferenced` to drop the reference and observe the dead-strip risk.
#[cfg(not(nbp_unreferenced))]
extern crate nbp_decls;
fn main() {
    println!("bench binary sees {} entries: {:?}", nbp_lib::count(), nbp_lib::names());
}
