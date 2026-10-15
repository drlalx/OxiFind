#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use std::process::Command;

const ALLOWED_LICENSES: &[&str] = &[
    "MIT",
    "Apache-2.0",
    "Apache-2.0 WITH LLVM-exception",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "ISC",
    "0BSD",
    "Zlib",
    "Unicode-3.0",
    "BSL-1.0",
    "OFL-1.1",
    "Ubuntu-font-1.0",
    "Unlicense",
    "CC0-1.0",
];

fn is_mit_compatible(expression: &str) -> bool {
    let normalized = expression.replace('/', " OR ");
    for and_part in normalized.split(" AND ") {
        let has_allowed_or = and_part
            .split(" OR ")
            .map(|part| part.trim().trim_matches(['(', ')']))
            .any(|part| ALLOWED_LICENSES.contains(&part));

        if !has_allowed_or {
            return false;
        }
    }
    true
}

#[test]
fn only_mit_compatible_dependencies_are_used() {
    let output = Command::new("cargo")
        .args(["metadata", "--format-version", "1"])
        .output()
        .expect("cargo metadata must execute successfully");

    assert!(output.status.success(), "cargo metadata failed");

    let stdout = String::from_utf8(output.stdout).expect("metadata must be valid utf-8");
    // Parse json without pulling heavy extra dependencies into the main binary
    // Simple deserialization using standard regex or serde_json if available, or manual line parsing
    let mut incompatible = Vec::new();

    // Parse each package from cargo metadata output
    for line in stdout.split("{\"name\":\"") {
        if let Some(pkg_end) = line.find("\"") {
            let pkg_name = &line[..pkg_end];
            if pkg_name == "oxifind" || pkg_name.is_empty() {
                continue;
            }
            if let Some(lic_start) = line.find("\"license\":\"") {
                let rest = &line[lic_start + 11..];
                if let Some(lic_end) = rest.find("\"") {
                    let lic = &rest[..lic_end];
                    if !is_mit_compatible(lic) {
                        incompatible.push((pkg_name.to_owned(), lic.to_owned()));
                    }
                }
            } else if let Some(lic_null) = line.find("\"license\":null") {
                // Check if license is null
                if lic_null < line.find("\"manifest_path\"").unwrap_or(usize::MAX) {
                    incompatible.push((pkg_name.to_owned(), "NO_LICENSE".to_owned()));
                }
            }
        }
    }

    assert!(
        incompatible.is_empty(),
        "Found non-MIT-compatible dependencies: {incompatible:?}"
    );
}

#[test]
fn test_license_compatibility_logic() {
    assert!(is_mit_compatible("MIT"));
    assert!(is_mit_compatible("MIT OR Apache-2.0"));
    assert!(is_mit_compatible("Apache-2.0 / MIT"));
    assert!(is_mit_compatible("Apache-2.0 OR GPL-2.0-only"));
    assert!(is_mit_compatible(
        "(MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0"
    ));
    assert!(!is_mit_compatible("GPL-3.0-only"));
    assert!(!is_mit_compatible("AGPL-3.0-only"));
    assert!(!is_mit_compatible("Proprietary"));
}
