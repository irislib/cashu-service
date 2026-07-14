# Sat-Denominated Service Credit

`cashu-credit` records authenticated useful-service receipts in sats. The service provider keeps one
`CreditAccount` for a counterparty and locally selects the issuers, value classes, and exposure limits
it accepts. Transport authentication, proof verification, mint selection, and service metering stay
outside the crate.

The three value classes deliberately have different redemption authority:

- Peer credit is an unbacked relationship-local IOU. Per-issuer, aggregate, and offline caps bound
  exposure, and it can never authorize external cashout.
- Closed-loop deposits are backed but can only buy issuer service or circulate among peers that
  accept that issuer.
- Withdrawable deposits are reserve-backed and are the only value that can authorize a Cashu or
  Lightning payout.

Every receipt names an application-defined service and resource. Applications admit only verified
useful work: for example acknowledged nVPN traffic, a requested and verified pubsub event, or the
first accepted hash-valid Hashtree block. Raw forwarded bytes, retries, duplicates, spam, and
unsolicited traffic do not become debt merely by reaching the accounting crate.

Backing, conversions, consumption, and payout authorization use stable operation IDs. Replays are
idempotent; reuse with changed contents is rejected. External settlement reserves principal plus a
fee ceiling before wallet I/O. The caller completes it only after the counterparty has actually
received value.

Replay protection inside `CreditAccount` is account-local. Persistence that holds multiple accounts
must claim `backing/<issuer>/<deposit_id>` for exactly one account, atomically with the snapshot
revision CAS. Otherwise one verified proof, quote, or payment could back more than one account.
Snapshot bytes and that binding index are trusted local state; protect them together with an outer
MAC, signature, or equivalent integrity mechanism when storage or synchronization is untrusted.
Snapshot validation does not re-authenticate historical signatures, proofs, or payments.

## Why This Helps Poor Connectivity

For ordinary Cashu, the receiver usually wants to check proofs with the mint. In an intermittent
mesh, a provider can instead admit a small offline peer-credit receipt because both issuer-local and
aggregate exposure are bounded. Backed value still requires online proof verification. When
connectivity improves, peer credit can be replaced with value from an accepted issuer.

## Non-Goals For V1

- Global pathfinding across trustlines.
- Treating peer-issued Cashu as globally fungible sats.
- Defining application-specific useful-service meters.
- Accepting unknown third-party peer tokens by default.

Those can come later. The first useful version is bounded bilateral credit for verified useful service.
