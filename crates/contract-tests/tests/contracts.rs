use shuttli_adapters::{content::payload, storage::SqlStore};
use shuttli_model::sync::*;
use shuttli_ports::sync::Store;
#[test]
fn real_sqlite_adapter_satisfies_durability_replay_and_history_contract() {
    let random = shuttli_adapters::random_epoch().unwrap();
    let dir = std::env::temp_dir().join(format!("shuttli-contract-{random:?}"));
    std::fs::create_dir_all(&dir).unwrap();
    let event = {
        let mut store = SqlStore::open(&dir).unwrap();
        let p = payload(Format::Text, b"synthetic contract content".to_vec()).unwrap();
        shuttli_contract_tests::store_contract(&mut store, &p).unwrap()
    };
    let mut recovered = SqlStore::open(&dir).unwrap();
    assert_eq!(
        recovered.receipt(event).unwrap(),
        Some(DeliveryState::Applied)
    );
    assert!(!recovered.reserve(event).unwrap());
    assert!(recovered.history(0, 10).unwrap().is_empty());
    drop(recovered);
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn invalid_content_is_rejected_by_production_capture_validation() {
    assert!(payload(Format::Text, vec![0xff]).is_err());
    assert!(payload(Format::Text, vec![b'x'; 1024 * 1024 + 1]).is_err());
    assert!(payload(Format::Png, b"invalid image".to_vec()).is_err());
}
