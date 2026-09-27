//! Reusable assertions against the production port contracts.
use shuttli_model::sync::*;
use shuttli_ports::sync::{Payload, Result, Store};
/// Verify durable apply intent, replay protection and independent object lifetime.
/// The caller supplies a fresh store and validated synthetic content.
pub fn store_contract(store: &mut dyn Store, payload: &Payload) -> Result<EventId> {
    let event = EventId {
        origin: [7; 32],
        epoch: [3; 16],
        seq: 1,
    };
    if store
        .record(
            event,
            "fixture",
            "receive",
            DeliveryState::Applied,
            payload,
            "fixture",
        )
        .is_ok()
    {
        return Err("applied receipt accepted without intent".into());
    }
    assert!(store.reserve(event)?);
    assert!(!store.reserve(event)?);
    assert!(
        store
            .record(
                event,
                "fixture",
                "receive",
                DeliveryState::Applied,
                payload,
                "fixture"
            )
            .is_err()
    );
    store.intent(event)?;
    let row = store.record(
        event,
        "fixture",
        "receive",
        DeliveryState::Applied,
        payload,
        "fixture",
    )?;
    assert_eq!(store.receipt(event)?, Some(DeliveryState::Applied));
    let retained = store.content(row)?;
    store.clear()?;
    assert!(store.content(row).is_err());
    assert_eq!(
        retained.data, payload.data,
        "active clipboard/transfer content survives history removal"
    );
    assert_eq!(store.receipt(event)?, Some(DeliveryState::Applied));
    assert!(!store.reserve(event)?);
    Ok(event)
}
