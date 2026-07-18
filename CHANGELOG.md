# Changelog

## cashu-service 0.4.0 - 2026-07-18

- Update Cashu/CDK, CDK SQLite, and `cdk-spilman` to the stable 0.17 line.
- Add an optional single-owner `CashuWalletService` runtime around CDK SQLite.
- Add exact secure-seed migration with missing-seed and mismatched-seed protection.
- Add startup melt, saga, and pending-quote recovery reporting.
- Preserve SQLite database, WAL, SHM, and journal files as one recovery family.
- Keep database-at-rest encryption out of scope; CDK SQLite remains the schema owner.
