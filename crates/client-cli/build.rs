fn main() {
    use std::path::PathBuf;
    use std::process::Command;

    // Get current date in YY.MM format
    let date = Command::new("date")
        .args(["+%y.%m"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "00.00".to_string());

    // Get commit count
    let commit_count = Command::new("git")
        .args(["rev-list", "--count", "HEAD"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "0".to_string());

    // Generate version: YY.MM.COMMITCOUNT
    let version = format!("{}.{}", date, commit_count);

    println!("cargo:rustc-env=APAS_VERSION={}", version);
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs/heads/");

    // Also export the same version for web builds (read by packages/web/next.config.ts).
    // This works with the deployment flow where Rust build happens before web rsync/build.
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let web_version_path = manifest_dir.join("../../packages/web/.apas-version");
    let _ = std::fs::write(web_version_path, format!("{}\n", version));
}
