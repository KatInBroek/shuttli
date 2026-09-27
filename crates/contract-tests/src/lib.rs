//! P0 contract harness. Synthetic controls are test fixtures, never production ports.
//! Real OS drivers can implement these controls to run the same assertions.
//! Passing a simulated backend does not establish native clipboard support.

use shuttli_model::{
    CaptureLimits, ContentKind, ObjectId, Observation, OperationId, OwnerEvidence, PortError,
    ReadbackPath, Revision, Snapshot, UseReference, Verification,
};
use shuttli_ports::{ApplyRecord, ClipboardReadPort, JournalPort, ObjectLifetimePort};

pub trait ClipboardFixture: ClipboardReadPort {
    /// Publish a known synthetic value through an independent source/OS client.
    fn external_text(&mut self, value: &str);
    fn change_during_next_capture(&mut self, value: &str);
    fn restart_backend(&mut self);
    fn deny_permission(&mut self);
    /// Number of backend read requests made, not expected-buffer comparisons.
    fn backend_reads(&self) -> u64;
}

#[derive(Debug, PartialEq, Eq)]
pub struct Violation(pub &'static str);

fn check(condition: bool, message: &'static str) -> Result<(), Violation> {
    if condition {
        Ok(())
    } else {
        Err(Violation(message))
    }
}

fn capture(port: &mut impl ClipboardReadPort) -> Result<Snapshot, Violation> {
    let observed = port.observe().map_err(|_| Violation("observe failed"))?;
    port.capture(
        observed.revision,
        CaptureLimits {
            max_bytes: 64,
            deadline_tick: 10,
        },
    )
    .map_err(|_| Violation("capture failed"))
}

/// A retained snapshot must not prove the current value after an external change.
pub fn verify_reads_current_backend(port: &mut impl ClipboardFixture) -> Result<(), Violation> {
    port.external_text("original");
    let original = capture(port)?;
    let before = port.backend_reads();
    let result = port
        .verify(&original)
        .map_err(|_| Violation("valid readback failed"))?;
    check(
        result.revision == original.revision,
        "wrong verified revision",
    )?;
    check(
        port.backend_reads() > before,
        "verify did not request backend data",
    )?;
    port.external_text("replacement");
    let before = port.backend_reads();
    let result = port.verify(&original);
    check(
        result == Err(PortError::Stale) || result == Err(PortError::VerificationFailed),
        "verify accepted stale snapshot instead of the current backend",
    )?;
    check(
        port.backend_reads() > before,
        "stale verify did not inspect backend",
    )
}

pub fn capture_rejects_races(port: &mut impl ClipboardFixture) -> Result<(), Violation> {
    port.external_text("before");
    let revision = port
        .observe()
        .map_err(|_| Violation("observe failed"))?
        .revision;
    port.change_during_next_capture("after");
    check(
        port.capture(
            revision,
            CaptureLimits {
                max_bytes: 64,
                deadline_tick: 10,
            },
        ) == Err(PortError::Stale),
        "capture accepted a value changed during reading",
    )
}

pub fn epoch_and_permission_are_not_hidden(
    port: &mut impl ClipboardFixture,
) -> Result<(), Violation> {
    port.external_text("same");
    let first = capture(port)?;
    port.external_text("same");
    let repeated = capture(port)?;
    check(
        first.revision != repeated.revision,
        "new native generation of same content was lost",
    )?;
    port.restart_backend();
    let restarted = capture(port)?;
    check(
        restarted.revision.backend_instance != repeated.revision.backend_instance,
        "backend restart reused its instance identity",
    )?;
    check(
        port.capture(
            repeated.revision,
            CaptureLimits {
                max_bytes: 64,
                deadline_tick: 10,
            },
        ) == Err(PortError::Stale),
        "previous backend revision accepted after restart",
    )?;
    port.deny_permission();
    check(
        !port.capabilities().available,
        "capability still available after permission loss",
    )?;
    check(
        port.observe() == Err(PortError::PermissionDenied),
        "permission loss was hidden",
    )
}

pub fn bounded_capture(port: &mut impl ClipboardFixture) -> Result<(), Violation> {
    port.external_text("123456789");
    let revision = port
        .observe()
        .map_err(|_| Violation("observe failed"))?
        .revision;
    check(
        port.capture(
            revision,
            CaptureLimits {
                max_bytes: 8,
                deadline_tick: 10,
            },
        ) == Err(PortError::TooLarge),
        "capture exceeded byte limit",
    )?;
    check(
        port.capture(
            revision,
            CaptureLimits {
                max_bytes: 64,
                deadline_tick: 0,
            },
        ) == Err(PortError::TimedOut),
        "capture ignored expired deadline",
    )
}

/// This contract applies to commit semantics, independently of SQLite/schema.
pub fn journal_requires_committed_intent(port: &mut impl JournalPort) -> Result<(), Violation> {
    let id = OperationId(123);
    let verified = Verification {
        revision: Revision {
            backend_instance: 1,
            token: Some(2),
        },
        path: ReadbackPath::SystemData,
        representation: ContentKind::Text,
    };
    check(
        port.record_applied(id, verified) == Err(PortError::StorageFailed),
        "applied was accepted before a durable intent",
    )?;
    port.record_intent(id)
        .map_err(|_| Violation("intent failed"))?;
    check(
        port.lookup(id) == Ok(Some(ApplyRecord::Intent)),
        "committed intent is missing",
    )?;
    port.record_applied(id, verified)
        .map_err(|_| Violation("applied failed"))?;
    // Repeating intent must not turn an applied event back into a pending write.
    port.record_intent(id)
        .map_err(|_| Violation("duplicate intent failed"))?;
    check(
        port.lookup(id) == Ok(Some(ApplyRecord::Applied(verified))),
        "duplicate intent regressed applied",
    )
}

pub fn active_selection_survives_history_deletion(
    port: &mut impl ObjectLifetimePort,
    object: ObjectId,
) -> Result<(), Violation> {
    port.retain(object, UseReference::Selection)
        .map_err(|_| Violation("selection retain failed"))?;
    port.retain(object, UseReference::History)
        .map_err(|_| Violation("history retain failed"))?;
    port.release(object, UseReference::History)
        .map_err(|_| Violation("history release failed"))?;
    check(
        !port
            .collect()
            .map_err(|_| Violation("collect failed"))?
            .contains(&object),
        "history deletion removed current selection",
    )?;
    port.release(object, UseReference::Selection)
        .map_err(|_| Violation("selection release failed"))?;
    check(
        port.collect()
            .map_err(|_| Violation("collect failed"))?
            .contains(&object),
        "unreferenced object was not collected",
    )
}

/// Keep platform fact types visible to drivers without introducing OS types.
pub fn external_observation(revision: Revision) -> Observation {
    Observation {
        revision,
        owner: OwnerEvidence::External,
        kind: Some(ContentKind::Text),
        concealed: false,
        transient: false,
    }
}
