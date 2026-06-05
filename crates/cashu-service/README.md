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
