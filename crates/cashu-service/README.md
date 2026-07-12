# cashu-service

Reusable Cashu helper and wallet primitives for paid connectivity services.

This crate provides the shared plumbing used by applications that need
Cashu-backed payment flows without duplicating wallet and process-management
logic in each binary.

## Features

- optional wallet support behind the `wallet` feature
- experimental Cashu Spilman channel support behind the `spilman` feature
- shared async helpers for invoking external payment workflows
- serde-friendly request and response types for service integration

## Cross-mint transfers

With the `wallet` feature, `transfer_between_mints` moves an exact sat amount
between two caller-selected Cashu mints over Lightning. The caller must supply:

- a stable `transfer_id` idempotency key
- approved source and destination mint URLs
- the exact destination amount and maximum total source-side fee

The transfer creates the destination quote first, validates its BOLT11 amount,
preflights the source melt and all wallet fees before payment, then issues the
paid destination quote and verifies the balance increase. Its saga is persisted
in the wallet database, so retry the same request and `transfer_id` after an
interruption.

This API does not select or approve mints and must not accept a seller-provided
mint URL as trusted input. Buyer-mode selection is a caller concern: automatic
selection may invoke it only in Auto mode, never in Off or Manual mode.
