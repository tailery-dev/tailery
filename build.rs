use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=.git/HEAD");

    // Extract build date in UTC
    let build_date = Command::new("date")
        .args(["-u", "+%Y-%m-%d"])
        .output()
        .ok()
        .and_then(|out| {
            if out.status.success() {
                String::from_utf8(out.stdout).ok()
            } else {
                None
            }
        })
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=VERGEN_BUILD_DATE={}", build_date.trim());

    // Extract git describe (e.g. v0.1.0-4-g1234abc)
    let git_describe = Command::new("git")
        .args(["describe", "--tags", "--always", "--dirty"])
        .output()
        .ok()
        .and_then(|out| {
            if out.status.success() {
                String::from_utf8(out.stdout).ok()
            } else {
                None
            }
        })
        .unwrap_or_else(|| "v0.1.0".to_string());
    println!(
        "cargo:rustc-env=VERGEN_GIT_DESCRIBE={}",
        git_describe.trim()
    );

    // Extract short git SHA
    let git_sha = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|out| {
            if out.status.success() {
                String::from_utf8(out.stdout).ok()
            } else {
                None
            }
        })
        .unwrap_or_else(|| "dev".to_string());
    println!("cargo:rustc-env=VERGEN_GIT_SHA={}", git_sha.trim());
}
