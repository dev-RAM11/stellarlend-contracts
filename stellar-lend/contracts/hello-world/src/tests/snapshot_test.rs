#![cfg(test)]
use crate::tests::test_helpers::{setup_test_env, create_token};
use crate::analytics::{ProtocolReport, UserReport};

#[test]
fn test_analytics_snapshots_deterministic() {
    let (env, client, admin) = setup_test_env();
    let user = env.ledger().with_mut(|li| li.sequence = 100); // Stable timestamp/sequence
    let user_addr = env.accounts().generate();
    let token = create_token(&env, &admin);

    // 1. Initial State Snapshot (Zero values)
    let report = client.get_protocol_report();
    assert_eq!(report.metrics.total_value_locked, 0, "TVL should start at 0");
    assert_eq!(report.metrics.utilization_rate, 0, "Utilization should start at 0");

    // 2. Perform actions to generate data
    // (Simulating deposit of 1000 units at 10^7 scale)
    client.deposit(&user_addr, &token.address, &1000_0000000);

    // 3. Verify Protocol Report Invariants
    let proto_report = client.get_protocol_report();
    assert!(proto_report.metrics.total_value_locked > 0);
    assert!(proto_report.timestamp > 0);

    // 4. Verify User Report Invariants
    let user_report = client.get_user_report(&user_addr);
    assert_eq!(user_report.user, user_addr);
    assert!(user_report.health_factor >= 10000); // 1.0 in 10^4 scale
}
