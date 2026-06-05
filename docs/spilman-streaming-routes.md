# Cashu Spilman Streaming Routes

`cashu-service` can optionally expose the experimental
`SatsAndSports/cashu_spilman_channels` implementation behind the `spilman` feature. The dependency
is a sibling checkout at `../cashu_spilman_channels` so local protocol fixes can be made and tested
without copying the implementation into this repository.

The target paid-connectivity flow is:

1. A seller advertises a metered route policy and accepted mints.
2. A buyer opens a small, capped Cashu Spilman channel with a short expiry.
3. The seller allows a free probe budget for STUN, latency, loss, jitter, and a small route test.
4. The buyer streams signed balance updates as route usage grows.
5. The seller continues routing only while the latest signed balance covers measured usage plus a
   small grace budget.
6. Either side closes the channel; unused capacity returns to the buyer according to the channel
   refund path.

This avoids the worst fixed-lease failure mode for public exits: a seller should not receive a full
prepayment before proving that it can route traffic.

The upstream protocol is early-alpha and the local checkout records its upstream base revision. Keep
application-facing types in this repository so FIPS, Nostr VPN, and other consumers do not take a
direct dependency on unstable upstream APIs. If this becomes effectively ours, move only the Rust
core into a dedicated crate with preserved history/license instead of copying the whole multi-language
repository.
