use std::process::Command;

fn main() {
    // Generate backend server version using the same scheme as CLI:
    // YY.MM.COMMITCOUNT
    let date = Command::new("date")
        .args(["+%y.%m"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "00.00".to_string());

    let commit_count = Command::new("git")
        .args(["rev-list", "--count", "HEAD"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "0".to_string());

    let version = format!("{}.{}", date, commit_count);
    println!("cargo:rustc-env=APAS_SERVER_VERSION={}", version);
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs/heads/");
}
