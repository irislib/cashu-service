# Changelog

## cashu-service 0.4.5 - 2026-08-17

- Consolidate wallet proofs into the selected active keyset before opening a Spilman channel, preventing mixed-keyset funding failures after mint key rotation.

## cashu-service 0.4.4 - 2026-08-08

- Reject Unix FIFO and device Spilman state without blocking the process during validation.

## cashu-service 0.4.3 - 2026-08-07

- Reject symlink and non-file Spilman state, and replace private state atomically without losing its non-root owner.
- Serialize file-backed Spilman mutations across processes with a lifetime-held lock and nonblocking daemon APIs.

## cashu-service 0.4.2 - 2026-07-20

- Restore deterministic Spilman sender refunds from the settlement keyset and import them into the file-backed wallet after mint key rotation.
- Report settlement completion separately from the recovered amount, including zero-refund channels, and avoid repeat mint requests once recovery completes.

## cashu-service 0.4.0 - 2026-07-18

- Update Cashu/CDK, CDK SQLite, and `cdk-spilman` to the stable 0.17 line.
- Add an optional single-owner `CashuWalletService` runtime around CDK SQLite.
- Add exact secure-seed migration with missing-seed and mismatched-seed protection.
- Add startup melt, saga, and pending-quote recovery reporting.
- Preserve SQLite database, WAL, SHM, and journal files as one recovery family.
- Keep database-at-rest encryption out of scope; CDK SQLite remains the schema owner.
