use std::process::Command;

fn main() {
    // Get current date in YY-MM-DD format
    let date = Command::new("date")
        .args(["+%y-%m-%d"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "00-00-00".to_string());

    // Get commit count
    let commit_count = Command::new("git")
        .args(["rev-list", "--count", "HEAD"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "0".to_string());

    // Generate version: YY-MM-DD-COMMITCOUNT
    let version = format!("{}-{}", date, commit_count);

    println!("cargo:rustc-env=APAS_VERSION={}", version);
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs/heads/");
}
