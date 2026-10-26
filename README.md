# GSTD bridge research

**Deferred. Not deployed. Not part of the production GSTD compute network.** No production bridge vaults or usable cross-chain transfer service exist. Do not send tokens expecting this code to bridge them.

The current Rust scaffold contains chain monitoring and experimental P2P coordination. It does not implement a chain-valid threshold-signing scheme or submit withdrawals. Withdrawal handlers fail explicitly instead of fabricating successful transaction hashes. Validator votes are not yet cryptographically bound to a registered validator key and authenticated gossip sender.

A production bridge would need a separate protocol design, authenticated consensus, real threshold key generation/signing, replay-safe settlement, independent audit and testnet acceptance. Passing Rust CI does not establish any of those properties. There is no operator installation or funding procedure for a production bridge.

The node installer no longer installs or launches bridge validators. Existing research files and operator keys have not been deleted.

## Use GSTD today

- [AI and compute platform](https://app.gstdtoken.com)
- [Compute node](https://github.com/gstdcoin/gstdbot)
- [GSTD payments on TON](https://app.gstdtoken.com/payments)
- [Release readiness](https://github.com/gstdcoin/ai/blob/main/docs/READINESS.md)

For source navigation see [CLAUDE.md](CLAUDE.md). The [historical design](docs/ARCHIVED_DESIGN.md) is archived for research and contains obsolete claims; it is not an operational specification.
