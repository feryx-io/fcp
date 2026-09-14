//! Where another crate's binary is. Cargo names a crate's own binaries
//! in `CARGO_BIN_EXE_<name>` for its integration tests, and nothing
//! else's; the conference tests start the call manager too. The order:
//! `FCP_E2E_BIN_DIR` when set, then the target directory the test
//! executable itself lives under (`cargo test --all-targets`, CI's
//! invocation, has built every binary there), then a `cargo build` of
//! the one binary, once per process.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

static BUILD_LOCK: Mutex<()> = Mutex::new(());

/// The path of binary `name` from package `package`.
pub fn path(name: &str, package: &str) -> PathBuf {
    if let Ok(dir) = std::env::var("FCP_E2E_BIN_DIR") {
        let p = Path::new(&dir).join(name);
        assert!(
            p.is_file(),
            "FCP_E2E_BIN_DIR is set but {} is not there",
            p.display()
        );
        return p;
    }
    let target_dir = target_dir();
    let p = target_dir.join(name);
    if p.is_file() {
        return p;
    }
    let _guard = BUILD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if p.is_file() {
        return p;
    }
    let profile = if target_dir.ends_with("release") {
        Some("--release")
    } else {
        None
    };
    let mut cmd = Command::new(env!("CARGO"));
    cmd.args(["build", "-p", package, "--bin", name]);
    if let Some(flag) = profile {
        cmd.arg(flag);
    }
    let status = cmd
        .status()
        .unwrap_or_else(|e| panic!("cargo build {package} --bin {name}: {e}"));
    assert!(
        status.success(),
        "cargo build -p {package} --bin {name} failed"
    );
    assert!(
        p.is_file(),
        "built {package} but {} is not there",
        p.display()
    );
    p
}

/// The profile directory the running test executable belongs to:
/// `target/<profile>`, one up from `deps`.
fn target_dir() -> PathBuf {
    let exe = std::env::current_exe().expect("current_exe");
    let deps = exe.parent().expect("test executable has a directory");
    if deps.file_name().is_some_and(|n| n == "deps") {
        deps.parent().expect("deps has a parent").to_path_buf()
    } else {
        deps.to_path_buf()
    }
}
