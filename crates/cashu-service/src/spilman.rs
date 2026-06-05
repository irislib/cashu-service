//! Experimental Cashu Spilman channel support for streaming paid routes.
//!
//! The upstream protocol and APIs are early-alpha. This module deliberately
//! exposes route-payment concepts and selected upstream primitives so consumers
//! can depend on `cashu-service` rather than coupling directly to the
//! implementation crate.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Upstream base revision for the local experimental Spilman checkout.
pub const CASHU_SPILMAN_CHANNELS_REV: &str = "bafc38f220e46289bebf157014ab1129c7deac63";

/// Git repository containing the experimental Spilman implementation.
pub const CASHU_SPILMAN_CHANNELS_REPO: &str =
    "https://github.com/SatsAndSports/cashu_spilman_channels.git";

/// Unit used by a metered paid route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamingRouteMeter {
    /// Charge by wall-clock time while a route lease is active.
    Milliseconds,
    /// Charge by tunneled bytes.
    Bytes,
    /// Charge by packets.
    Packets,
}

/// Seller-side policy for a metered route backed by a Cashu Spilman channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamingRoutePolicy {
    /// Meter used to compute the amount due.
    pub meter: StreamingRouteMeter,
    /// Price numerator in millisats.
    pub price_msat: u64,
    /// Price denominator in metered units.
    pub per_units: u64,
    /// Maximum buyer-funded channel capacity accepted for one route.
    pub max_channel_capacity_sat: u64,
    /// Expiry applied to newly opened channels.
    pub channel_expiry_secs: u64,
    /// Free probe budget before streaming payments are required.
    pub free_probe_units: u64,
    /// Small risk window allowed after the last valid balance update.
    pub grace_units: u64,
}

/// Seller routing state implied by route usage and the latest signed balance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamingRouteAccessState {
    /// Usage is still inside the seller's free probe allowance.
    FreeProbe,
    /// The latest signed balance fully covers current usage.
    Paid,
    /// Routing may continue, but only because the grace window still covers
    /// unpaid usage while the buyer sends a fresh balance update.
    Grace,
    /// Routing should stop until the buyer provides a larger signed balance.
    Suspended,
}

/// Route gating decision for a metered Spilman-backed route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamingRouteDecision {
    pub state: StreamingRouteAccessState,
    pub allow_routing: bool,
    pub delivered_units: u64,
    pub paid_msat: u64,
    pub amount_due_msat: u64,
    pub enforced_amount_due_msat: u64,
    pub unpaid_msat: u64,
    pub free_probe_remaining_units: u64,
    pub grace_remaining_units: u64,
}

/// Wire protocol version for Cashu-Spilman paid-route payment messages.
pub const STREAMING_ROUTE_PAYMENT_PROTOCOL_VERSION: u16 = 1;

/// Serializable snapshot of an upstream Spilman payment.
///
/// `balance` is denominated in the Cashu channel unit (`unit` in the route
/// envelope), while route accounting can additionally carry `paid_msat`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CashuSpilmanPayment {
    pub channel_id: String,
    pub balance: u64,
    pub signature: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub funding_proofs: Option<Value>,
}

impl CashuSpilmanPayment {
    pub fn has_funding(&self) -> bool {
        self.params.is_some() && self.funding_proofs.is_some()
    }
}

/// Transport-neutral payment message for a metered route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamingRoutePaymentEnvelope {
    #[serde(default = "default_streaming_route_payment_protocol_version")]
    pub version: u16,
    /// Stable service/offer id in the seller's namespace.
    pub service_id: String,
    pub lease_id: String,
    pub buyer: String,
    pub seller: String,
    pub sent_at_unix: u64,
    pub payload: StreamingRoutePaymentPayload,
}

impl StreamingRoutePaymentEnvelope {
    pub fn new(
        service_id: impl Into<String>,
        lease_id: impl Into<String>,
        buyer: impl Into<String>,
        seller: impl Into<String>,
        sent_at_unix: u64,
        payload: StreamingRoutePaymentPayload,
    ) -> Self {
        Self {
            version: STREAMING_ROUTE_PAYMENT_PROTOCOL_VERSION,
            service_id: service_id.into(),
            lease_id: lease_id.into(),
            buyer: buyer.into(),
            seller: seller.into(),
            sent_at_unix,
            payload,
        }
    }

    pub fn channel_id(&self) -> &str {
        self.payload.channel_id()
    }
}

/// Route payment payloads exchanged between buyer and seller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamingRoutePaymentPayload {
    /// First payment for a channel, usually containing funding parameters and
    /// proofs so the seller can register the channel before charging traffic.
    ChannelOpen(StreamingRouteChannelOpen),
    /// Monotonic balance update as usage grows.
    BalanceUpdate(StreamingRouteBalanceUpdate),
    /// Buyer asks the seller to cooperatively close at a final balance.
    CooperativeClose(StreamingRouteCooperativeClose),
    /// Seller acknowledges close processing. Settlement receipt format is
    /// deliberately opaque while the upstream protocol is experimental.
    CooperativeCloseAck(StreamingRouteCooperativeCloseAck),
}

impl StreamingRoutePaymentPayload {
    pub fn channel_id(&self) -> &str {
        match self {
            Self::ChannelOpen(open) => &open.payment.channel_id,
            Self::BalanceUpdate(update) => &update.payment.channel_id,
            Self::CooperativeClose(close) => &close.payment.channel_id,
            Self::CooperativeCloseAck(ack) => &ack.channel_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamingRouteChannelOpen {
    pub mint_url: String,
    /// Cashu unit for `capacity` and `payment.balance`, for example `sat` or
    /// `msat`.
    pub unit: String,
    pub capacity: u64,
    pub expires_unix: u64,
    pub receiver_pubkey_hex: String,
    pub paid_msat: u64,
    pub payment: CashuSpilmanPayment,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamingRouteBalanceUpdate {
    pub delivered_units: u64,
    pub amount_due_msat: u64,
    pub paid_msat: u64,
    pub payment: CashuSpilmanPayment,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamingRouteCooperativeClose {
    pub final_paid_msat: u64,
    pub payment: CashuSpilmanPayment,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamingRouteCooperativeCloseAck {
    pub channel_id: String,
    pub accepted_balance: u64,
    pub accepted_paid_msat: u64,
    pub closed_at_unix: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt: Option<Value>,
}

impl StreamingRoutePolicy {
    /// Computes the millisats due for delivered route usage.
    pub fn amount_due_msat(&self, delivered_units: u64) -> u64 {
        let billable = delivered_units.saturating_sub(self.free_probe_units);
        if billable == 0 || self.price_msat == 0 {
            return 0;
        }

        let per_units = self.per_units.max(1) as u128;
        let total = billable as u128 * self.price_msat as u128;
        total.div_ceil(per_units) as u64
    }

    /// Returns true when a signed balance is sufficient to keep routing.
    pub fn is_balance_sufficient(&self, delivered_units: u64, paid_msat: u64) -> bool {
        self.routing_decision(delivered_units, paid_msat)
            .allow_routing
    }

    /// Computes the current route-gating decision from delivered usage and the
    /// latest signed Spilman balance.
    pub fn routing_decision(&self, delivered_units: u64, paid_msat: u64) -> StreamingRouteDecision {
        let amount_due_msat = self.amount_due_msat(delivered_units);
        let free_probe_remaining_units = self.free_probe_units.saturating_sub(delivered_units);
        let grace_limit_units = self.free_probe_units.saturating_add(self.grace_units);
        let grace_remaining_units = grace_limit_units.saturating_sub(delivered_units);
        let enforced_units = delivered_units.saturating_sub(self.grace_units);
        let enforced_amount_due_msat = self.amount_due_msat(enforced_units);
        let allow_routing =
            delivered_units <= grace_limit_units || paid_msat >= enforced_amount_due_msat;
        let unpaid_msat = amount_due_msat.saturating_sub(paid_msat);
        let state = if delivered_units <= self.free_probe_units {
            StreamingRouteAccessState::FreeProbe
        } else if paid_msat >= amount_due_msat {
            StreamingRouteAccessState::Paid
        } else if allow_routing {
            StreamingRouteAccessState::Grace
        } else {
            StreamingRouteAccessState::Suspended
        };

        StreamingRouteDecision {
            state,
            allow_routing,
            delivered_units,
            paid_msat,
            amount_due_msat,
            enforced_amount_due_msat,
            unpaid_msat,
            free_probe_remaining_units,
            grace_remaining_units,
        }
    }
}

fn default_streaming_route_payment_protocol_version() -> u16 {
    STREAMING_ROUTE_PAYMENT_PROTOCOL_VERSION
}

#[cfg(feature = "spilman")]
impl From<upstream::Payment> for CashuSpilmanPayment {
    fn from(payment: upstream::Payment) -> Self {
        Self {
            channel_id: payment.channel_id,
            balance: payment.balance,
            signature: payment.signature,
            params: payment.params,
            funding_proofs: payment
                .funding_proofs
                .map(|proofs| serde_json::to_value(proofs).expect("serialize funding proofs")),
        }
    }
}

#[cfg(feature = "spilman")]
impl TryFrom<CashuSpilmanPayment> for upstream::Payment {
    type Error = String;

    fn try_from(payment: CashuSpilmanPayment) -> Result<Self, Self::Error> {
        Ok(Self {
            channel_id: payment.channel_id,
            balance: payment.balance,
            signature: payment.signature,
            params: payment.params,
            funding_proofs: payment
                .funding_proofs
                .map(serde_json::from_value)
                .transpose()
                .map_err(|error| format!("invalid Spilman funding proofs: {error}"))?,
        })
    }
}

/// Re-exports of the pinned implementation crate for callers that need the
/// channel protocol while this crate grows higher-level route-payment APIs.
pub mod upstream {
    pub use cdk_spilman::{
        BalanceUpdateMessage, BridgeError, BridgeErrorResponse, ChannelFunding, ChannelId,
        ChannelParameters, ChannelPolicy, ChannelState, ClientChannelInfo, ClientChannelState,
        ClientPaymentState, ClientStorage, CloseData, CloseError, ClosePreparationError,
        CloseSuccess, ClosingData, EstablishedChannel, FundChannelResult, KeysetInfo,
        MemoryClientStorage, OpenChannelResult, Payment, PaymentProof, PaymentSuccess,
        PaymentValidationResult, PreparedClose, SpilmanBridge, SpilmanClientBridge,
        SpilmanClientHost, SpilmanClientNetworking, SpilmanHost, SpilmanNetworking, UnblindResult,
    };

    #[cfg(feature = "spilman-configurable-host")]
    pub use cdk_spilman::configurable_host;

    #[cfg(feature = "spilman-axum")]
    pub use cdk_spilman::axum;
}

#[cfg(test)]
mod tests {
    use super::{
        CashuSpilmanPayment, StreamingRouteAccessState, StreamingRouteBalanceUpdate,
        StreamingRouteChannelOpen, StreamingRouteCooperativeCloseAck, StreamingRouteMeter,
        StreamingRoutePaymentEnvelope, StreamingRoutePaymentPayload, StreamingRoutePolicy,
        CASHU_SPILMAN_CHANNELS_REV, STREAMING_ROUTE_PAYMENT_PROTOCOL_VERSION,
    };

    #[test]
    fn route_policy_charges_only_after_free_probe() {
        let policy = StreamingRoutePolicy {
            meter: StreamingRouteMeter::Bytes,
            price_msat: 25,
            per_units: 10,
            max_channel_capacity_sat: 100,
            channel_expiry_secs: 600,
            free_probe_units: 100,
            grace_units: 20,
        };

        assert_eq!(policy.amount_due_msat(100), 0);
        assert_eq!(policy.amount_due_msat(101), 3);
        assert_eq!(policy.amount_due_msat(130), 75);
        assert!(policy.is_balance_sufficient(119, 0));
        assert!(policy.is_balance_sufficient(130, 25));
        assert!(!policy.is_balance_sufficient(130, 24));
    }

    #[test]
    fn route_policy_reports_free_paid_grace_and_suspended_states() {
        let policy = StreamingRoutePolicy {
            meter: StreamingRouteMeter::Bytes,
            price_msat: 25,
            per_units: 10,
            max_channel_capacity_sat: 100,
            channel_expiry_secs: 600,
            free_probe_units: 100,
            grace_units: 20,
        };

        let free = policy.routing_decision(100, 0);
        assert_eq!(free.state, StreamingRouteAccessState::FreeProbe);
        assert!(free.allow_routing);
        assert_eq!(free.amount_due_msat, 0);

        let paid = policy.routing_decision(130, 75);
        assert_eq!(paid.state, StreamingRouteAccessState::Paid);
        assert!(paid.allow_routing);
        assert_eq!(paid.unpaid_msat, 0);

        let grace = policy.routing_decision(130, 25);
        assert_eq!(grace.state, StreamingRouteAccessState::Grace);
        assert!(grace.allow_routing);
        assert_eq!(grace.amount_due_msat, 75);
        assert_eq!(grace.enforced_amount_due_msat, 25);
        assert_eq!(grace.unpaid_msat, 50);

        let suspended = policy.routing_decision(130, 24);
        assert_eq!(suspended.state, StreamingRouteAccessState::Suspended);
        assert!(!suspended.allow_routing);
        assert_eq!(suspended.unpaid_msat, 51);
    }

    #[test]
    fn route_payment_envelope_round_trips_channel_open() {
        let payment = CashuSpilmanPayment {
            channel_id: "channel-1".to_string(),
            balance: 5,
            signature: "sig-5".to_string(),
            params: Some(serde_json::json!({"receiver":"receiver-pubkey"})),
            funding_proofs: Some(serde_json::json!([{"amount":8,"secret":"proof"}])),
        };
        assert!(payment.has_funding());

        let envelope = StreamingRoutePaymentEnvelope::new(
            "internet-exit",
            "lease-1",
            "npub1buyer",
            "npub1seller",
            123,
            StreamingRoutePaymentPayload::ChannelOpen(StreamingRouteChannelOpen {
                mint_url: "https://mint.example".to_string(),
                unit: "sat".to_string(),
                capacity: 100,
                expires_unix: 723,
                receiver_pubkey_hex: "receiver-pubkey".to_string(),
                paid_msat: 5_000,
                payment,
            }),
        );

        let encoded = serde_json::to_string(&envelope).expect("serialize envelope");
        assert!(encoded.contains(r#""type":"channel_open""#));
        assert!(encoded.contains(r#""version":1"#));

        let decoded: StreamingRoutePaymentEnvelope =
            serde_json::from_str(&encoded).expect("decode envelope");
        assert_eq!(decoded.version, STREAMING_ROUTE_PAYMENT_PROTOCOL_VERSION);
        assert_eq!(decoded.channel_id(), "channel-1");
        match decoded.payload {
            StreamingRoutePaymentPayload::ChannelOpen(open) => {
                assert_eq!(open.mint_url, "https://mint.example");
                assert_eq!(open.unit, "sat");
                assert_eq!(open.paid_msat, 5_000);
                assert!(open.payment.has_funding());
            }
            other => panic!("unexpected payload: {other:?}"),
        }
    }

    #[test]
    fn route_payment_payloads_expose_channel_id() {
        let update = StreamingRoutePaymentPayload::BalanceUpdate(StreamingRouteBalanceUpdate {
            delivered_units: 130,
            amount_due_msat: 75,
            paid_msat: 100,
            payment: CashuSpilmanPayment {
                channel_id: "channel-2".to_string(),
                balance: 1,
                signature: "sig-1".to_string(),
                params: None,
                funding_proofs: None,
            },
        });
        let ack =
            StreamingRoutePaymentPayload::CooperativeCloseAck(StreamingRouteCooperativeCloseAck {
                channel_id: "channel-3".to_string(),
                accepted_balance: 2,
                accepted_paid_msat: 2_000,
                closed_at_unix: 456,
                receipt: Some(serde_json::json!({"ok":true})),
            });

        assert_eq!(update.channel_id(), "channel-2");
        assert_eq!(ack.channel_id(), "channel-3");
    }

    #[cfg(feature = "spilman")]
    #[test]
    fn cashu_spilman_payment_converts_upstream_payment() {
        let upstream =
            super::upstream::Payment::new("channel-1".to_string(), 7, "sig-7".to_string());

        let payment = CashuSpilmanPayment::from(upstream);
        assert_eq!(payment.channel_id, "channel-1");
        assert_eq!(payment.balance, 7);
        assert!(!payment.has_funding());

        let restored = super::upstream::Payment::try_from(payment).expect("restore payment");
        assert_eq!(restored.channel_id, "channel-1");
        assert_eq!(restored.balance, 7);
        assert_eq!(restored.signature, "sig-7");
        assert!(restored.params.is_none());
        assert!(restored.funding_proofs.is_none());
    }

    #[test]
    fn spilman_upstream_base_is_recorded() {
        assert_eq!(
            CASHU_SPILMAN_CHANNELS_REV,
            "bafc38f220e46289bebf157014ab1129c7deac63"
        );
    }
}
