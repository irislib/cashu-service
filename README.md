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

See [docs/peer-credit.md](docs/peer-credit.md) for the starting model.
