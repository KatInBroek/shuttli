//! OS adapter routing. No network or authorization decisions.
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "linux")]
mod x11;
use shuttli_ports::sync::{Clipboard, Result};
pub fn open() -> Result<Box<dyn Clipboard>> {
    #[cfg(target_os = "linux")]
    {
        Ok(Box::new(x11::XClipboard::open()?))
    }
    #[cfg(target_os = "macos")]
    {
        Ok(Box::new(macos::MacClipboard::open()?))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Err("clipboard backend is not implemented on this OS".into())
    }
}
