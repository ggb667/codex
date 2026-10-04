use super::idle_drain_error_message;

#[test]
fn idle_drain_failure_message_snapshot() {
    let err = std::io::Error::other("receipt ledger unavailable");

    insta::assert_snapshot!(idle_drain_error_message(&err), @"Idle deferred because inbound agent messages could not be checked: receipt ledger unavailable");
}
