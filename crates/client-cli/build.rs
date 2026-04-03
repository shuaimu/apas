fn main() {
    use std::process::Command;

    // Get current date in YY.MM format
    let date = Command::new("date")
        .args(["+%y.%m"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "00.00".to_string());

    // Get month start timestamp for current month (YYYY-MM-01 00:00:00)
    let month_start = Command::new("date")
        .args(["+%Y-%m-01 00:00:00"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();

    // Get commit count since month start
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

    // Generate version: YY.MM.COMMITCOUNT_THIS_MONTH
    let version = format!("{}.{}", date, commit_count);

    println!("cargo:rustc-env=APAS_VERSION={}", version);
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs/heads/");
}
