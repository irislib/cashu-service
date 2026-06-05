use serde::{Deserialize, Serialize};

/// `sat` is the only accounting unit for credit lines in this crate.
pub const CREDIT_UNIT: &str = "sat";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettlementMethod {
    /// Settle with tokens from a Cashu mint the creditor accepts.
    CashuMint { mint_url: String },
    /// Settle by paying a Lightning invoice.
    Lightning,
    /// Relationship-local settlement outside this protocol.
    Manual { label: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OfflineAcceptancePolicy {
    /// Maximum debt that may be accepted without contacting the debtor's mint.
    pub max_offline_debt_sat: u64,
    /// Maximum age for a peer-issued token accepted while offline.
    pub max_token_age_secs: u64,
}

impl Default for OfflineAcceptancePolicy {
    fn default() -> Self {
        Self {
            max_offline_debt_sat: 0,
            max_token_age_secs: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreditLine {
    /// Peer whose issued Cashu may be accepted as credit.
    pub debtor: String,
    /// Peer accepting that issued Cashu up to `credit_limit_sat`.
    pub creditor: String,
    /// Debtor-operated or debtor-authorized mint for this credit line.
    pub debtor_mint_url: String,
    /// Maximum outstanding debt, denominated in sats.
    pub credit_limit_sat: u64,
    /// Start asking for refill/settlement at or above this outstanding debt.
    pub settlement_threshold_sat: u64,
    /// Assets or mechanisms the creditor accepts for refilling the line.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub settlement_methods: Vec<SettlementMethod>,
    /// Relationship-local offline risk budget.
    #[serde(default)]
    pub offline: OfflineAcceptancePolicy,
    /// Optional absolute expiry for this trustline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_unix: Option<u64>,
    /// Free-form policy hints such as "social_graph_distance=1".
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

impl CreditLine {
    pub fn normalized(mut self) -> Self {
        if self.settlement_threshold_sat == 0
            || self.settlement_threshold_sat > self.credit_limit_sat
        {
            self.settlement_threshold_sat = self.credit_limit_sat;
        }
        self
    }

    pub fn available_credit_sat(&self, balance: &CreditBalance) -> u64 {
        self.credit_limit_sat
            .saturating_sub(balance.outstanding_debt_sat)
    }

    pub fn settlement_due(&self, balance: &CreditBalance) -> bool {
        balance.outstanding_debt_sat >= self.settlement_threshold_sat
            && balance.outstanding_debt_sat > 0
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreditBalance {
    /// Accepted debtor-issued tokens not yet settled, denominated in sats.
    pub outstanding_debt_sat: u64,
    /// Settlement initiated but not finalized.
    #[serde(default)]
    pub pending_settlement_sat: u64,
    /// Monotonic relationship-local revision for conflict detection.
    #[serde(default)]
    pub revision: u64,
    /// Last local accounting update timestamp.
    #[serde(default)]
    pub updated_at_unix: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerCreditToken {
    pub debtor: String,
    pub creditor: String,
    pub mint_url: String,
    pub amount_sat: u64,
    pub issued_at_unix: u64,
    pub token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettlementRequest {
    pub debtor: String,
    pub creditor: String,
    pub requested_sat: u64,
    pub outstanding_debt_sat: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accepted_methods: Vec<SettlementMethod>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettlementReceipt {
    pub debtor: String,
    pub creditor: String,
    pub settled_sat: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<SettlementMethod>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    pub settled_at_unix: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcceptanceMode {
    OnlineVerified,
    OfflineDeferred,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptanceDecision {
    pub mode: AcceptanceMode,
    pub resulting_debt_sat: u64,
    pub settlement_due: bool,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CreditError {
    #[error("credit line has expired")]
    Expired,
    #[error("token is for a different debtor")]
    WrongDebtor,
    #[error("token is for a different creditor")]
    WrongCreditor,
    #[error("token mint does not match credit line")]
    WrongMint,
    #[error("credit limit exceeded")]
    CreditLimitExceeded,
    #[error("offline credit limit exceeded")]
    OfflineCreditLimitExceeded,
    #[error("offline token is too old")]
    OfflineTokenTooOld,
}

pub fn assess_peer_credit_token(
    line: &CreditLine,
    balance: &CreditBalance,
    token: &PeerCreditToken,
    now_unix: u64,
    mode: AcceptanceMode,
) -> Result<AcceptanceDecision, CreditError> {
    let line = line.clone().normalized();
    if line.expires_unix.is_some_and(|expires| expires <= now_unix) {
        return Err(CreditError::Expired);
    }
    if token.debtor != line.debtor {
        return Err(CreditError::WrongDebtor);
    }
    if token.creditor != line.creditor {
        return Err(CreditError::WrongCreditor);
    }
    if token.mint_url != line.debtor_mint_url {
        return Err(CreditError::WrongMint);
    }

    let resulting_debt_sat = balance
        .outstanding_debt_sat
        .checked_add(token.amount_sat)
        .ok_or(CreditError::CreditLimitExceeded)?;
    if resulting_debt_sat > line.credit_limit_sat {
        return Err(CreditError::CreditLimitExceeded);
    }

    if mode == AcceptanceMode::OfflineDeferred {
        if resulting_debt_sat > line.offline.max_offline_debt_sat {
            return Err(CreditError::OfflineCreditLimitExceeded);
        }
        if line.offline.max_token_age_secs == 0 {
            return Err(CreditError::OfflineTokenTooOld);
        }
        let age = now_unix.saturating_sub(token.issued_at_unix);
        if age > line.offline.max_token_age_secs {
            return Err(CreditError::OfflineTokenTooOld);
        }
    }

    Ok(AcceptanceDecision {
        mode,
        resulting_debt_sat,
        settlement_due: line.settlement_due(&CreditBalance {
            outstanding_debt_sat: resulting_debt_sat,
            ..balance.clone()
        }),
    })
}

pub fn record_peer_credit_acceptance(
    balance: &mut CreditBalance,
    accepted_sat: u64,
    updated_at_unix: u64,
) {
    balance.outstanding_debt_sat = balance.outstanding_debt_sat.saturating_add(accepted_sat);
    balance.revision = balance.revision.saturating_add(1);
    balance.updated_at_unix = updated_at_unix;
}

pub fn record_settlement(balance: &mut CreditBalance, settled_sat: u64, updated_at_unix: u64) {
    balance.outstanding_debt_sat = balance.outstanding_debt_sat.saturating_sub(settled_sat);
    balance.pending_settlement_sat = balance.pending_settlement_sat.saturating_sub(settled_sat);
    balance.revision = balance.revision.saturating_add(1);
    balance.updated_at_unix = updated_at_unix;
}

pub fn settlement_request(line: &CreditLine, balance: &CreditBalance) -> Option<SettlementRequest> {
    let line = line.clone().normalized();
    if !line.settlement_due(balance) {
        return None;
    }
    Some(SettlementRequest {
        debtor: line.debtor,
        creditor: line.creditor,
        requested_sat: balance.outstanding_debt_sat,
        outstanding_debt_sat: balance.outstanding_debt_sat,
        accepted_methods: line.settlement_methods,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line() -> CreditLine {
        CreditLine {
            debtor: "npub1alice".to_string(),
            creditor: "npub1bob".to_string(),
            debtor_mint_url: "cashu+fips://npub1alice/local".to_string(),
            credit_limit_sat: 1_000,
            settlement_threshold_sat: 800,
            settlement_methods: vec![
                SettlementMethod::CashuMint {
                    mint_url: "https://mint.example".to_string(),
                },
                SettlementMethod::Lightning,
            ],
            offline: OfflineAcceptancePolicy {
                max_offline_debt_sat: 100,
                max_token_age_secs: 60,
            },
            expires_unix: None,
            tags: vec!["social_distance=1".to_string()],
        }
    }

    fn token(amount_sat: u64, issued_at_unix: u64) -> PeerCreditToken {
        PeerCreditToken {
            debtor: "npub1alice".to_string(),
            creditor: "npub1bob".to_string(),
            mint_url: "cashu+fips://npub1alice/local".to_string(),
            amount_sat,
            issued_at_unix,
            token: "cashuAey...".to_string(),
        }
    }

    #[test]
    fn online_acceptance_tracks_sat_denominated_debt() {
        let balance = CreditBalance {
            outstanding_debt_sat: 700,
            ..CreditBalance::default()
        };
        let decision = assess_peer_credit_token(
            &line(),
            &balance,
            &token(100, 10),
            20,
            AcceptanceMode::OnlineVerified,
        )
        .expect("token accepted");
        assert_eq!(decision.resulting_debt_sat, 800);
        assert!(decision.settlement_due);
    }

    #[test]
    fn credit_limit_is_enforced() {
        let balance = CreditBalance {
            outstanding_debt_sat: 950,
            ..CreditBalance::default()
        };
        let error = assess_peer_credit_token(
            &line(),
            &balance,
            &token(100, 10),
            20,
            AcceptanceMode::OnlineVerified,
        )
        .expect_err("limit exceeded");
        assert_eq!(error, CreditError::CreditLimitExceeded);
    }

    #[test]
    fn offline_acceptance_has_separate_risk_budget() {
        let balance = CreditBalance {
            outstanding_debt_sat: 90,
            ..CreditBalance::default()
        };
        let error = assess_peer_credit_token(
            &line(),
            &balance,
            &token(20, 10),
            20,
            AcceptanceMode::OfflineDeferred,
        )
        .expect_err("offline limit exceeded");
        assert_eq!(error, CreditError::OfflineCreditLimitExceeded);
    }

    #[test]
    fn settlement_reduces_outstanding_debt() {
        let mut balance = CreditBalance {
            outstanding_debt_sat: 900,
            pending_settlement_sat: 250,
            revision: 7,
            updated_at_unix: 10,
        };
        record_settlement(&mut balance, 300, 20);
        assert_eq!(balance.outstanding_debt_sat, 600);
        assert_eq!(balance.pending_settlement_sat, 0);
        assert_eq!(balance.revision, 8);
        assert_eq!(balance.updated_at_unix, 20);
    }

    #[test]
    fn settlement_request_uses_accepted_methods() {
        let request = settlement_request(
            &line(),
            &CreditBalance {
                outstanding_debt_sat: 850,
                ..CreditBalance::default()
            },
        )
        .expect("settlement due");
        assert_eq!(request.requested_sat, 850);
        assert_eq!(request.accepted_methods.len(), 2);
    }

    #[test]
    fn structs_roundtrip_as_json() {
        let encoded = serde_json::to_string(&line()).expect("encode");
        let decoded: CreditLine = serde_json::from_str(&encoded).expect("decode");
        assert_eq!(decoded, line());
        assert_eq!(CREDIT_UNIT, "sat");
    }
}
