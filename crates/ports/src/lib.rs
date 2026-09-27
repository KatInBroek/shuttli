//! Execution ports. Adapters report facts; application/core own policy.
//!
//! This P0 subset intentionally has no production clipboard write/network port.
//! Those ports require C01/C02 task permits before they can be implemented.

use shuttli_model::{
    AutostartObserved, CaptureLimits, ClipboardCapabilities, ObjectId, Observation, OperationId,
    PortError, Revision, Snapshot, UseReference, Verification,
};

pub trait ClipboardReadPort {
    fn capabilities(&self) -> ClipboardCapabilities;
    fn observe(&mut self) -> Result<Observation, PortError>;
    /// Read both before and after capture; reject a changed revision.
    fn capture(&mut self, expected: Revision, limits: CaptureLimits)
    -> Result<Snapshot, PortError>;
    /// Request the current value through the backend, not the supplied snapshot.
    fn verify(&mut self, expected: &Snapshot) -> Result<Verification, PortError>;
}

pub trait ObjectLifetimePort {
    fn retain(&mut self, object: ObjectId, usage: UseReference) -> Result<(), PortError>;
    fn release(&mut self, object: ObjectId, usage: UseReference) -> Result<(), PortError>;
    /// Remove only objects with no remaining references.
    fn collect(&mut self) -> Result<Vec<ObjectId>, PortError>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplyRecord {
    Intent,
    Applied(Verification),
}

pub trait JournalPort {
    /// Return success only after the implementation's durability boundary.
    fn record_intent(&mut self, operation: OperationId) -> Result<(), PortError>;
    /// Must reject an applied record without a previously committed intent.
    fn record_applied(
        &mut self,
        operation: OperationId,
        result: Verification,
    ) -> Result<(), PortError>;
    fn lookup(&self, operation: OperationId) -> Result<Option<ApplyRecord>, PortError>;
}

pub trait AutostartPort {
    fn query(&mut self) -> Result<AutostartObserved, PortError>;
    /// Submitting a setting is not proof of its effective OS status.
    fn set_enabled(&mut self, enabled: bool) -> Result<(), PortError>;
}

pub mod sync;
