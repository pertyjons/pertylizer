fn main() {
    // Embed build date as YYYY-MM-DD
    let now = std::process::Command::new("date")
        .args(["+%Y-%m-%d"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default();
    println!("cargo:rustc-env=BUILD_DATE={}", now.trim());
}
