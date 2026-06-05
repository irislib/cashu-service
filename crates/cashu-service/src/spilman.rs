//! Experimental Cashu Spilman channel support for streaming paid routes.
//!
//! The upstream protocol and APIs are early-alpha. This module deliberately
//! exposes route-payment concepts and selected upstream primitives so consumers
//! can depend on `cashu-service` rather than coupling directly to the
//! implementation crate.

use serde::{Deserialize, Serialize};

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
        if delivered_units <= self.free_probe_units.saturating_add(self.grace_units) {
            return true;
        }

        let enforced_units = delivered_units.saturating_sub(self.grace_units);
        paid_msat >= self.amount_due_msat(enforced_units)
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
    use super::{StreamingRouteMeter, StreamingRoutePolicy, CASHU_SPILMAN_CHANNELS_REV};

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
    fn spilman_upstream_base_is_recorded() {
        assert_eq!(
            CASHU_SPILMAN_CHANNELS_REV,
            "bafc38f220e46289bebf157014ab1129c7deac63"
        );
    }
}
