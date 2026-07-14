#![allow(dead_code)]

use cashu_credit::{
    AccountPolicy, BackedCreditSettlement, BackingDeposit, CreditAccount,
    ExternalSettlementRequest, IssuerPolicy, ServiceReceiptClaim, ValueClass,
};

pub const NOW: u64 = 1_000;

pub fn policy() -> AccountPolicy {
    AccountPolicy {
        counterparty: "bob".into(),
        max_total_peer_credit_sat: 120,
        issuers: vec![
            IssuerPolicy {
                issuer: "alice".into(),
                max_peer_credit_sat: 100,
                max_offline_peer_credit_sat: 20,
                max_closed_loop_sat: 200,
                max_withdrawable_sat: 500,
                expires_at_unix: Some(NOW + 100),
            },
            IssuerPolicy {
                issuer: "carol".into(),
                max_peer_credit_sat: 50,
                max_offline_peer_credit_sat: 10,
                max_closed_loop_sat: 100,
                max_withdrawable_sat: 100,
                expires_at_unix: Some(NOW + 100),
            },
        ],
    }
}

pub fn new_account() -> CreditAccount {
    CreditAccount::new(policy()).unwrap()
}

pub fn claim(id: &str, amount_sat: u64, value_class: ValueClass) -> ServiceReceiptClaim {
    ServiceReceiptClaim {
        receipt_id: id.into(),
        issuer: "alice".into(),
        counterparty: "bob".into(),
        service: "pubsub_delivery".into(),
        resource: "subscription:alice:kind-1".into(),
        useful_service_units: 4_096,
        amount_sat,
        value_class,
        issued_at_unix: NOW - 10,
        expires_at_unix: NOW + 10,
    }
}

pub fn settlement(id: &str, amount_sat: u64) -> ExternalSettlementRequest {
    ExternalSettlementRequest {
        settlement_id: id.into(),
        issuer: "alice".into(),
        counterparty: "bob".into(),
        payout_destination: "https://mint.example".into(),
        amount_sat,
        max_fee_sat: 0,
        expires_at_unix: NOW + 10,
    }
}

pub fn claim_from(issuer: &str, id: &str, amount_sat: u64) -> ServiceReceiptClaim {
    ServiceReceiptClaim {
        issuer: issuer.into(),
        ..claim(id, amount_sat, ValueClass::PeerCredit)
    }
}

pub fn deposit(id: &str, issuer: &str, amount_sat: u64, value_class: ValueClass) -> BackingDeposit {
    BackingDeposit {
        deposit_id: id.into(),
        issuer: issuer.into(),
        amount_sat,
        value_class,
    }
}

pub fn backed_settlement(
    id: &str,
    from_issuer: &str,
    backing_issuer: &str,
    amount_sat: u64,
    value_class: ValueClass,
) -> BackedCreditSettlement {
    BackedCreditSettlement {
        settlement_id: id.into(),
        counterparty: "bob".into(),
        from_issuer: from_issuer.into(),
        backing_issuer: backing_issuer.into(),
        amount_sat,
        value_class,
        expires_at_unix: NOW + 10,
    }
}
