//! Production OS, storage and transport adapters.
pub mod content;

pub mod clipboard;

pub mod files;

pub mod storage;

pub mod identity;

pub mod discovery;

pub mod network;

pub mod platform;

pub fn random_epoch() -> shuttli_ports::sync::Result<[u8; 16]> {
    let mut epoch = [0; 16];
    getrandom::getrandom(&mut epoch).map_err(|e| e.to_string())?;
    Ok(epoch)
}

mod recent;
