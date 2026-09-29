//! Desktop persistence for the shared, portable device TLS identity.
use crate::files::atomic_write;
use std::path::Path;

pub use shuttli_identity::{Identity, device_id};

pub fn load(dir: &Path) -> Result<Identity, String> {
    let path = dir.join("identity.json");
    match std::fs::read(&path) {
        Ok(bytes) => Identity::from_bytes(&bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let (identity, bytes) = Identity::generate()?;
            atomic_write(&path, &bytes)?;
            Ok(identity)
        }
        Err(e) => Err(e.to_string()),
    }
}
