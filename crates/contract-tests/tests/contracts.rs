use shuttli_contract_tests::*;
use shuttli_model::*;
use shuttli_ports::*;
use std::collections::BTreeMap;

#[derive(Default)]
struct Clipboard {
    value: String,
    instance: u64,
    token: u64,
    reads: u64,
    denied: bool,
    race: Option<String>,
    snapshots: BTreeMap<ObjectId, String>,
    faulty_verify: bool,
}

impl Clipboard {
    fn revision(&self) -> Revision {
        Revision {
            backend_instance: self.instance,
            token: Some(self.token),
        }
    }
}

impl ClipboardReadPort for Clipboard {
    fn capabilities(&self) -> ClipboardCapabilities {
        ClipboardCapabilities {
            read_text: !self.denied,
            read_png: false,
            write_text: false,
            write_png: false,
            observe_revision: true,
            verify: !self.denied,
            available: !self.denied,
        }
    }
    fn observe(&mut self) -> Result<Observation, PortError> {
        if self.denied {
            return Err(PortError::PermissionDenied);
        }
        Ok(external_observation(self.revision()))
    }
    fn capture(
        &mut self,
        expected: Revision,
        limits: CaptureLimits,
    ) -> Result<Snapshot, PortError> {
        self.observe()?;
        if expected != self.revision() {
            return Err(PortError::Stale);
        }
        if limits.deadline_tick == 0 {
            return Err(PortError::TimedOut);
        }
        if self.value.len() as u64 > limits.max_bytes {
            return Err(PortError::TooLarge);
        }
        self.reads += 1;
        let value = self.value.clone();
        if let Some(next) = self.race.take() {
            self.external_text(&next);
        }
        if expected != self.revision() {
            return Err(PortError::Stale);
        }
        let object = ObjectId(self.snapshots.len() as u64);
        let byte_len = value.len() as u64;
        self.snapshots.insert(object, value);
        Ok(Snapshot {
            revision: expected,
            object,
            kind: ContentKind::Text,
            byte_len,
        })
    }
    fn verify(&mut self, expected: &Snapshot) -> Result<Verification, PortError> {
        if !self.faulty_verify {
            self.observe()?;
            self.reads += 1;
            if expected.revision != self.revision() {
                return Err(PortError::Stale);
            }
            if self.snapshots.get(&expected.object) != Some(&self.value) {
                return Err(PortError::VerificationFailed);
            }
        }
        Ok(Verification {
            revision: expected.revision,
            path: ReadbackPath::SystemData,
            representation: ContentKind::Text,
        })
    }
}

impl ClipboardFixture for Clipboard {
    fn external_text(&mut self, value: &str) {
        self.token = self.token.wrapping_add(1);
        self.value = value.into();
    }
    fn change_during_next_capture(&mut self, value: &str) {
        self.race = Some(value.into());
    }
    fn restart_backend(&mut self) {
        self.instance += 1;
        self.token = 0;
    }
    fn deny_permission(&mut self) {
        self.denied = true;
    }
    fn backend_reads(&self) -> u64 {
        self.reads
    }
}

#[test]
fn good_reader_passes_current_value_and_race_contracts() {
    verify_reads_current_backend(&mut Clipboard::default()).unwrap();
    capture_rejects_races(&mut Clipboard::default()).unwrap();
    epoch_and_permission_are_not_hidden(&mut Clipboard::default()).unwrap();
    bounded_capture(&mut Clipboard::default()).unwrap();
}

#[test]
fn dishonest_readback_backend_is_detected() {
    let mut port = Clipboard {
        faulty_verify: true,
        ..Clipboard::default()
    };
    assert_eq!(
        verify_reads_current_backend(&mut port),
        Err(Violation("verify did not request backend data"))
    );
}

#[test]
fn native_token_wrap_does_not_imply_ordering() {
    let mut port = Clipboard {
        token: u64::MAX,
        ..Clipboard::default()
    };
    let old = port.revision();
    port.external_text("after-wrap");
    assert_eq!(port.revision().token, Some(0));
    assert_eq!(
        port.capture(
            old,
            CaptureLimits {
                max_bytes: 64,
                deadline_tick: 10
            }
        ),
        Err(PortError::Stale)
    );
}

#[derive(Clone, Default)]
struct Journal {
    committed: BTreeMap<OperationId, ApplyRecord>,
    fail_commit: bool,
    allow_missing_intent: bool,
}

impl JournalPort for Journal {
    fn record_intent(&mut self, id: OperationId) -> Result<(), PortError> {
        if self.fail_commit {
            return Err(PortError::StorageFailed);
        }
        self.committed.entry(id).or_insert(ApplyRecord::Intent);
        Ok(())
    }
    fn record_applied(&mut self, id: OperationId, result: Verification) -> Result<(), PortError> {
        if self.fail_commit || (!self.allow_missing_intent && !self.committed.contains_key(&id)) {
            return Err(PortError::StorageFailed);
        }
        self.committed.insert(id, ApplyRecord::Applied(result));
        Ok(())
    }
    fn lookup(&self, id: OperationId) -> Result<Option<ApplyRecord>, PortError> {
        Ok(self.committed.get(&id).copied())
    }
}

#[test]
fn durable_intent_contract_accepts_good_journal_and_rejects_broken_one() {
    journal_requires_committed_intent(&mut Journal::default()).unwrap();
    let mut broken = Journal {
        allow_missing_intent: true,
        ..Journal::default()
    };
    assert_eq!(
        journal_requires_committed_intent(&mut broken),
        Err(Violation("applied was accepted before a durable intent"))
    );
}

#[test]
fn simulated_crash_cannot_recover_a_failed_commit() {
    let mut journal = Journal::default();
    journal.record_intent(OperationId(1)).unwrap();
    journal.fail_commit = true;
    assert_eq!(
        journal.record_intent(OperationId(2)),
        Err(PortError::StorageFailed)
    );
    // Copy only durable state to a new simulated process. This is NOT a disk test.
    let recovered = Journal {
        committed: journal.committed.clone(),
        ..Journal::default()
    };
    assert_eq!(
        recovered.lookup(OperationId(1)),
        Ok(Some(ApplyRecord::Intent))
    );
    assert_eq!(recovered.lookup(OperationId(2)), Ok(None));
}

#[derive(Default)]
struct Objects(BTreeMap<ObjectId, [u32; 3]>);
fn slot(usage: UseReference) -> usize {
    match usage {
        UseReference::Selection => 0,
        UseReference::Transfer => 1,
        UseReference::History => 2,
    }
}
impl ObjectLifetimePort for Objects {
    fn retain(&mut self, id: ObjectId, usage: UseReference) -> Result<(), PortError> {
        let count = &mut self.0.entry(id).or_default()[slot(usage)];
        *count = count.checked_add(1).ok_or(PortError::TooLarge)?;
        Ok(())
    }
    fn release(&mut self, id: ObjectId, usage: UseReference) -> Result<(), PortError> {
        let count = &mut self.0.get_mut(&id).ok_or(PortError::InvalidContent)?[slot(usage)];
        *count = count.checked_sub(1).ok_or(PortError::InvalidContent)?;
        Ok(())
    }
    fn collect(&mut self) -> Result<Vec<ObjectId>, PortError> {
        let ids: Vec<_> = self
            .0
            .iter()
            .filter(|(_, counts)| **counts == [0; 3])
            .map(|(id, _)| *id)
            .collect();
        for id in &ids {
            self.0.remove(id);
        }
        Ok(ids)
    }
}

#[test]
fn active_selection_prevents_premature_collection() {
    active_selection_survives_history_deletion(&mut Objects::default(), ObjectId(1)).unwrap();
}

#[test]
fn unmatched_release_is_an_error_not_refcount_underflow() {
    let mut objects = Objects::default();
    objects.retain(ObjectId(1), UseReference::Transfer).unwrap();
    assert_eq!(
        objects.release(ObjectId(1), UseReference::History),
        Err(PortError::InvalidContent)
    );
    assert_eq!(objects.collect(), Ok(vec![]));
}
