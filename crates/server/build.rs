use std::process::Command;

fn main() {
    // Generate backend server version using the same scheme as CLI:
    // YY.MM.COMMITCOUNT_THIS_MONTH
    let date = Command::new("date")
        .args(["+%y.%m"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "00.00".to_string());

    // Month start timestamp for current month (YYYY-MM-01 00:00:00)
    let month_start = Command::new("date")
        .args(["+%Y-%m-01 00:00:00"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();

    // Commit count since month start
    let commit_count = if month_start.is_empty() {
        "0".to_string()
    } else {
        Command::new("git")
            .arg("rev-list")
            .arg("--count")
            .arg(format!("--since={month_start}"))
            .arg("HEAD")
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_else(|_| "0".to_string())
    };

    let version = format!("{}.{}", date, commit_count);
    println!("cargo:rustc-env=APAS_SERVER_VERSION={}", version);
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs/heads/");
}
