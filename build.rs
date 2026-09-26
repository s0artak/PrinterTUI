//! Windows builds embed pdfium.dll (bblanchon/pdfium-binaries, BSD/Apache licensed), which draws
//! PDF pages for the printer; the exe unpacks it on first use. Set PDFIUM_DLL to use a local copy.

use sha2::{Digest, Sha256};
use std::process::Command;

// chromium/7881 matches pdfium-render's `pdfium_7881` feature
const RELEASE: &str = "https://github.com/bblanchon/pdfium-binaries/releases/download/chromium%2F7881";
const SHA256: [(&str, &str); 2] = [
    ("x64", "73cc0de638ac2095e7445bf56a38200a5b7c7ca0e9f4ba144598f2457377ac08"),
    ("arm64", "d3035d4d2cacac6ecd1a2ece197a3d702a1b2a58466276b9f870b8cb278a9d84"),
];

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-env-changed=PDFIUM_DLL");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let dll = out.join("pdfium.dll");
    if let Ok(local) = std::env::var("PDFIUM_DLL") {
        std::fs::copy(local, &dll).expect("PDFIUM_DLL: cannot copy");
        return;
    }
    if dll.exists() {
        return;
    }
    let arch = if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("aarch64") { "arm64" } else { "x64" };
    let tgz = out.join(format!("pdfium-win-{arch}.tgz"));
    let ok = |c: &mut Command| c.status().is_ok_and(|s| s.success());
    // curl and tar ship with Windows 10+ as well as Linux and macOS
    assert!(ok(Command::new("curl").args(["-fsSL", "-o"]).arg(&tgz).arg(format!("{RELEASE}/pdfium-win-{arch}.tgz"))), "cannot download pdfium");
    let hash: String = Sha256::digest(std::fs::read(&tgz).unwrap()).iter().map(|b| format!("{b:02x}")).collect();
    let want = SHA256.iter().find(|(a, _)| *a == arch).unwrap().1;
    assert_eq!(hash, want, "pdfium download does not match its SHA-256");
    assert!(ok(Command::new("tar").arg("xzf").arg(&tgz).arg("-C").arg(&out).arg("bin/pdfium.dll")), "cannot unpack pdfium");
    std::fs::rename(out.join("bin").join("pdfium.dll"), &dll).unwrap();
}
