//! Deterministic synchronization core. No OS, clock, or runtime I/O.
//!
//! Only validated local observations and explicit commands issue publications.
//! There is deliberately no public `new`, `Default`, or deserializer.
//!
//! ```compile_fail
//! use shuttli_core::LocalPublication;
//! let _ = LocalPublication { event_id: 1 };
//! ```
//!
//! ```compile_fail
//! let _: shuttli_core::LocalPublication = Default::default();
//! ```
#![no_std]

use shuttli_model::OperationId;

pub use sync::Publication as LocalPublication;

/// Host supplies clock and random epoch. Core never obtains these itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StartupInput {
    pub epoch: u64,
    pub monotonic_tick: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    EstablishBaseline { operation: OperationId },
}

#[derive(Debug, PartialEq, Eq)]
pub struct Core {
    startup: StartupInput,
}

impl Core {
    pub fn start(startup: StartupInput) -> (Self, Effect) {
        (
            Self { startup },
            Effect::EstablishBaseline {
                operation: OperationId(0),
            },
        )
    }
}

#[cfg(test)]
#[path = "tests/bootstrap.rs"]
mod tests;

pub mod sync;
