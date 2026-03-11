# GSTD Cross-Chain Bridge Node

Decentralized cross-chain bridge validator for GSTD tokens.

**TON ↔ Solana ↔ XRPL**

## Architecture

```
┌─────────────────────────────────────────────────────┐
│                   GSTD Bridge Node                  │
│                                                     │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐          │
│  │TON Watch │  │SOL Watch │  │XRP Watch │ Monitors  │
│  └────┬─────┘  └────┬─────┘  └────┬─────┘          │
│       └──────────────┼──────────────┘               │
│                      ▼                              │
│            ┌─────────────────┐                      │
│            │ Consensus Engine │◄──── P2P Gossipsub  │
│            │   (67% Quorum)  │                      │
│            └────────┬────────┘                      │
│                     ▼                               │
│            ┌─────────────────┐                      │
│            │   MPC Signer    │ Threshold Signatures │
│            └────────┬────────┘                      │
│                     ▼                               │
│            ┌─────────────────┐                      │
│            │  Vault Manager  │ Lock-and-Unlock      │
│            └────────┬────────┘                      │
│                     ▼                               │
│            ┌─────────────────┐                      │
│            │   RPC Server    │ → Frontend / Node OS │
│            └─────────────────┘                      │
└─────────────────────────────────────────────────────┘
```

## Quick Start

```bash
# Generate default config
cargo run -- --init

# Start the bridge node
cargo run

# With custom config
cargo run -- --config my-bridge.toml

# With debug logging
RUST_LOG=debug cargo run
```

## GSTD Addresses

| Chain  | Vault Address |
|--------|---------------|
| TON    | `EQDv6cYW9nNiKjN3Nwl8D6ABjUiH1gYfWVGZhfP7-9tZskTO` |
| Solana | `AzN7uPhQZgThxsRvhNGHPUPRjdEjScTbqQdf5gt6Fqby` |
| XRPL   | `ryHSvxUqpcTjoESHbCkMJoqzenjFgPQSf` |

## Bridge Protocol

### Lock-and-Unlock Model

1. User sends GSTD to vault on **source chain** with memo: `bridge:<target_chain>:<recipient>`
2. Chain monitor detects the deposit
3. Deposit is proposed to P2P network
4. Validators vote (67% quorum required)
5. MPC threshold signature is generated
6. GSTD is unlocked from vault on **target chain**

### Memo Format

```
bridge:solana:AzN7uPhQZg...     # TON → Solana
bridge:ton:EQDv6cYW9n...        # Solana → TON
bridge:xrpl:ryHSvxUqpc...       # Any → XRPL
```

## Integration with Node OS

The bridge runs as a sidecar process alongside GSTD Node OS. The Node OS gateway proxies bridge RPC at `/api/bridge/*`.

### RPC Endpoints

- `GET /api/bridge/status` — Bridge node status
- `GET /api/bridge/transfers` — Recent bridge transfers
- `GET /api/bridge/transfer/:id` — Single transfer details
- `GET /api/bridge/vaults` — Vault balances per chain

## Module Structure

```
src/
├── main.rs           # Main event loop
├── config.rs         # Configuration (TOML)
├── p2p/
│   ├── mod.rs        # libp2p Swarm setup
│   ├── gossip.rs     # Gossipsub handler
│   └── discovery.rs  # Kademlia DHT
├── chains/
│   ├── mod.rs        # ChainMonitor trait
│   ├── ton.rs        # TON watcher
│   ├── solana.rs     # Solana watcher
│   └── xrpl.rs       # XRPL watcher
├── consensus/
│   ├── mod.rs        # Voting engine (67% quorum)
│   └── state.rs      # Shared state table
├── mpc/
│   ├── mod.rs        # Threshold signatures
│   └── keygen.rs     # Key generation
├── bridge/
│   ├── mod.rs        # Message types
│   └── vault.rs      # Lock/Unlock vault
└── rpc/
    └── mod.rs        # HTTP RPC server
```

## License

MIT
