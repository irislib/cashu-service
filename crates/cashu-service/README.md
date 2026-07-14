# cashu-service

Reusable Cashu helper and wallet primitives for paid connectivity services.

This crate provides the shared plumbing used by applications that need
Cashu-backed payment flows without duplicating wallet and process-management
logic in each binary.

## Features

- optional wallet support behind the `wallet` feature
- validated credit-account snapshot persistence behind the `credit-store` feature
- exact authorized-accounting to CDK route binding behind the `credit-settlement` feature
- isolated real-CDK/fake-Lightning integration scenarios behind the `simulation` feature
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
paid destination quote and verifies that exact quote was issued. Its saga is
persisted in the wallet database, so retry the same request and `transfer_id`
after an interruption. Saga locking is process-local: run exactly one wallet
writer process for each wallet data directory.

This conversion leaves proofs in the caller's destination-mint wallet; it is not
evidence that a counterparty received payment. A payout application must durably
journal `settlement_id -> exact bearer token -> authenticated receiver ACK` and
resume that record after a crash instead of creating a second token.

This API does not select or approve mints and must not accept a seller-provided
mint URL as trusted input. Buyer-mode selection is a caller concern: automatic
selection may invoke it only in Auto mode, never in Off or Manual mode.

`credit-settlement` binds an already authorized accounting settlement to one
exact CDK route. It neither chooses trust/payment policy nor proves that an
authenticated counterparty received the resulting bearer token.
