#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

fn main() {
    println!("cargo:rerun-if-changed=packaging/windows/oxifind.exe.manifest");

    let target = std::env::var("TARGET").unwrap_or_default();
    if target.contains("windows")
        && target.contains("msvc")
        && let Ok(manifest_path) = std::fs::canonicalize("packaging/windows/oxifind.exe.manifest")
    {
        println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}",
            manifest_path.display()
        );
    }
}
