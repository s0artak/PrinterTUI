//! Windows builds embed pdfium.dll (bblanchon/pdfium-binaries, BSD/Apache licensed), which draws
//! PDF pages for the printer; the exe unpacks it on first use. Set PDFIUM_DLL to use a local copy.

use sha2::{Digest, Sha256};
use std::process::Command;

fn build_number() -> String {
    for key in ["BUILD_NUMBER", "GITHUB_RUN_NUMBER"] {
        if let Some(number) = std::env::var(key).ok().and_then(|value| value.parse::<u64>().ok()) {
            return number.to_string();
        }
    }
    Command::new("git")
        .args(["rev-list", "--count", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|number| number.trim().parse::<u64>().ok())
        .map_or_else(|| "0".into(), |number| number.to_string())
}

fn build_date() -> String {
    let override_date = std::env::var("BUILD_DATE").ok().filter(|date| {
        date.len() == 10
            && date.as_bytes()[4] == b'-'
            && date.as_bytes()[7] == b'-'
            && date.bytes().enumerate().all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit())
    });
    let iso = override_date.unwrap_or_else(|| {
        let epoch = std::env::var("SOURCE_DATE_EPOCH")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or_else(|| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
            });
        let days = (epoch / 86_400) as i64;
        let z = days + 719_468;
        let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
        let day_of_era = z - era * 146_097;
        let year_of_era = (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
        let mut year = year_of_era + era * 400;
        let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
        let month_prime = (5 * day_of_year + 2) / 153;
        let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
        let month = month_prime + if month_prime < 10 { 3 } else { -9 };
        if month <= 2 {
            year += 1;
        }
        format!("{year:04}-{month:02}-{day:02}")
    });
    let [year, month, day] = iso.split('-').collect::<Vec<_>>().try_into().unwrap();
    format!("{day}/{month}/{year}")
}

// chromium/7881 matches pdfium-render's `pdfium_7881` feature
const RELEASE: &str = "https://github.com/bblanchon/pdfium-binaries/releases/download/chromium%2F7881";
const SHA256: [(&str, &str); 2] = [
    ("x64", "73cc0de638ac2095e7445bf56a38200a5b7c7ca0e9f4ba144598f2457377ac08"),
    ("arm64", "d3035d4d2cacac6ecd1a2ece197a3d702a1b2a58466276b9f870b8cb278a9d84"),
];

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-changed=src");
    println!("cargo::rerun-if-env-changed=PDFIUM_DLL");
    println!("cargo::rerun-if-env-changed=BUILD_NUMBER");
    println!("cargo::rerun-if-env-changed=GITHUB_RUN_NUMBER");
    println!("cargo::rerun-if-env-changed=BUILD_DATE");
    println!("cargo::rerun-if-env-changed=SOURCE_DATE_EPOCH");
    println!("cargo::rustc-env=PRINTERTUI_BUILD_NUMBER={}", build_number());
    println!("cargo::rustc-env=PRINTERTUI_BUILD_DATE={}", build_date());
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    println!("cargo::rerun-if-changed=assets/printertui.rc");
    println!("cargo::rerun-if-changed=assets/printertui.ico");
    let _ = embed_resource::compile("assets/printertui.rc", embed_resource::NONE);
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
