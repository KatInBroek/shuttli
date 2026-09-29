//! Small, versioned native boundary. Only UI-safe operations cross UniFFI;
//! authority tickets, TLS state and database handles remain in Rust.
use shuttli_mobile_sdk::MobileHistory;
use std::sync::Mutex;

uniffi::setup_scaffolding!();

#[uniffi::export]
pub fn sdk_api_version() -> u32 {
    1
}

#[derive(uniffi::Object)]
pub struct MobileSession {
    history: Mutex<MobileHistory>,
}

#[uniffi::export]
impl MobileSession {
    #[uniffi::constructor]
    pub fn new() -> Self {
        Self {
            history: Mutex::new(MobileHistory::default()),
        }
    }

    pub fn enter_foreground(&self) -> u64 {
        self.history
            .lock()
            .expect("mobile session lock")
            .enter_foreground()
    }

    pub fn enter_background(&self) {
        self.history
            .lock()
            .expect("mobile session lock")
            .enter_background();
    }

    pub fn history_count(&self) -> u32 {
        self.history
            .lock()
            .expect("mobile session lock")
            .timeline()
            .len() as u32
    }
}

impl Default for MobileSession {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_bridge_preserves_one_session_across_lifecycle() {
        let session = MobileSession::new();
        assert_eq!(sdk_api_version(), 1);
        let first = session.enter_foreground();
        session.enter_background();
        assert!(session.enter_foreground() > first);
        assert_eq!(session.history_count(), 0);
    }
}
