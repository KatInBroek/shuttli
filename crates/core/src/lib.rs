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

pub use sync::Publication as LocalPublication;
pub mod sync;
