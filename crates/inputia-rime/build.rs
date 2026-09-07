use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=INPUTIA_STATIC_RIME_DIR");
    println!("cargo:rerun-if-env-changed=MACOSX_DEPLOYMENT_TARGET");
    if std::env::var_os("CARGO_FEATURE_BUNDLED_STATIC_RIME").is_none() {
        return;
    }
    assert_eq!(
        std::env::var("CARGO_CFG_TARGET_OS").as_deref(),
        Ok("macos"),
        "bundled-static-rime currently requires the verified macOS artifact"
    );
    let architecture = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("aarch64") => "arm64",
        Ok("x86_64") => "x86_64",
        _ => panic!("unsupported bundled-static-rime architecture"),
    };
    let minimum = std::env::var("MACOSX_DEPLOYMENT_TARGET").unwrap_or_default();
    assert!(matches!(minimum.as_str(), "13" | "13.0" | "13.0.0"),
        "bundled-static-rime requires explicit MACOSX_DEPLOYMENT_TARGET=13.0 for all native dependencies");
    let directory = PathBuf::from(std::env::var_os("INPUTIA_STATIC_RIME_DIR")
        .expect("bundled-static-rime requires INPUTIA_STATIC_RIME_DIR; automatic download/fallback is forbidden"));
    let checker = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap())
        .join("../../native/static-rime/verify_link_input.py");
    println!("cargo:rerun-if-changed={}", checker.display());
    for file in ["artifact_binding.py", "sources.lock.json", "probe/main.cpp"] {
        println!(
            "cargo:rerun-if-changed={}",
            checker.parent().unwrap().join(file).display()
        );
    }
    for file in [
        "lib/libinputia_rime_static.a",
        "manifest.json",
        "sources.lock.json",
        "source-snapshot.json",
        "static-rime-probe",
        "include/rime_api.h",
    ] {
        println!("cargo:rerun-if-changed={}", directory.join(file).display());
    }
    let output = Command::new("/usr/bin/python3")
        .arg(&checker)
        .arg(&directory)
        .arg(architecture)
        .output()
        .expect("cannot execute read-only static Rime artifact validation");
    assert!(
        output.status.success(),
        "invalid bundled-static-rime input: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    println!(
        "cargo:rustc-link-search=native={}",
        directory.join("lib").display()
    );
    // 保留 Rime 模块 constructor。导出给 Swift 的 staticlib 仍须由最终链接器 force_load。
    println!("cargo:rustc-link-lib=static:+whole-archive=inputia_rime_static");
    println!("cargo:rustc-link-lib=dylib=c++");
}
