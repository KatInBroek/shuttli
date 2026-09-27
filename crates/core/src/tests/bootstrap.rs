use super::*;

#[test]
fn bootstrap_replays_without_creating_a_publication() {
    let input = StartupInput {
        epoch: 42,
        monotonic_tick: 100,
    };
    assert_eq!(Core::start(input), Core::start(input));
    assert_eq!(
        Core::start(input).1,
        Effect::EstablishBaseline {
            operation: OperationId(0)
        }
    );
}
