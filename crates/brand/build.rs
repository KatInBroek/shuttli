fn main() {
    println!("cargo:rerun-if-changed=../../branding/name.txt");
    let raw = std::fs::read_to_string("../../branding/name.txt").expect("read product name");
    let name = raw.trim_end_matches(['\r', '\n']);
    assert!(
        !name.is_empty()
            && name.len() <= 128
            && name.trim() == name
            && !name.chars().any(char::is_control),
        "product name must be 1..128 UTF-8 bytes, without controls or surrounding whitespace"
    );
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    std::fs::write(
        out.join("brand.rs"),
        format!("pub const NAME: &str = {name:?};\n"),
    )
    .unwrap();
}
