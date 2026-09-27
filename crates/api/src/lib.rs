//! Versioned UI/CLI contract. No core, platform, database, or I/O dependencies.
#![no_std]

pub const API_VERSION: u16 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Status,
    GetAutostartStatus,
    SetAutostart { enabled: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    pub version: u16,
    pub command: Command,
}

impl Request {
    pub fn new(command: Command) -> Self {
        Self {
            version: API_VERSION,
            command,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObservedAutostart {
    Enabled,
    Disabled,
    RequiresUserAction,
    Unavailable,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApiError {
    UnsupportedVersion,
    Unavailable,
    PermissionDenied,
    Busy,
    OperationFailed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AutostartStatus {
    /// None until a setting has been loaded or explicitly requested.
    pub requested_enabled: Option<bool>,
    pub observed: ObservedAutostart,
    pub error: Option<ApiError>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Response {
    /// Foundation build: no clipboard sync backend is installed.
    Status {
        sync_available: bool,
    },
    Autostart(AutostartStatus),
}

pub trait ApplicationApi {
    fn dispatch(&mut self, request: Request) -> Result<Response, ApiError>;
}

pub mod control;
