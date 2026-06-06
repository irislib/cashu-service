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

/// Cashu denomination used by the Spilman channel balance.
///
/// Route accounting is denominated in millisats, while the underlying Cashu
/// channel may be funded in whole sats. Whole-sat channels round required
/// balances up so a signed update never underpays a metered route.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamingRouteCashuUnit {
    Sat,
    Msat,
}

impl StreamingRouteCashuUnit {
    pub fn parse(unit: &str) -> Result<Self, String> {
        match unit.trim().to_ascii_lowercase().as_str() {
            "" | "sat" | "sats" => Ok(Self::Sat),
            "msat" | "msats" => Ok(Self::Msat),
            other => Err(format!("unsupported Cashu channel unit '{other}'")),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sat => "sat",
            Self::Msat => "msat",
        }
    }

    pub fn balance_to_msat(self, balance: u64) -> u64 {
        match self {
            Self::Sat => balance.saturating_mul(1_000),
            Self::Msat => balance,
        }
    }

    pub fn balance_from_msat(self, paid_msat: u64) -> u64 {
        match self {
            Self::Sat => paid_msat.div_ceil(1_000),
            Self::Msat => paid_msat,
        }
    }

    pub fn capacity_from_sat(self, capacity_sat: u64) -> u64 {
        match self {
            Self::Sat => capacity_sat,
            Self::Msat => capacity_sat.saturating_mul(1_000),
        }
    }

    pub fn capacity_to_sat(self, capacity: u64) -> u64 {
        match self {
            Self::Sat => capacity,
            Self::Msat => capacity.div_ceil(1_000),
        }
    }
}

pub fn streaming_route_cashu_balance_msat(unit: &str, balance: u64) -> Result<u64, String> {
    Ok(StreamingRouteCashuUnit::parse(unit)?.balance_to_msat(balance))
}

pub fn streaming_route_cashu_balance_for_msat(unit: &str, paid_msat: u64) -> Result<u64, String> {
    Ok(StreamingRouteCashuUnit::parse(unit)?.balance_from_msat(paid_msat))
}

pub fn streaming_route_cashu_capacity_for_sat(
    unit: &str,
    capacity_sat: u64,
) -> Result<u64, String> {
    Ok(StreamingRouteCashuUnit::parse(unit)?.capacity_from_sat(capacity_sat))
}

pub fn streaming_route_cashu_capacity_sat(unit: &str, capacity: u64) -> Result<u64, String> {
    Ok(StreamingRouteCashuUnit::parse(unit)?.capacity_to_sat(capacity))
}

pub fn streaming_route_cashu_capacity_msat(unit: &str, capacity: u64) -> Result<u64, String> {
    Ok(StreamingRouteCashuUnit::parse(unit)?.balance_to_msat(capacity))
}

/// Kind of signed Spilman payment a buyer needs for route streaming.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamingRouteCashuPaymentKind {
    ChannelOpen,
    BalanceUpdate,
    CooperativeClose,
}

impl StreamingRouteCashuPaymentKind {
    pub fn include_funding(self) -> bool {
        matches!(self, Self::ChannelOpen)
    }
}

/// Request for creating a signed Cashu-Spilman payment for a paid route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamingRouteCashuPaymentRequest {
    pub kind: StreamingRouteCashuPaymentKind,
    pub channel_id: String,
    #[serde(default = "default_streaming_route_cashu_unit")]
    pub unit: String,
    pub paid_msat: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub previous_paid_msat: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub capacity_sat: u64,
}

/// Signed payment plus the route-accounting values that produced it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamingRouteCashuPaymentResult {
    pub payment: CashuSpilmanPayment,
    pub unit: String,
    pub balance: u64,
    pub paid_msat: u64,
    pub include_funding: bool,
}

/// Normalized result of checking a route-payment claim against a signed
/// Cashu-Spilman payment snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamingRouteCashuPaymentClaimValidation {
    pub channel_id: String,
    pub unit: String,
    pub balance: u64,
    pub paid_msat: u64,
    pub capacity_msat: u64,
    pub has_funding: bool,
}

/// Request for a fixed Cashu-token lease payment.
///
/// This is a fallback/dev mode for callers that cannot open a streaming
/// Spilman channel yet. The token is opaque to this crate; higher-level code
/// should redeem or verify it with the configured Cashu wallet before treating
/// the lease as settled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamingRouteCashuTokenLeaseRequest {
    pub channel_id: String,
    pub mint_url: String,
    #[serde(default = "default_streaming_route_cashu_unit")]
    pub unit: String,
    /// Token amount in `unit`.
    pub amount: u64,
    /// Optional route-credit amount. Defaults to the token amount converted to
    /// millisats and must not exceed it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paid_msat: Option<u64>,
    pub expires_unix: u64,
    pub token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamingRouteCashuTokenLease {
    pub channel_id: String,
    pub mint_url: String,
    #[serde(default = "default_streaming_route_cashu_unit")]
    pub unit: String,
    pub amount: u64,
    pub paid_msat: u64,
    pub expires_unix: u64,
    pub token: String,
}

/// Small signer facade implemented by the local upstream Spilman client bridge.
///
/// Tests and app code can depend on this trait without coupling storage,
/// networking, or key management to `cashu-service`.
pub trait CashuSpilmanPaymentSigner {
    fn sign_cashu_spilman_payment(
        &self,
        channel_id: &str,
        balance: u64,
        include_funding: bool,
    ) -> Result<CashuSpilmanPayment, String>;

    fn sign_cashu_spilman_close(
        &self,
        channel_id: &str,
        final_balance: u64,
    ) -> Result<CashuSpilmanPayment, String> {
        self.sign_cashu_spilman_payment(channel_id, final_balance, false)
    }
}

pub fn create_streaming_route_cashu_payment<S: CashuSpilmanPaymentSigner>(
    signer: &S,
    request: StreamingRouteCashuPaymentRequest,
) -> Result<StreamingRouteCashuPaymentResult, String> {
    let channel_id = request.channel_id.trim();
    if channel_id.is_empty() {
        return Err("missing Cashu Spilman channel id".to_string());
    }
    let unit = StreamingRouteCashuUnit::parse(&request.unit)?;
    if request.paid_msat < request.previous_paid_msat {
        return Err(format!(
            "paid route payment amount regressed: {} msat < {} msat",
            request.paid_msat, request.previous_paid_msat
        ));
    }
    let capacity_msat = request.capacity_sat.saturating_mul(1_000);
    if capacity_msat > 0 && request.paid_msat > capacity_msat {
        return Err(format!(
            "paid route payment {} msat exceeds channel capacity {} msat",
            request.paid_msat, capacity_msat
        ));
    }

    let balance = unit.balance_from_msat(request.paid_msat);
    let include_funding = request.kind.include_funding();
    let payment = if request.kind == StreamingRouteCashuPaymentKind::CooperativeClose {
        signer.sign_cashu_spilman_close(channel_id, balance)?
    } else {
        signer.sign_cashu_spilman_payment(channel_id, balance, include_funding)?
    };
    if payment.channel_id.trim() != channel_id {
        return Err(format!(
            "signed Cashu Spilman payment channel {} does not match requested channel {}",
            payment.channel_id, channel_id
        ));
    }
    if payment.balance != balance {
        return Err(format!(
            "signed Cashu Spilman payment balance {} does not match requested balance {}",
            payment.balance, balance
        ));
    }
    if include_funding && !payment.has_funding() {
        return Err("opening Cashu Spilman payment is missing funding data".to_string());
    }

    Ok(StreamingRouteCashuPaymentResult {
        payment,
        unit: unit.as_str().to_string(),
        balance,
        paid_msat: unit.balance_to_msat(balance),
        include_funding,
    })
}

/// Verify transport-neutral route accounting fields against the signed Cashu
/// Spilman payment snapshot.
///
/// This does not replace upstream cryptographic validation by the receiver-side
/// Spilman bridge. It catches application-layer mismatches before a service
/// trusts `paid_msat`, such as a balance update claiming more route credit than
/// the included channel balance can represent.
pub fn validate_streaming_route_cashu_payment_claim(
    payment: &CashuSpilmanPayment,
    expected_channel_id: &str,
    unit: &str,
    claimed_paid_msat: u64,
    capacity_sat: u64,
    require_funding: bool,
) -> Result<StreamingRouteCashuPaymentClaimValidation, String> {
    let expected_channel_id = expected_channel_id.trim();
    if expected_channel_id.is_empty() {
        return Err("missing Cashu Spilman channel id".to_string());
    }
    let channel_id = payment.channel_id.trim();
    if channel_id != expected_channel_id {
        return Err(format!(
            "Cashu Spilman payment channel {channel_id} does not match expected channel {expected_channel_id}"
        ));
    }
    if payment.signature.trim().is_empty() {
        return Err("Cashu Spilman payment signature is empty".to_string());
    }
    if require_funding && !payment.has_funding() {
        return Err("opening Cashu Spilman payment is missing funding data".to_string());
    }

    let unit = StreamingRouteCashuUnit::parse(unit)?;
    let paid_msat = unit.balance_to_msat(payment.balance);
    if paid_msat != claimed_paid_msat {
        return Err(format!(
            "paid route payment claim {claimed_paid_msat} msat does not match Cashu Spilman balance {} {} ({} msat)",
            payment.balance,
            unit.as_str(),
            paid_msat
        ));
    }
    let capacity_msat = capacity_sat.saturating_mul(1_000);
    if capacity_msat > 0 && paid_msat > capacity_msat {
        return Err(format!(
            "paid route payment {paid_msat} msat exceeds channel capacity {capacity_msat} msat"
        ));
    }

    Ok(StreamingRouteCashuPaymentClaimValidation {
        channel_id: channel_id.to_string(),
        unit: unit.as_str().to_string(),
        balance: payment.balance,
        paid_msat,
        capacity_msat,
        has_funding: payment.has_funding(),
    })
}

pub fn create_streaming_route_cashu_token_lease(
    request: StreamingRouteCashuTokenLeaseRequest,
) -> Result<StreamingRouteCashuTokenLease, String> {
    let channel_id = request.channel_id.trim();
    if channel_id.is_empty() {
        return Err("missing Cashu token lease id".to_string());
    }
    let mint_url = request.mint_url.trim();
    if mint_url.is_empty() {
        return Err("missing Cashu token mint URL".to_string());
    }
    let token = request.token.trim();
    if token.is_empty() {
        return Err("missing Cashu token".to_string());
    }
    let unit = StreamingRouteCashuUnit::parse(&request.unit)?;
    let max_paid_msat = unit.balance_to_msat(request.amount);
    let paid_msat = request.paid_msat.unwrap_or(max_paid_msat);
    if paid_msat > max_paid_msat {
        return Err(format!(
            "paid route token lease credit {} msat exceeds token amount {} msat",
            paid_msat, max_paid_msat
        ));
    }

    Ok(StreamingRouteCashuTokenLease {
        channel_id: channel_id.to_string(),
        mint_url: mint_url.to_string(),
        unit: unit.as_str().to_string(),
        amount: request.amount,
        paid_msat,
        expires_unix: request.expires_unix,
        token: token.to_string(),
    })
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
    /// Fixed Cashu token payment for a prepaid lease. This is a fallback/dev
    /// path; streaming Spilman channels remain the default for metered usage.
    CashuTokenLease(StreamingRouteCashuTokenLease),
}

impl StreamingRoutePaymentPayload {
    pub fn channel_id(&self) -> &str {
        match self {
            Self::ChannelOpen(open) => &open.payment.channel_id,
            Self::BalanceUpdate(update) => &update.payment.channel_id,
            Self::CooperativeClose(close) => &close.payment.channel_id,
            Self::CooperativeCloseAck(ack) => &ack.channel_id,
            Self::CashuTokenLease(lease) => &lease.channel_id,
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

fn default_streaming_route_cashu_unit() -> String {
    StreamingRouteCashuUnit::Sat.as_str().to_string()
}

fn is_zero(value: &u64) -> bool {
    *value == 0
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

#[cfg(feature = "spilman")]
impl<H, N> CashuSpilmanPaymentSigner for upstream::SpilmanClientBridge<H, N>
where
    H: upstream::SpilmanClientHost,
    N: upstream::SpilmanClientNetworking,
{
    fn sign_cashu_spilman_payment(
        &self,
        channel_id: &str,
        balance: u64,
        include_funding: bool,
    ) -> Result<CashuSpilmanPayment, String> {
        let payment = if include_funding {
            self.create_payment_with_funding(channel_id, balance)?
        } else {
            self.create_payment(channel_id, balance)?
        };
        Ok(payment.into())
    }

    fn sign_cashu_spilman_close(
        &self,
        channel_id: &str,
        final_balance: u64,
    ) -> Result<CashuSpilmanPayment, String> {
        self.create_cooperative_close_request(channel_id, final_balance)
            .map(Into::into)
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
        create_streaming_route_cashu_payment, create_streaming_route_cashu_token_lease,
        validate_streaming_route_cashu_payment_claim, CashuSpilmanPayment,
        CashuSpilmanPaymentSigner, StreamingRouteAccessState, StreamingRouteBalanceUpdate,
        StreamingRouteCashuPaymentKind, StreamingRouteCashuPaymentRequest,
        StreamingRouteCashuTokenLeaseRequest, StreamingRouteChannelOpen,
        StreamingRouteCooperativeCloseAck, StreamingRouteMeter, StreamingRoutePaymentEnvelope,
        StreamingRoutePaymentPayload, StreamingRoutePolicy, CASHU_SPILMAN_CHANNELS_REV,
        STREAMING_ROUTE_PAYMENT_PROTOCOL_VERSION,
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
        let token = StreamingRoutePaymentPayload::CashuTokenLease(
            create_streaming_route_cashu_token_lease(StreamingRouteCashuTokenLeaseRequest {
                channel_id: "token-lease-4".to_string(),
                mint_url: "https://mint.example".to_string(),
                unit: "sat".to_string(),
                amount: 2,
                paid_msat: Some(1_500),
                expires_unix: 789,
                token: "cashuAdevtoken".to_string(),
            })
            .expect("token lease"),
        );

        assert_eq!(update.channel_id(), "channel-2");
        assert_eq!(ack.channel_id(), "channel-3");
        assert_eq!(token.channel_id(), "token-lease-4");
    }

    #[test]
    fn route_token_lease_payment_round_trips_and_caps_credit() {
        let token_lease =
            create_streaming_route_cashu_token_lease(StreamingRouteCashuTokenLeaseRequest {
                channel_id: "token-lease-1".to_string(),
                mint_url: "https://mint.example".to_string(),
                unit: "sat".to_string(),
                amount: 2,
                paid_msat: Some(1_500),
                expires_unix: 999,
                token: "cashuBtoken".to_string(),
            })
            .expect("create token lease");
        assert_eq!(token_lease.paid_msat, 1_500);

        let envelope = StreamingRoutePaymentEnvelope::new(
            "internet-exit",
            "lease-1",
            "npub1buyer",
            "npub1seller",
            123,
            StreamingRoutePaymentPayload::CashuTokenLease(token_lease),
        );

        let encoded = serde_json::to_string(&envelope).expect("serialize envelope");
        assert!(encoded.contains(r#""type":"cashu_token_lease""#));
        let decoded: StreamingRoutePaymentEnvelope =
            serde_json::from_str(&encoded).expect("decode envelope");
        assert_eq!(decoded.channel_id(), "token-lease-1");
        match decoded.payload {
            StreamingRoutePaymentPayload::CashuTokenLease(lease) => {
                assert_eq!(lease.unit, "sat");
                assert_eq!(lease.amount, 2);
                assert_eq!(lease.paid_msat, 1_500);
                assert_eq!(lease.token, "cashuBtoken");
            }
            other => panic!("unexpected payload: {other:?}"),
        }

        let too_much =
            create_streaming_route_cashu_token_lease(StreamingRouteCashuTokenLeaseRequest {
                channel_id: "token-lease-2".to_string(),
                mint_url: "https://mint.example".to_string(),
                unit: "sat".to_string(),
                amount: 2,
                paid_msat: Some(2_001),
                expires_unix: 999,
                token: "cashuBtoken".to_string(),
            })
            .expect_err("over-credit should fail");
        assert!(too_much.contains("exceeds token amount"));
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
    fn route_cashu_unit_conversions_round_up_sat_balances() {
        assert_eq!(
            super::streaming_route_cashu_balance_for_msat("sat", 1).unwrap(),
            1
        );
        assert_eq!(
            super::streaming_route_cashu_balance_for_msat("sat", 1_001).unwrap(),
            2
        );
        assert_eq!(
            super::streaming_route_cashu_balance_msat("sat", 2).unwrap(),
            2_000
        );
        assert_eq!(
            super::streaming_route_cashu_balance_for_msat("msat", 1_001).unwrap(),
            1_001
        );
        assert_eq!(
            super::streaming_route_cashu_capacity_for_sat("msat", 2).unwrap(),
            2_000
        );
        assert_eq!(
            super::streaming_route_cashu_capacity_sat("msat", 2_001).unwrap(),
            3
        );
    }

    #[test]
    fn route_cashu_payment_builder_signs_open_with_funding() {
        let signer = FakeSigner;
        let result = create_streaming_route_cashu_payment(
            &signer,
            StreamingRouteCashuPaymentRequest {
                kind: StreamingRouteCashuPaymentKind::ChannelOpen,
                channel_id: "channel-1".to_string(),
                unit: "sat".to_string(),
                paid_msat: 1,
                previous_paid_msat: 0,
                capacity_sat: 10,
            },
        )
        .expect("create payment");

        assert_eq!(result.balance, 1);
        assert_eq!(result.paid_msat, 1_000);
        assert!(result.include_funding);
        assert!(result.payment.has_funding());
        assert_eq!(result.payment.signature, "sig-channel-1-1-funding");
    }

    #[test]
    fn route_cashu_payment_builder_signs_updates_without_funding() {
        let signer = FakeSigner;
        let result = create_streaming_route_cashu_payment(
            &signer,
            StreamingRouteCashuPaymentRequest {
                kind: StreamingRouteCashuPaymentKind::BalanceUpdate,
                channel_id: "channel-1".to_string(),
                unit: "msat".to_string(),
                paid_msat: 1_500,
                previous_paid_msat: 1_000,
                capacity_sat: 2,
            },
        )
        .expect("create payment");

        assert_eq!(result.balance, 1_500);
        assert_eq!(result.paid_msat, 1_500);
        assert!(!result.include_funding);
        assert!(!result.payment.has_funding());
        assert_eq!(result.payment.signature, "sig-channel-1-1500-update");
    }

    #[test]
    fn route_cashu_payment_builder_rejects_regressions_and_over_capacity() {
        let signer = FakeSigner;
        let regression = create_streaming_route_cashu_payment(
            &signer,
            StreamingRouteCashuPaymentRequest {
                kind: StreamingRouteCashuPaymentKind::BalanceUpdate,
                channel_id: "channel-1".to_string(),
                unit: "sat".to_string(),
                paid_msat: 999,
                previous_paid_msat: 1_000,
                capacity_sat: 2,
            },
        )
        .expect_err("regression should fail");
        assert!(regression.contains("regressed"));

        let over_capacity = create_streaming_route_cashu_payment(
            &signer,
            StreamingRouteCashuPaymentRequest {
                kind: StreamingRouteCashuPaymentKind::BalanceUpdate,
                channel_id: "channel-1".to_string(),
                unit: "sat".to_string(),
                paid_msat: 2_001,
                previous_paid_msat: 0,
                capacity_sat: 2,
            },
        )
        .expect_err("over capacity should fail");
        assert!(over_capacity.contains("exceeds channel capacity"));
    }

    #[test]
    fn route_cashu_payment_claim_validation_matches_signed_balance() {
        let payment = CashuSpilmanPayment {
            channel_id: "channel-1".to_string(),
            balance: 2,
            signature: "sig-2".to_string(),
            params: Some(serde_json::json!({"ok": true})),
            funding_proofs: Some(serde_json::json!([])),
        };
        let validated = validate_streaming_route_cashu_payment_claim(
            &payment,
            "channel-1",
            "sat",
            2_000,
            2,
            true,
        )
        .expect("valid claim");

        assert_eq!(validated.channel_id, "channel-1");
        assert_eq!(validated.unit, "sat");
        assert_eq!(validated.balance, 2);
        assert_eq!(validated.paid_msat, 2_000);
        assert_eq!(validated.capacity_msat, 2_000);
        assert!(validated.has_funding);
    }

    #[test]
    fn route_cashu_payment_claim_validation_rejects_mismatches() {
        let payment = CashuSpilmanPayment {
            channel_id: "channel-1".to_string(),
            balance: 1,
            signature: "sig-1".to_string(),
            params: None,
            funding_proofs: None,
        };

        let claimed_too_much = validate_streaming_route_cashu_payment_claim(
            &payment,
            "channel-1",
            "sat",
            2_000,
            2,
            false,
        )
        .expect_err("over-claimed payment should fail");
        assert!(claimed_too_much.contains("does not match"));

        let missing_funding = validate_streaming_route_cashu_payment_claim(
            &payment,
            "channel-1",
            "sat",
            1_000,
            2,
            true,
        )
        .expect_err("opening without funding should fail");
        assert!(missing_funding.contains("missing funding"));
    }

    #[test]
    fn spilman_upstream_base_is_recorded() {
        assert_eq!(
            CASHU_SPILMAN_CHANNELS_REV,
            "bafc38f220e46289bebf157014ab1129c7deac63"
        );
    }

    struct FakeSigner;

    impl CashuSpilmanPaymentSigner for FakeSigner {
        fn sign_cashu_spilman_payment(
            &self,
            channel_id: &str,
            balance: u64,
            include_funding: bool,
        ) -> Result<CashuSpilmanPayment, String> {
            Ok(CashuSpilmanPayment {
                channel_id: channel_id.to_string(),
                balance,
                signature: format!(
                    "sig-{channel_id}-{balance}-{}",
                    if include_funding { "funding" } else { "update" }
                ),
                params: include_funding.then(|| serde_json::json!({"ok": true})),
                funding_proofs: include_funding.then(|| serde_json::json!([])),
            })
        }
    }
}
