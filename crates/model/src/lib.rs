//! Platform-neutral facts. These values are not authorization tokens.
#![no_std]

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct OperationId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ObjectId(pub u64);

/// A token is opaque: compare equality only, and include the backend instance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Revision {
    pub backend_instance: u64,
    pub token: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnerEvidence {
    External,
    Internal(OperationId),
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentKind {
    Text,
    Png,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Observation {
    pub revision: Revision,
    pub owner: OwnerEvidence,
    pub kind: Option<ContentKind>,
    pub concealed: bool,
    pub transient: bool,
}

/// Reference to an immutable, bounded object, not clipboard bytes or an OS path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub revision: Revision,
    pub object: ObjectId,
    pub kind: ContentKind,
    pub byte_len: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureLimits {
    pub max_bytes: u64,
    pub deadline_tick: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadbackPath {
    SystemData,
    OwnerServed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Verification {
    pub revision: Revision,
    pub path: ReadbackPath,
    pub representation: ContentKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClipboardCapabilities {
    pub read_text: bool,
    pub read_png: bool,
    pub write_text: bool,
    pub write_png: bool,
    pub observe_revision: bool,
    pub verify: bool,
    pub available: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortError {
    Unavailable,
    PermissionDenied,
    Busy,
    Stale,
    UnsupportedType,
    InvalidContent,
    TooLarge,
    WriteFailed,
    VerificationFailed,
    Cancelled,
    TimedOut,
    StorageFailed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutostartObserved {
    Enabled,
    Disabled,
    RequiresUserAction,
    Unavailable,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UseReference {
    Selection,
    Transfer,
    History,
}

pub mod sync;
