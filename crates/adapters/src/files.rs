use shuttli_ports::sync::Result;
use std::{
    io::Write,
    path::{Path, PathBuf},
};
pub fn data_dir() -> Result<PathBuf> {
    let dir = if let Some(p) = std::env::var_os("SHUTTLI_DATA_DIR") {
        PathBuf::from(p)
    } else {
        let home = std::env::var_os("HOME").ok_or("HOME unavailable")?;
        if cfg!(target_os = "macos") {
            PathBuf::from(home).join("Library/Application Support/Shuttli")
        } else {
            std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(home).join(".local/share"))
                .join("shuttli")
        }
    };
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }
    Ok(dir)
}
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut random = [0; 8];
    getrandom::getrandom(&mut random).map_err(|e| e.to_string())?;
    let temp = path.with_extension(format!("{}.tmp", hex::encode(random)));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut f = options.open(&temp).map_err(|e| e.to_string())?;
        f.write_all(bytes).map_err(|e| e.to_string())?;
        f.sync_all().map_err(|e| e.to_string())?;
        std::fs::rename(&temp, path).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        std::fs::File::open(path.parent().ok_or("missing parent")?)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temp);
    }
    result
}

/// Keep this handle alive for the entire daemon lifetime.
pub fn instance_lock(dir: &Path) -> Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(dir.join("agent.lock"))
        .map_err(|e| e.to_string())?;
    fs2::FileExt::try_lock_exclusive(&file)
        .map_err(|_| format!("{} agent already running", shuttli_brand::NAME))?;
    Ok(file)
}
