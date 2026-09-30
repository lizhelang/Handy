fn main() {
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_NATIVE_CODE_VERIFICATION");
    if std::env::var_os("CARGO_FEATURE_NATIVE_CODE_VERIFICATION").is_none() {
        return;
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        panic!("native-code-verification requires macOS; no simulated native adapter is available");
    }
    let architecture = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("aarch64") => "arm64",
        Ok("x86_64") => "x86_64",
        _ => panic!("unsupported native updater target"),
    };
    let out =
        std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo output directory"));
    let source = std::path::PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest directory"),
    )
    .join("../../native/inputia-install-support/InputiaInstallSupport.swift");
    println!("cargo:rerun-if-changed={}", source.display());
    let quiescence = source.with_file_name("InputiaWriterQuiescence.swift");
    println!("cargo:rerun-if-changed={}", quiescence.display());
    let object = out.join("InputiaInstallSupport.o");
    let result = std::process::Command::new("/usr/bin/swiftc")
        .args([
            "-parse-as-library",
            "-emit-object",
            "-O",
            "-whole-module-optimization",
            "-module-name",
            "InputiaInstallSupport",
            "-target",
        ])
        .arg(format!("{architecture}-apple-macos13.0"))
        .arg(&source)
        .arg(&quiescence)
        .arg("-o")
        .arg(&object)
        .status()
        .expect("run build-time Swift compiler");
    assert!(result.success(), "compile native read-only verifier");
    let result = std::process::Command::new("/usr/bin/libtool")
        .arg("-static")
        .arg("-o")
        .arg(out.join("libinputia_install_support.a"))
        .arg(&object)
        .status()
        .expect("archive native verifier");
    assert!(result.success(), "archive native verifier");
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-search=native=/usr/lib/swift");
    println!("cargo:rustc-link-lib=static=inputia_install_support");
    for framework in ["Foundation", "Security", "CryptoKit"] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
}
