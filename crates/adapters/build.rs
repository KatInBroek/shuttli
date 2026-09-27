fn main() {
    let (source, framework, name) = ("Clipboard.swift", "AppKit", "shuttli-pasteboard");
    println!("cargo:rerun-if-changed=native/macos/{source}");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join(name);
        let result = std::process::Command::new("swiftc")
            .args(["-O", "-framework", framework])
            .arg(format!("native/macos/{source}"))
            .arg("-o")
            .arg(out)
            .status()
            .expect("swiftc from Xcode Command Line Tools is required");
        assert!(result.success(), "{name} helper build failed");
    }
}
