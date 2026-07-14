# AGENTS.md

- Keep crates transport- and application-neutral; FIPS, Nostr VPN, and Hashtree-specific glue belongs in those repos.
- Credit balances are denominated in sats. Peer-issued Cashu represents relationship-local credit, not globally trusted money.
- Prefer small, serializable protocol structs and deterministic accounting helpers over runtime policy hidden in services.
- Run `cargo test --workspace --all-features` before committing.
