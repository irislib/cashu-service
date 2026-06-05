# Sat-Denominated Peer Credit

This model treats peer-issued Cashu as a relationship-local IOU denominated in sats.

Example:

- Alice operates or controls a Cashu mint at `cashu+fips://npub1alice/local`.
- Bob trusts Alice up to `1_000 sat`.
- Alice can pay Bob with Alice-issued Cashu tokens until Bob's local accounting reaches that limit.
- At Bob's settlement threshold, Alice refills the line with something Bob accepts, such as tokens from
  `https://mint.example`, a Lightning payment, or a manual relationship-local settlement.

The unit is always sats. The issuer changes trust semantics, not denomination.

## Rules

- A credit line is bilateral: one debtor, one creditor, one debtor mint.
- A token from the debtor mint increases `outstanding_debt_sat`.
- The creditor enforces `credit_limit_sat` locally.
- The creditor can require settlement once `settlement_threshold_sat` is reached.
- Settlement reduces `outstanding_debt_sat`.
- Offline acceptance is an explicit smaller risk budget, not the main credit limit.

## Why This Helps Poor Connectivity

For ordinary Cashu, the receiver usually wants to check with the mint before accepting a token.
In a mesh or intermittently connected network, a peer may be able to reach a neighbor but not a
global mint or Lightning node.

With peer credit, Bob can accept a small Alice-issued token while offline because Bob's exposure is
bounded by the trustline. Later, when connectivity improves, Alice settles with an asset Bob accepts.

## Non-Goals For V1

- Global pathfinding across trustlines.
- Treating peer-issued Cashu as globally fungible sats.
- Proving per-packet delivery before payment.
- Accepting unknown third-party peer tokens by default.

Those can come later. The first useful version is bilateral credit for paid service leases.
