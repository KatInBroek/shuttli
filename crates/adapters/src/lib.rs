//! Safe unavailable backend for the foundation binary. No OS setting is changed.

use shuttli_model::{AutostartObserved, PortError};
use shuttli_ports::AutostartPort;

pub struct UnavailableAutostart;

impl AutostartPort for UnavailableAutostart {
    fn query(&mut self) -> Result<AutostartObserved, PortError> {
        Ok(AutostartObserved::Unavailable)
    }
    fn set_enabled(&mut self, _: bool) -> Result<(), PortError> {
        Err(PortError::Unavailable)
    }
}

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
