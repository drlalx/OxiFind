#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[cfg(windows)]
fn embed_windows_icon() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("resources/oxifind.ico");
        if let Err(err) = resource.compile() {
            println!("cargo:warning=Failed to embed Windows icon resource: {err}");
        }
    }
}

#[cfg(not(windows))]
fn embed_windows_icon() {}

fn main() {
    println!("cargo:rerun-if-changed=packaging/windows/oxifind.exe.manifest");
    println!("cargo:rerun-if-changed=resources/oxifind.ico");

    embed_windows_icon();

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
