mod common;

use cashu_credit::{
    AcceptanceMode, ClosedLoopConsumption, CreditAccount, CreditAccountSnapshotV1, CreditError,
    CreditNovation, ReceiptApplication, SnapshotError, ValueClass,
};
use common::*;
use serde_json::Value;

fn corrupt(
    account: &CreditAccount,
    mutate: impl FnOnce(&mut Value),
) -> Result<CreditAccountSnapshotV1, SnapshotError> {
    let mut value = serde_json::to_value(account.snapshot()).unwrap();
    mutate(&mut value);
    CreditAccountSnapshotV1::decode_json(&serde_json::to_string(&value).unwrap())
}

fn assert_invalid(result: Result<CreditAccountSnapshotV1, SnapshotError>) {
    assert!(matches!(
        result,
        Err(SnapshotError::Credit(CreditError::InvalidSnapshot(_)))
            | Err(SnapshotError::Credit(
                CreditError::UnsupportedSnapshotVersion
            ))
    ));
}

#[test]
fn revision_advances_once_for_each_applied_mutation_and_never_for_replay() {
    let mut account = new_account();
    assert_eq!(account.revision(), 0);

    let carol_backing = deposit("carol-backing", "carol", 20, ValueClass::ClosedLoopDeposit);
    account
        .record_backing_deposit(&carol_backing, "carol")
        .unwrap();
    assert_eq!(account.revision(), 1);
    account
        .record_backing_deposit(&carol_backing, "carol")
        .unwrap();
    assert_eq!(account.revision(), 1);

    let debt = claim("debt", 40, ValueClass::PeerCredit);
    account
        .apply_receipt(&debt, "alice", AcceptanceMode::Online, NOW)
        .unwrap();
    assert_eq!(account.revision(), 2);
    account
        .apply_receipt(&debt, "alice", AcceptanceMode::Online, NOW)
        .unwrap();
    assert_eq!(account.revision(), 2);

    let novation = CreditNovation {
        novation_id: "novation".into(),
        counterparty: "bob".into(),
        from_issuer: "alice".into(),
        to_issuer: "carol".into(),
        amount_sat: 5,
        expires_at_unix: NOW + 10,
    };
    account.novate_peer_credit(&novation, "carol", NOW).unwrap();
    assert_eq!(account.revision(), 3);
    account.novate_peer_credit(&novation, "carol", NOW).unwrap();
    assert_eq!(account.revision(), 3);

    let backed = backed_settlement(
        "backed",
        "alice",
        "carol",
        10,
        ValueClass::ClosedLoopDeposit,
    );
    account
        .settle_peer_credit_with_backing(&backed, "carol", NOW)
        .unwrap();
    assert_eq!(account.revision(), 4);
    account
        .settle_peer_credit_with_backing(&backed, "carol", NOW)
        .unwrap();
    assert_eq!(account.revision(), 4);

    let alice_closed = deposit("alice-closed", "alice", 10, ValueClass::ClosedLoopDeposit);
    account
        .record_backing_deposit(&alice_closed, "alice")
        .unwrap();
    let closed_receipt = claim("closed", 10, ValueClass::ClosedLoopDeposit);
    account
        .apply_receipt(&closed_receipt, "alice", AcceptanceMode::Online, NOW)
        .unwrap();
    assert_eq!(account.revision(), 6);
    let consumption = ClosedLoopConsumption {
        consumption_id: "consume".into(),
        issuer: "alice".into(),
        counterparty: "bob".into(),
        resource: "fsp:budget".into(),
        amount_sat: 4,
    };
    account.consume_closed_loop(&consumption, "bob").unwrap();
    assert_eq!(account.revision(), 7);
    account.consume_closed_loop(&consumption, "bob").unwrap();
    assert_eq!(account.revision(), 7);

    let reserve = deposit(
        "reserve",
        "alice",
        40,
        ValueClass::ReserveBackedWithdrawable,
    );
    account.record_backing_deposit(&reserve, "alice").unwrap();
    account
        .apply_receipt(
            &claim("withdrawable", 30, ValueClass::ReserveBackedWithdrawable),
            "alice",
            AcceptanceMode::Online,
            NOW,
        )
        .unwrap();
    assert_eq!(account.revision(), 9);
    let mut payout = settlement("payout", 20);
    payout.max_fee_sat = 1;
    account
        .authorize_external_settlement(&payout, "bob", NOW)
        .unwrap();
    assert_eq!(account.revision(), 10);
    account
        .authorize_external_settlement(&payout, "bob", NOW)
        .unwrap();
    assert_eq!(account.revision(), 10);
    account.complete_external_settlement("payout", 1).unwrap();
    assert_eq!(account.revision(), 11);
    account.complete_external_settlement("payout", 1).unwrap();
    assert_eq!(account.revision(), 11);

    let cancelled = settlement("cancelled", 5);
    account
        .authorize_external_settlement(&cancelled, "bob", NOW)
        .unwrap();
    account.cancel_external_settlement("cancelled").unwrap();
    assert_eq!(account.revision(), 13);
    account.cancel_external_settlement("cancelled").unwrap();
    assert_eq!(account.revision(), 13);
}

#[test]
fn validated_json_restart_preserves_idempotency_and_resource_binding() {
    let mut account = new_account();
    let receipt = claim("receipt", 10, ValueClass::PeerCredit);
    account
        .apply_receipt(&receipt, "alice", AcceptanceMode::Online, NOW)
        .unwrap();
    let encoded = account.snapshot().encode_json().unwrap();
    assert!(encoded.contains("\"version\":1"));
    assert!(encoded.contains("subscription:alice:kind-1"));

    let snapshot = CreditAccountSnapshotV1::decode_json(&encoded).unwrap();
    let mut restored = CreditAccount::from_snapshot(snapshot).unwrap();
    assert_eq!(restored, account);
    let revision = restored.revision();
    assert_eq!(
        restored
            .apply_receipt(&receipt, "alice", AcceptanceMode::Online, NOW)
            .unwrap(),
        ReceiptApplication::AlreadyApplied
    );
    assert_eq!(restored.revision(), revision);

    let mut rebound = receipt;
    rebound.resource = "fmp:generic-forwarded-bytes".into();
    assert_eq!(
        restored.apply_receipt(&rebound, "alice", AcceptanceMode::Online, NOW),
        Err(CreditError::ReceiptConflict)
    );
    assert_eq!(restored.revision(), revision);
}

#[test]
fn snapshot_orders_backing_by_issuer_and_id_and_rejects_duplicate_tuples() {
    let mut account = new_account();
    for issuer in ["carol", "alice"] {
        account
            .record_backing_deposit(
                &deposit(
                    "same-local-id",
                    issuer,
                    5,
                    ValueClass::ReserveBackedWithdrawable,
                ),
                issuer,
            )
            .unwrap();
    }

    let encoded = account.snapshot().encode_json().unwrap();
    assert_eq!(encoded, account.snapshot().encode_json().unwrap());
    let mut json: Value = serde_json::from_str(&encoded).unwrap();
    let deposits = json["backing_deposits"].as_array_mut().unwrap();
    assert_eq!(deposits.len(), 2);
    assert_eq!(deposits[0]["issuer"], "alice");
    assert_eq!(deposits[1]["issuer"], "carol");

    let restored =
        CreditAccount::from_snapshot(CreditAccountSnapshotV1::decode_json(&encoded).unwrap())
            .unwrap();
    assert_eq!(restored, account);

    deposits.push(deposits[0].clone());
    assert_invalid(CreditAccountSnapshotV1::decode_json(
        &serde_json::to_string(&json).unwrap(),
    ));

    let mut unordered: Value = serde_json::from_str(&encoded).unwrap();
    unordered["backing_deposits"]
        .as_array_mut()
        .unwrap()
        .swap(0, 1);
    assert_invalid(CreditAccountSnapshotV1::decode_json(
        &serde_json::to_string(&unordered).unwrap(),
    ));
}

#[test]
fn snapshot_rejects_unknown_fields_inside_protocol_records() {
    let mut account = new_account();
    account
        .record_backing_deposit(
            &deposit("backing", "alice", 5, ValueClass::ReserveBackedWithdrawable),
            "alice",
        )
        .unwrap();
    account
        .apply_receipt(
            &claim("receipt", 5, ValueClass::ReserveBackedWithdrawable),
            "alice",
            AcceptanceMode::Online,
            NOW,
        )
        .unwrap();

    for mutate in [
        |json: &mut Value| json["issuers"]["alice"]["policy"]["unknown_cap"] = 1.into(),
        |json: &mut Value| json["applied_receipts"]["receipt"]["unknown_proof"] = true.into(),
        |json: &mut Value| json["backing_deposits"][0]["unknown_binding"] = "x".into(),
    ] {
        let mut json = serde_json::to_value(account.snapshot()).unwrap();
        mutate(&mut json);
        assert!(matches!(
            CreditAccountSnapshotV1::decode_json(&serde_json::to_string(&json).unwrap()),
            Err(SnapshotError::Json(_))
        ));
    }
}

#[test]
fn restore_rejects_version_peer_total_and_operation_key_corruption() {
    let mut account = new_account();
    account
        .apply_receipt(
            &claim("receipt", 10, ValueClass::PeerCredit),
            "alice",
            AcceptanceMode::Online,
            NOW,
        )
        .unwrap();

    assert_invalid(corrupt(&account, |json| json["version"] = 2.into()));
    assert_invalid(corrupt(&account, |json| {
        json["total_peer_credit_sat"] = 9.into();
    }));
    assert_invalid(corrupt(&account, |json| json["revision"] = 2.into()));
    assert_invalid(corrupt(&account, |json| {
        let receipt = json["applied_receipts"]
            .as_object_mut()
            .unwrap()
            .remove("receipt")
            .unwrap();
        json["applied_receipts"]["wrong-key"] = receipt;
    }));
}

#[test]
fn restore_reconstructs_offline_exposure_from_retained_receipts() {
    let mut account = new_account();
    account
        .apply_receipt(
            &claim("offline", 10, ValueClass::PeerCredit),
            "alice",
            AcceptanceMode::OfflineDeferred,
            NOW,
        )
        .unwrap();

    let encoded = account.snapshot().encode_json().unwrap();
    let restored =
        CreditAccount::from_snapshot(CreditAccountSnapshotV1::decode_json(&encoded).unwrap())
            .unwrap();
    assert_eq!(
        restored
            .peer_credit("alice")
            .unwrap()
            .offline_outstanding_sat(),
        10
    );

    assert_invalid(corrupt(&account, |json| {
        json["peer_credit_events"][0]["mode"] = "online".into();
    }));
    assert_invalid(corrupt(&account, |json| {
        json["peer_credit_events"] = serde_json::json!([]);
    }));
}

#[test]
fn restore_replays_interleaved_offline_credit_and_novation_in_revision_order() {
    let mut account = new_account();
    account
        .apply_receipt(
            &claim("offline", 10, ValueClass::PeerCredit),
            "alice",
            AcceptanceMode::OfflineDeferred,
            NOW,
        )
        .unwrap();
    account
        .novate_peer_credit(
            &CreditNovation {
                novation_id: "move-offline".into(),
                counterparty: "bob".into(),
                from_issuer: "alice".into(),
                to_issuer: "carol".into(),
                amount_sat: 10,
                expires_at_unix: NOW + 10,
            },
            "carol",
            NOW,
        )
        .unwrap();
    account
        .apply_receipt(
            &claim("online-after", 10, ValueClass::PeerCredit),
            "alice",
            AcceptanceMode::Online,
            NOW,
        )
        .unwrap();

    let encoded = account.snapshot().encode_json().unwrap();
    let restored =
        CreditAccount::from_snapshot(CreditAccountSnapshotV1::decode_json(&encoded).unwrap())
            .unwrap();
    assert_eq!(restored, account);
    assert_eq!(
        restored
            .peer_credit("alice")
            .unwrap()
            .offline_outstanding_sat(),
        0
    );
}

#[test]
fn restore_rejects_backing_allocation_and_consumption_corruption() {
    let mut account = new_account();
    account
        .record_backing_deposit(
            &deposit("closed", "alice", 10, ValueClass::ClosedLoopDeposit),
            "alice",
        )
        .unwrap();
    account
        .apply_receipt(
            &claim("closed-receipt", 8, ValueClass::ClosedLoopDeposit),
            "alice",
            AcceptanceMode::Online,
            NOW,
        )
        .unwrap();
    account
        .consume_closed_loop(
            &ClosedLoopConsumption {
                consumption_id: "consume".into(),
                issuer: "alice".into(),
                counterparty: "bob".into(),
                resource: "fsp:budget".into(),
                amount_sat: 3,
            },
            "bob",
        )
        .unwrap();

    assert_invalid(corrupt(&account, |json| {
        json["issuers"]["alice"]["closed_loop"]["total_deposited_sat"] = 11.into();
        json["issuers"]["alice"]["closed_loop"]["available_backing_sat"] = 3.into();
    }));
    assert_invalid(corrupt(&account, |json| {
        json["issuers"]["alice"]["closed_loop"]["available_backing_sat"] = 1.into();
        json["issuers"]["alice"]["closed_loop"]["claimable_sat"] = 6.into();
    }));
    assert_invalid(corrupt(&account, |json| {
        json["closed_loop_consumptions"]["consume"]["amount_sat"] = 2.into();
    }));

    let mut backed = new_account();
    backed
        .record_backing_deposit(
            &deposit("carol", "carol", 10, ValueClass::ClosedLoopDeposit),
            "carol",
        )
        .unwrap();
    backed
        .apply_receipt(
            &claim("peer-credit", 10, ValueClass::PeerCredit),
            "alice",
            AcceptanceMode::Online,
            NOW,
        )
        .unwrap();
    backed
        .settle_peer_credit_with_backing(
            &backed_settlement(
                "backed",
                "alice",
                "carol",
                10,
                ValueClass::ClosedLoopDeposit,
            ),
            "carol",
            NOW,
        )
        .unwrap();
    assert_invalid(corrupt(&backed, |json| {
        json["backed_settlements"]["backed"]["amount_sat"] = 9.into();
    }));
}

#[test]
fn restore_rejects_external_reservation_pending_and_completed_corruption() {
    let mut pending = new_account();
    pending
        .record_backing_deposit(
            &deposit(
                "reserve",
                "alice",
                50,
                ValueClass::ReserveBackedWithdrawable,
            ),
            "alice",
        )
        .unwrap();
    pending
        .apply_receipt(
            &claim("reserve-receipt", 50, ValueClass::ReserveBackedWithdrawable),
            "alice",
            AcceptanceMode::Online,
            NOW,
        )
        .unwrap();
    let mut request = settlement("payout", 20);
    request.max_fee_sat = 2;
    pending
        .authorize_external_settlement(&request, "bob", NOW)
        .unwrap();

    assert_invalid(corrupt(&pending, |json| {
        json["settlements"]["payout"]["authorization"]["reserved_sat"] = 21.into();
    }));
    assert_invalid(corrupt(&pending, |json| {
        json["issuers"]["alice"]["withdrawable"]["pending_external_sat"] = 21.into();
        json["issuers"]["alice"]["withdrawable"]["redeemable_sat"] = 29.into();
    }));

    pending.complete_external_settlement("payout", 1).unwrap();
    assert_invalid(corrupt(&pending, |json| {
        json["settlements"]["payout"]["state"]["fee_sat"] = 2.into();
    }));
}

#[test]
fn restored_account_enumerates_only_pending_authorizations_in_stable_order() {
    let mut account = new_account();
    account
        .record_backing_deposit(
            &deposit(
                "recovery-reserve",
                "alice",
                100,
                ValueClass::ReserveBackedWithdrawable,
            ),
            "alice",
        )
        .unwrap();
    account
        .apply_receipt(
            &claim(
                "recovery-receipt",
                100,
                ValueClass::ReserveBackedWithdrawable,
            ),
            "alice",
            AcceptanceMode::Online,
            NOW,
        )
        .unwrap();

    for id in ["zeta", "completed", "alpha", "cancelled"] {
        account
            .authorize_external_settlement(&settlement(id, 10), "bob", NOW)
            .unwrap();
    }
    account
        .complete_external_settlement("completed", 0)
        .unwrap();
    account.cancel_external_settlement("cancelled").unwrap();

    let encoded = account.snapshot().encode_json().unwrap();
    let restored =
        CreditAccount::from_snapshot(CreditAccountSnapshotV1::decode_json(&encoded).unwrap())
            .unwrap();
    let pending = restored.pending_external_settlement_authorizations();
    let pending_ids = pending
        .iter()
        .map(|authorization| authorization.settlement_id.as_str())
        .collect::<Vec<_>>();

    assert_eq!(pending_ids, ["alpha", "zeta"]);
}
