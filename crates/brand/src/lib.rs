//! Presentation identity only. Never use this name for storage, trust, or wire IDs.
include!(concat!(env!("OUT_DIR"), "/brand.rs"));

/// Escape a display name for a freedesktop desktop-entry string value.
pub fn desktop_name() -> String {
    NAME.replace('\\', "\\\\")
}
