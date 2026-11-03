//! Native helper process boundary; all Windows handles stay outside portable Rust.
pub fn command() -> Result<std::process::Command, String> {
    use std::os::windows::process::CommandExt;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut directory = exe.parent().ok_or("missing executable directory")?;
    if directory
        .file_name()
        .is_some_and(|name| name == "examples" || name == "deps")
    {
        directory = directory.parent().ok_or("missing build directory")?;
    }
    let path = directory.join("shuttli-windows.exe");
    let mut command = std::process::Command::new(path);
    command.env("SHUTTLI_PRODUCT_NAME", shuttli_brand::NAME);
    command.env("SHUTTLI_PRODUCT_VERSION", shuttli_brand::VERSION);
    command.creation_flags(0x08000000); // CREATE_NO_WINDOW; Forms UI remains visible.
    Ok(command)
}
