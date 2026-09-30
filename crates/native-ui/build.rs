fn main() {
    println!("cargo:rerun-if-changed=macos/ShuttliUI.swift");
    println!("cargo:rerun-if-changed=../../branding/name.txt");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("shuttli-ui");
        let brand = out.with_file_name("BrandedUI.swift");
        // Byte literals preserve Unicode, quotes and backslashes without Swift interpolation.
        std::fs::write(
            &brand,
            format!(
                "enum ProductBrand {{ static let name = String(decoding: {:?}, as: UTF8.self); static let version = String(decoding: {:?}, as: UTF8.self) }}\n{}",
                shuttli_brand::NAME.as_bytes(),
                shuttli_brand::VERSION.as_bytes(),
                std::fs::read_to_string("macos/ShuttliUI.swift").expect("read Swift UI")
            ),
        )
        .expect("write Swift product name");
        let result = std::process::Command::new("swiftc")
            .arg(&brand)
            .args(["-O", "-framework", "SwiftUI", "-framework", "AppKit", "-o"])
            .arg(out)
            .status()
            .expect("swiftc required");
        assert!(result.success(), "native UI build failed");
    }
}
