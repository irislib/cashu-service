# cashu-service

Reusable Cashu primitives for paid connectivity services.

This workspace currently has two layers:

- `cashu-service`: wallet/helper plumbing for sending, receiving, and settling Cashu tokens.
- `cashu-credit`: sat-denominated bilateral credit-line protocol types for peer-issued Cashu.

The credit model treats peer-issued Cashu as an IOU denominated in sats. A peer may accept another
peer's issued tokens up to a local trust limit, then require settlement in an accepted Cashu mint,
Lightning payment, or another explicitly configured method.

The intended first consumers are FIPS paid connectivity services and Nostr VPN exit-node leases,
but the crates avoid depending on either project.

`cashu-service` also has an experimental `spilman` feature backed by a sibling checkout at
`../cashu_spilman_channels`, based on `SatsAndSports/cashu_spilman_channels`. It is intended for
streaming paid routes where a buyer opens a small channel, probes route quality, and releases
incremental signed balances as traffic flows. The upstream API is early-alpha, so consumers should
depend on the local `cashu-service` facade rather than the upstream crate directly.

See [docs/peer-credit.md](docs/peer-credit.md) for the starting model.
