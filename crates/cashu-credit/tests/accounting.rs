mod common;

use cashu_credit::{
    AcceptanceMode, ClosedLoopConsumption, CreditAccount, CreditError, CreditNovation,
    ExternalSettlementRequest, ReceiptApplication, ServiceReceiptClaim, ValueClass,
};
use common::*;

#[test]
fn backing_deposits_are_idempotent_and_issuer_authenticated() {
    let mut account = new_account();
    let backing = deposit(
        "deposit-1",
        "alice",
        50,
        ValueClass::ReserveBackedWithdrawable,
    );

    assert_eq!(
        account.record_backing_deposit(&backing, "alice").unwrap(),
        ReceiptApplication::Applied
    );
    assert_eq!(
        account.record_backing_deposit(&backing, "alice").unwrap(),
        ReceiptApplication::AlreadyApplied
    );
    assert_eq!(account.sat_reserve("alice").unwrap().available_sat(), 50);

    let other_issuer = deposit("deposit-1", "carol", 7, ValueClass::ClosedLoopDeposit);
    assert_eq!(
        account
            .record_backing_deposit(&other_issuer, "carol")
            .unwrap(),
        ReceiptApplication::Applied
    );
    assert_eq!(
        account
            .backing_deposits()
            .map(|deposit| (deposit.issuer.as_str(), deposit.deposit_id.as_str()))
            .collect::<Vec<_>>(),
        [("alice", "deposit-1"), ("carol", "deposit-1")]
    );

    let mut conflict = backing.clone();
    conflict.amount_sat = 51;
    assert_eq!(
        account.record_backing_deposit(&conflict, "alice"),
        Err(CreditError::DepositConflict)
    );
    assert_eq!(
        account.record_backing_deposit(
            &deposit("deposit-2", "alice", 1, ValueClass::ClosedLoopDeposit,),
            "mallory"
        ),
        Err(CreditError::UnauthenticatedIssuer)
    );
}

#[test]
fn proof_backed_credit_settlement_is_idempotent_and_party_bound() {
    let mut account = new_account();
    account
        .apply_receipt(
            &claim_from("alice", "debt", 40),
            "alice",
            AcceptanceMode::Online,
            NOW,
        )
        .unwrap();
    account
        .record_backing_deposit(
            &deposit("deposit", "carol", 30, ValueClass::ClosedLoopDeposit),
            "carol",
        )
        .unwrap();
    let settlement = backed_settlement(
        "backed-1",
        "alice",
        "carol",
        20,
        ValueClass::ClosedLoopDeposit,
    );

    assert_eq!(
        account
            .settle_peer_credit_with_backing(&settlement, "carol", NOW)
            .unwrap(),
        ReceiptApplication::Applied
    );
    assert_eq!(
        account
            .settle_peer_credit_with_backing(&settlement, "carol", NOW)
            .unwrap(),
        ReceiptApplication::AlreadyApplied
    );
    assert_eq!(account.total_peer_credit_sat(), 20);
    assert_eq!(account.closed_loop("carol").unwrap().claimable_sat(), 20);

    let mut conflict = settlement.clone();
    conflict.amount_sat = 21;
    assert_eq!(
        account.settle_peer_credit_with_backing(&conflict, "carol", NOW),
        Err(CreditError::BackedSettlementConflict)
    );
    assert_eq!(
        account.settle_peer_credit_with_backing(
            &backed_settlement(
                "backed-2",
                "alice",
                "carol",
                1,
                ValueClass::ClosedLoopDeposit,
            ),
            "mallory",
            NOW,
        ),
        Err(CreditError::UnauthenticatedIssuer)
    );
}

#[test]
fn receipt_application_is_idempotent_and_conflicts_are_rejected() {
    let mut account = new_account();
    let receipt = claim("r1", 10, ValueClass::PeerCredit);

    assert_eq!(
        account
            .apply_receipt(&receipt, "alice", AcceptanceMode::Online, NOW)
            .unwrap(),
        ReceiptApplication::Applied
    );
    assert_eq!(
        account
            .apply_receipt(&receipt, "alice", AcceptanceMode::Online, NOW)
            .unwrap(),
        ReceiptApplication::AlreadyApplied
    );
    assert_eq!(account.peer_credit("alice").unwrap().outstanding_sat(), 10);

    let mut conflict = receipt;
    conflict.amount_sat = 11;
    assert_eq!(
        account.apply_receipt(&conflict, "alice", AcceptanceMode::Online, NOW),
        Err(CreditError::ReceiptConflict)
    );

    let mut rebound_resource = claim("r1", 10, ValueClass::PeerCredit);
    rebound_resource.resource = "fmp:generic-forwarded-bytes".into();
    assert_eq!(
        account.apply_receipt(&rebound_resource, "alice", AcceptanceMode::Online, NOW),
        Err(CreditError::ReceiptConflict)
    );
}

#[test]
fn committed_operations_remain_idempotent_after_their_deadline() {
    let mut account = new_account();
    let receipt = claim("deadline-receipt", 40, ValueClass::PeerCredit);
    account
        .apply_receipt(&receipt, "alice", AcceptanceMode::Online, NOW)
        .unwrap();
    assert_eq!(
        account
            .apply_receipt(&receipt, "alice", AcceptanceMode::Online, NOW + 20)
            .unwrap(),
        ReceiptApplication::AlreadyApplied
    );

    let novation = CreditNovation {
        novation_id: "deadline-novation".into(),
        counterparty: "bob".into(),
        from_issuer: "alice".into(),
        to_issuer: "carol".into(),
        amount_sat: 10,
        expires_at_unix: NOW + 10,
    };
    account.novate_peer_credit(&novation, "carol", NOW).unwrap();
    account
        .novate_peer_credit(&novation, "carol", NOW + 20)
        .unwrap();

    account
        .record_backing_deposit(
            &deposit(
                "deadline-deposit",
                "carol",
                20,
                ValueClass::ClosedLoopDeposit,
            ),
            "carol",
        )
        .unwrap();
    let backed = backed_settlement(
        "deadline-backed",
        "alice",
        "carol",
        10,
        ValueClass::ClosedLoopDeposit,
    );
    account
        .settle_peer_credit_with_backing(&backed, "carol", NOW)
        .unwrap();
    assert_eq!(
        account
            .settle_peer_credit_with_backing(&backed, "carol", NOW + 20)
            .unwrap(),
        ReceiptApplication::AlreadyApplied
    );

    account
        .record_backing_deposit(
            &deposit(
                "deadline-withdrawable",
                "alice",
                10,
                ValueClass::ReserveBackedWithdrawable,
            ),
            "alice",
        )
        .unwrap();
    account
        .apply_receipt(
            &claim(
                "deadline-reserve-receipt",
                10,
                ValueClass::ReserveBackedWithdrawable,
            ),
            "alice",
            AcceptanceMode::Online,
            NOW,
        )
        .unwrap();
    let external = settlement("deadline-external", 10);
    let first = account
        .authorize_external_settlement(&external, "bob", NOW)
        .unwrap();
    let replay = account
        .authorize_external_settlement(&external, "bob", NOW + 20)
        .unwrap();
    assert_eq!(replay, first);
}

#[test]
fn arithmetic_overflow_never_changes_ledgers() {
    let mut overflow_policy = policy();
    overflow_policy.max_total_peer_credit_sat = u64::MAX;
    overflow_policy.issuers[0].max_peer_credit_sat = u64::MAX;
    overflow_policy.issuers[0].max_offline_peer_credit_sat = u64::MAX;
    overflow_policy.issuers[0].max_withdrawable_sat = u64::MAX;
    let mut account = CreditAccount::new(overflow_policy).unwrap();

    account
        .apply_receipt(
            &claim("max", u64::MAX, ValueClass::PeerCredit),
            "alice",
            AcceptanceMode::Online,
            NOW,
        )
        .unwrap();
    assert_eq!(
        account.apply_receipt(
            &claim("overflow", 1, ValueClass::PeerCredit),
            "alice",
            AcceptanceMode::Online,
            NOW,
        ),
        Err(CreditError::ArithmeticOverflow)
    );
    assert_eq!(
        account.peer_credit("alice").unwrap().outstanding_sat(),
        u64::MAX
    );

    let mut reserve_account = new_account();
    reserve_account
        .record_backing_deposit(
            &deposit(
                "max-deposit",
                "alice",
                u64::MAX,
                ValueClass::ReserveBackedWithdrawable,
            ),
            "alice",
        )
        .unwrap();
    assert_eq!(
        reserve_account.record_backing_deposit(
            &deposit(
                "overflow-deposit",
                "alice",
                1,
                ValueClass::ReserveBackedWithdrawable,
            ),
            "alice",
        ),
        Err(CreditError::ArithmeticOverflow)
    );
    assert_eq!(
        reserve_account
            .sat_reserve("alice")
            .unwrap()
            .available_sat(),
        u64::MAX
    );
}

#[test]
fn authenticated_issuer_parties_and_unit_are_enforced() {
    let mut account = new_account();
    let receipt = claim("r1", 1, ValueClass::PeerCredit);

    assert_eq!(
        account.apply_receipt(&receipt, "mallory", AcceptanceMode::Online, NOW),
        Err(CreditError::UnauthenticatedIssuer)
    );

    let mut wrong_issuer = receipt.clone();
    wrong_issuer.issuer = "mallory".into();
    assert_eq!(
        account.apply_receipt(&wrong_issuer, "mallory", AcceptanceMode::Online, NOW),
        Err(CreditError::WrongIssuer)
    );

    let mut wrong_counterparty = receipt.clone();
    wrong_counterparty.counterparty = "carol".into();
    assert_eq!(
        account.apply_receipt(&wrong_counterparty, "alice", AcceptanceMode::Online, NOW),
        Err(CreditError::WrongCounterparty)
    );

    assert_eq!(
        account.apply_receipt(
            &claim("reserve", 1, ValueClass::ReserveBackedWithdrawable),
            "alice",
            AcceptanceMode::OfflineDeferred,
            NOW
        ),
        Err(CreditError::BackingVerificationRequired)
    );
}

#[test]
fn expired_policy_receipt_and_settlement_are_rejected() {
    let mut expired_policy = policy();
    expired_policy.issuers[0].expires_at_unix = Some(NOW);
    let mut expired_account = CreditAccount::new(expired_policy).unwrap();
    assert_eq!(
        expired_account.apply_receipt(
            &claim("r1", 1, ValueClass::PeerCredit),
            "alice",
            AcceptanceMode::Online,
            NOW
        ),
        Err(CreditError::PolicyExpired)
    );

    let mut account = new_account();
    let mut expired_receipt = claim("r2", 1, ValueClass::PeerCredit);
    expired_receipt.expires_at_unix = NOW;
    assert_eq!(
        account.apply_receipt(&expired_receipt, "alice", AcceptanceMode::Online, NOW),
        Err(CreditError::ReceiptExpired)
    );

    assert_eq!(
        account.authorize_external_settlement(
            &ExternalSettlementRequest {
                expires_at_unix: NOW,
                ..settlement("s1", 1)
            },
            "bob",
            NOW
        ),
        Err(CreditError::SettlementExpired)
    );
}

#[test]
fn empty_service_resource_and_zero_units_are_rejected() {
    let mut account = new_account();

    let mut no_service = claim("r1", 1, ValueClass::PeerCredit);
    no_service.useful_service_units = 0;
    assert_eq!(
        account.apply_receipt(&no_service, "alice", AcceptanceMode::Online, NOW),
        Err(CreditError::NoUsefulService)
    );

    let mut unnamed_service = claim("r2", 1, ValueClass::PeerCredit);
    unnamed_service.service.clear();
    assert_eq!(
        account.apply_receipt(&unnamed_service, "alice", AcceptanceMode::Online, NOW),
        Err(CreditError::NoUsefulService)
    );

    let mut unnamed_resource = claim("r2-resource", 1, ValueClass::PeerCredit);
    unnamed_resource.resource.clear();
    assert_eq!(
        account.apply_receipt(&unnamed_resource, "alice", AcceptanceMode::Online, NOW),
        Err(CreditError::NoUsefulService)
    );

    assert_eq!(
        account.apply_receipt(
            &claim("r3", 0, ValueClass::PeerCredit),
            "alice",
            AcceptanceMode::Online,
            NOW
        ),
        Err(CreditError::ZeroAmount)
    );
    assert_eq!(account.peer_credit("alice").unwrap().outstanding_sat(), 0);
}

#[test]
fn reserve_is_conserved_across_claim_authorize_and_complete() {
    let mut account = new_account();
    account
        .record_backing_deposit(
            &deposit(
                "reserve-deposit",
                "alice",
                100,
                ValueClass::ReserveBackedWithdrawable,
            ),
            "alice",
        )
        .unwrap();
    account
        .apply_receipt(
            &claim("r1", 30, ValueClass::ReserveBackedWithdrawable),
            "alice",
            AcceptanceMode::Online,
            NOW,
        )
        .unwrap();

    assert_eq!(account.sat_reserve("alice").unwrap().available_sat(), 70);
    assert_eq!(account.sat_reserve("alice").unwrap().redeemable_sat(), 30);
    assert_eq!(
        account
            .sat_reserve("alice")
            .unwrap()
            .conserved_sat()
            .unwrap(),
        100
    );

    let mut request = settlement("s1", 20);
    request.max_fee_sat = 2;
    let authorization = account
        .authorize_external_settlement(&request, "bob", NOW)
        .unwrap();
    assert_eq!(authorization.amount_sat, 20);
    assert_eq!(authorization.reserved_sat, 22);
    assert_eq!(account.sat_reserve("alice").unwrap().redeemable_sat(), 8);
    assert_eq!(
        account.sat_reserve("alice").unwrap().pending_external_sat(),
        22
    );
    assert_eq!(
        account
            .sat_reserve("alice")
            .unwrap()
            .conserved_sat()
            .unwrap(),
        100
    );

    account.complete_external_settlement("s1", 1).unwrap();
    assert_eq!(
        account.sat_reserve("alice").unwrap().pending_external_sat(),
        0
    );
    assert_eq!(
        account.sat_reserve("alice").unwrap().settled_external_sat(),
        21
    );
    assert_eq!(account.sat_reserve("alice").unwrap().redeemable_sat(), 9);
    assert_eq!(
        account
            .sat_reserve("alice")
            .unwrap()
            .conserved_sat()
            .unwrap(),
        100
    );
}

#[test]
fn offline_credit_has_an_independent_exposure_cap() {
    let mut account = new_account();
    account
        .apply_receipt(
            &claim("r1", 20, ValueClass::PeerCredit),
            "alice",
            AcceptanceMode::OfflineDeferred,
            NOW,
        )
        .unwrap();
    assert_eq!(
        account
            .peer_credit("alice")
            .unwrap()
            .offline_outstanding_sat(),
        20
    );

    assert_eq!(
        account.apply_receipt(
            &claim("r2", 1, ValueClass::PeerCredit),
            "alice",
            AcceptanceMode::OfflineDeferred,
            NOW,
        ),
        Err(CreditError::OfflineExposureExceeded)
    );
    assert_eq!(account.peer_credit("alice").unwrap().outstanding_sat(), 20);
}

#[test]
fn peer_credit_can_never_authorize_external_settlement() {
    let mut account = new_account();
    account
        .apply_receipt(
            &claim("credit", 100, ValueClass::PeerCredit),
            "alice",
            AcceptanceMode::Online,
            NOW,
        )
        .unwrap();

    assert_eq!(
        account.authorize_external_settlement(&settlement("steal", 1), "bob", NOW),
        Err(CreditError::InsufficientRedeemableReserve)
    );
    assert_eq!(account.peer_credit("alice").unwrap().outstanding_sat(), 100);
}

#[test]
fn settlement_authorization_is_idempotent_and_party_bound() {
    let mut account = new_account();
    account
        .record_backing_deposit(
            &deposit(
                "settlement-deposit",
                "alice",
                50,
                ValueClass::ReserveBackedWithdrawable,
            ),
            "alice",
        )
        .unwrap();
    account
        .apply_receipt(
            &claim("reserve", 50, ValueClass::ReserveBackedWithdrawable),
            "alice",
            AcceptanceMode::Online,
            NOW,
        )
        .unwrap();

    let mut request = settlement("s1", 20);
    request.max_fee_sat = 2;
    let first = account
        .authorize_external_settlement(&request, "bob", NOW)
        .unwrap();
    let replay = account
        .authorize_external_settlement(&request, "bob", NOW)
        .unwrap();
    assert_eq!(first, replay);
    assert_eq!(
        account.sat_reserve("alice").unwrap().pending_external_sat(),
        22
    );

    let mut conflict = request.clone();
    conflict.amount_sat = 21;
    assert_eq!(
        account.authorize_external_settlement(&conflict, "bob", NOW),
        Err(CreditError::SettlementConflict)
    );

    let mut redirected = request.clone();
    redirected.payout_destination = "https://mallory.example".into();
    assert_eq!(
        account.authorize_external_settlement(&redirected, "bob", NOW),
        Err(CreditError::SettlementConflict)
    );

    let mut fee_increased = request.clone();
    fee_increased.max_fee_sat = 3;
    assert_eq!(
        account.authorize_external_settlement(&fee_increased, "bob", NOW),
        Err(CreditError::SettlementConflict)
    );

    let mut wrong_party = settlement("s2", 1);
    wrong_party.counterparty = "mallory".into();
    assert_eq!(
        account.authorize_external_settlement(&wrong_party, "mallory", NOW),
        Err(CreditError::WrongCounterparty)
    );

    assert_eq!(
        account.authorize_external_settlement(&settlement("s3", 1), "mallory", NOW),
        Err(CreditError::UnauthenticatedCounterparty)
    );

    let mut missing_destination = settlement("s4", 1);
    missing_destination.payout_destination.clear();
    assert_eq!(
        account.authorize_external_settlement(&missing_destination, "bob", NOW),
        Err(CreditError::MissingPayoutDestination)
    );

    assert_eq!(
        account.complete_external_settlement("s1", 3),
        Err(CreditError::SettlementFeeExceeded)
    );
    assert_eq!(
        account.sat_reserve("alice").unwrap().pending_external_sat(),
        22
    );
    account.complete_external_settlement("s1", 1).unwrap();
    account.complete_external_settlement("s1", 1).unwrap();
    assert_eq!(
        account.complete_external_settlement("s1", 2),
        Err(CreditError::SettlementCompletionConflict)
    );
    assert_eq!(account.sat_reserve("alice").unwrap().redeemable_sat(), 29);
    assert_eq!(
        account.sat_reserve("alice").unwrap().settled_external_sat(),
        21
    );
}

#[test]
fn per_issuer_and_total_soft_exposure_are_both_bounded() {
    let mut account = new_account();
    account
        .apply_receipt(
            &claim_from("alice", "a", 100),
            "alice",
            AcceptanceMode::Online,
            NOW,
        )
        .unwrap();
    account
        .apply_receipt(
            &claim_from("carol", "c", 20),
            "carol",
            AcceptanceMode::Online,
            NOW,
        )
        .unwrap();
    assert_eq!(account.total_peer_credit_sat(), 120);

    assert_eq!(
        account.apply_receipt(
            &claim_from("carol", "total-cap", 1),
            "carol",
            AcceptanceMode::Online,
            NOW,
        ),
        Err(CreditError::TotalExposureExceeded)
    );

    let mut issuer_account = new_account();
    assert_eq!(
        issuer_account.apply_receipt(
            &claim_from("carol", "issuer-cap", 51),
            "carol",
            AcceptanceMode::Online,
            NOW,
        ),
        Err(CreditError::IssuerExposureExceeded)
    );
}

#[test]
fn trusted_peer_novation_moves_but_never_destroys_soft_exposure() {
    let mut account = new_account();
    account
        .apply_receipt(
            &claim_from("alice", "debt", 80),
            "alice",
            AcceptanceMode::Online,
            NOW,
        )
        .unwrap();
    let novation = CreditNovation {
        novation_id: "n1".into(),
        counterparty: "bob".into(),
        from_issuer: "alice".into(),
        to_issuer: "carol".into(),
        amount_sat: 30,
        expires_at_unix: NOW + 10,
    };

    account.novate_peer_credit(&novation, "carol", NOW).unwrap();
    account.novate_peer_credit(&novation, "carol", NOW).unwrap();
    assert_eq!(account.peer_credit("alice").unwrap().outstanding_sat(), 50);
    assert_eq!(account.peer_credit("carol").unwrap().outstanding_sat(), 30);
    assert_eq!(account.total_peer_credit_sat(), 80);

    let mut carol_cashout = settlement("carol-cashout", 1);
    carol_cashout.issuer = "carol".into();
    assert_eq!(
        account.authorize_external_settlement(&carol_cashout, "bob", NOW),
        Err(CreditError::InsufficientRedeemableReserve)
    );

    assert_eq!(
        account.novate_peer_credit(
            &CreditNovation {
                novation_id: "over-carol-cap".into(),
                amount_sat: 21,
                ..novation
            },
            "carol",
            NOW
        ),
        Err(CreditError::IssuerExposureExceeded)
    );
    assert_eq!(account.total_peer_credit_sat(), 80);
}

#[test]
fn withdrawable_backing_extinguishes_soft_exposure_and_enables_cashout() {
    let mut account = new_account();
    account
        .apply_receipt(
            &claim_from("alice", "debt", 40),
            "alice",
            AcceptanceMode::Online,
            NOW,
        )
        .unwrap();
    account
        .record_backing_deposit(
            &deposit(
                "withdrawable-deposit",
                "alice",
                20,
                ValueClass::ReserveBackedWithdrawable,
            ),
            "alice",
        )
        .unwrap();

    account
        .settle_peer_credit_with_backing(
            &backed_settlement(
                "withdrawable-conversion",
                "alice",
                "alice",
                20,
                ValueClass::ReserveBackedWithdrawable,
            ),
            "alice",
            NOW,
        )
        .unwrap();
    assert_eq!(account.total_peer_credit_sat(), 20);
    assert_eq!(account.peer_credit("alice").unwrap().outstanding_sat(), 20);
    assert_eq!(account.sat_reserve("alice").unwrap().redeemable_sat(), 20);

    account
        .authorize_external_settlement(&settlement("cashout", 20), "bob", NOW)
        .unwrap();
}

#[test]
fn closed_loop_backing_settles_peer_credit_without_creating_cashout() {
    let mut account = new_account();
    account
        .apply_receipt(
            &claim_from("alice", "debt", 40),
            "alice",
            AcceptanceMode::Online,
            NOW,
        )
        .unwrap();
    account
        .record_backing_deposit(
            &deposit(
                "closed-loop-deposit",
                "carol",
                20,
                ValueClass::ClosedLoopDeposit,
            ),
            "carol",
        )
        .unwrap();

    account
        .settle_peer_credit_with_backing(
            &backed_settlement(
                "closed-loop-conversion",
                "alice",
                "carol",
                20,
                ValueClass::ClosedLoopDeposit,
            ),
            "carol",
            NOW,
        )
        .unwrap();
    assert_eq!(account.total_peer_credit_sat(), 20);
    assert_eq!(account.peer_credit("alice").unwrap().outstanding_sat(), 20);
    assert_eq!(account.closed_loop("carol").unwrap().claimable_sat(), 20);
    assert_eq!(
        account
            .closed_loop("carol")
            .unwrap()
            .conserved_sat()
            .unwrap(),
        20
    );

    let mut request = settlement("no-transitive-cashout", 1);
    request.issuer = "carol".into();
    assert_eq!(
        account.authorize_external_settlement(&request, "bob", NOW),
        Err(CreditError::InsufficientRedeemableReserve)
    );
}

#[test]
fn closed_loop_receipts_require_backing_and_never_touch_withdrawable_reserve() {
    let mut account = new_account();
    account
        .record_backing_deposit(
            &deposit(
                "direct-closed-deposit",
                "alice",
                10,
                ValueClass::ClosedLoopDeposit,
            ),
            "alice",
        )
        .unwrap();
    let receipt = claim("closed", 10, ValueClass::ClosedLoopDeposit);

    assert_eq!(
        account.apply_receipt(&receipt, "alice", AcceptanceMode::OfflineDeferred, NOW),
        Err(CreditError::BackingVerificationRequired)
    );
    account
        .apply_receipt(&receipt, "alice", AcceptanceMode::Online, NOW)
        .unwrap();
    assert_eq!(account.closed_loop("alice").unwrap().claimable_sat(), 10);

    let consumption = ClosedLoopConsumption {
        consumption_id: "buy-bandwidth".into(),
        issuer: "alice".into(),
        counterparty: "bob".into(),
        resource: "fsp:destination:service:budget".into(),
        amount_sat: 4,
    };
    assert_eq!(
        account.consume_closed_loop(&consumption, "bob").unwrap(),
        ReceiptApplication::Applied
    );
    assert_eq!(
        account.consume_closed_loop(&consumption, "bob").unwrap(),
        ReceiptApplication::AlreadyApplied
    );
    let mut conflict = consumption;
    conflict.amount_sat = 5;
    assert_eq!(
        account.consume_closed_loop(&conflict, "bob"),
        Err(CreditError::ClosedLoopConsumptionConflict)
    );
    assert_eq!(account.closed_loop("alice").unwrap().claimable_sat(), 6);
    assert_eq!(account.closed_loop("alice").unwrap().consumed_sat(), 4);
    assert_eq!(account.sat_reserve("alice").unwrap().redeemable_sat(), 0);
    assert_eq!(
        account.authorize_external_settlement(&settlement("closed-cashout", 1), "bob", NOW),
        Err(CreditError::InsufficientRedeemableReserve)
    );
}

#[test]
fn protocol_claims_roundtrip_without_a_signature_field() {
    let receipt = claim("receipt", 7, ValueClass::PeerCredit);
    let encoded = serde_json::to_string(&receipt).unwrap();
    let decoded: ServiceReceiptClaim = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded, receipt);
    assert!(!encoded.contains("signature"));
}
