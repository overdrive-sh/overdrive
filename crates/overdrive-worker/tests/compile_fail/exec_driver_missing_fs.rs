//! Compile-fail fixture: `ExecDriver::new` MUST reject a call that
//! omits the `fs: Arc<dyn CgroupFs>` parameter.
//!
//! Activated at step 01-05 alongside the `ExecDriver::new` arity
//! change. The `.stderr` companion pins the rustc arity-mismatch
//! diagnostic so future drift (e.g. an accidentally-defaulted `fs`
//! parameter, or a builder shape that makes `fs` optional) is caught
//! at PR time.

use std::path::PathBuf;
use std::sync::Arc;

use overdrive_core::traits::clock::Clock;
use overdrive_worker::ExecDriver;

fn _missing_fs(clock: Arc<dyn Clock>) {
    // 2-arg call against the post-01-05 3-arg signature — must NOT
    // compile.
    let _driver = ExecDriver::new(PathBuf::from("/"), clock);
}

fn main() {}
