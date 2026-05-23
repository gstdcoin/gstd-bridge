# GSTD Cross-Chain Bridge

> Trustless GSTD token transfers between TON, Solana, and XRPL.  
> Written in Rust. Runs as a lightweight validator node alongside GSTD Node OS.

---

## What It Does

The bridge lets you move GSTD tokens freely between chains — no custodians, no central relayers. A decentralized set of validators watches all three chains simultaneously. When a deposit is detected, validators reach consensus (67% quorum), aggregate their MPC threshold signatures, and release funds on the destination chain.

**Lock on source → Quorum consensus → MPC sign → Unlock on destination**

No single party can steal funds. No single party can stop a transfer.

---

## Architecture

```
┌─────────────────────────────────────────────────────┐
│                   GSTD Bridge Node                  │
│                                                     │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐          │
│  │TON Watch │  │SOL Watch │  │XRP Watch │           │
│  └────┬─────┘  └────┬─────┘  └────┬─────┘          │
│       └──────────────┼──────────────┘               │
│                      ▼                              │
│            ┌─────────────────┐                      │
│            │ Consensus Engine │◄── libp2p Gossipsub │
│            │   (67% quorum)  │                      │
│            └────────┬────────┘                      │
│                     ▼                               │
│            ┌─────────────────┐                      │
│            │   MPC Signer    │ threshold signatures  │
│            └────────┬────────┘                      │
│                     ▼                               │
│            ┌─────────────────┐                      │
│            │  Vault Manager  │ lock / unlock         │
│            └────────┬────────┘                      │
│                     ▼                               │
│            ┌─────────────────┐                      │
│            │   RPC Server    │ → Node OS / Frontend  │
│            └─────────────────┘                      │
└─────────────────────────────────────────────────────┘
```

### P2P Network

- **Discovery:** Kademlia DHT — validators find each other automatically
- **Messaging:** Gossipsub — proposals and votes propagate to all validators
- **No bootstrap server required** — validators connect via known peer multiaddrs (fetched from `app.gstdtoken.com/api/v1/nodes/peers`)

### Consensus

- **Threshold:** 67% of active validators must approve a transfer
- **Timeout:** 10 minutes — expired proposals are discarded
- **Double-spend protection:** Transfer IDs are globally unique (hash of tx_hash + chain + recipient)

### MPC Threshold Signatures

Private keys are never held by a single party. The signing key is split across validators using threshold cryptography (t-of-n). Even if `t-1` validators collude, they cannot sign anything.

---

## Vault Addresses

| Chain | Vault | Explorer |
|---|---|---|
| **TON** | `EQDv6cYW9nNiKjN3Nwl8D6ABjUiH1gYfWVGZhfP7-9tZskTO` | [tonscan.org](https://tonscan.org) |
| **Solana** | `AzN7uPhQZgThxsRvhNGHPUPRjdEjScTbqQdf5gt6Fqby` | [solscan.io](https://solscan.io) |
| **XRPL** | `ryHSvxUqpcTjoESHbCkMJoqzenjFgPQSf` | [xrpscan.com](https://xrpscan.com) |

---

## Bridge Protocol

### How to Bridge

1. Send GSTD to the **source chain vault** with memo:
   ```
   bridge:<target_chain>:<recipient_address>
   ```

2. Example — TON → Solana:
   ```
   bridge:solana:AzN7uPhQZgThxsRvhNGHPUPRjdEjScTbqQdf5gt6Fqby
   ```

3. Wait for 3 confirmations on source chain (~15-30 seconds on TON)

4. Validators detect deposit, reach quorum, release funds on destination

5. Total bridge time: **~2 minutes** (depends on validator count)

### Bridge Fees

- **0.1% flat fee** on bridged amount
- Fee is split among validators who signed the transaction
- Minimum fee: 1 GSTD (to cover gas on destination chain)

---

## Running a Bridge Validator

Running a bridge validator earns you a share of bridge fees.

### Prerequisites

- Rust 1.75+
- 1GB RAM, 10GB disk
- Stable internet connection (the bridge node must be reachable)
- Small balance of TON, SOL, and XRP for gas (outgoing transactions)

### Quick Start

```bash
# Install
git clone https://github.com/gstdcoin/gstd-bridge
cd gstd-bridge
cargo build --release

# Generate config
./target/release/gstd-bridge --init

# Edit bridge.toml — add your RPC endpoints and wallet keys
nano bridge.toml

# Start
./target/release/gstd-bridge

# Or with Docker
docker build -t gstd-bridge .
docker run -v $(pwd)/bridge.toml:/app/bridge.toml gstd-bridge
```

### Config (`bridge.toml`)

```toml
[node]
data_dir = "./data"
keypair_path = "./data/keypair"

[p2p]
listen_addr = "/ip4/0.0.0.0/tcp/9000"
bootstrap_peers = []  # Auto-fetched from app.gstdtoken.com/api/v1/nodes/peers

[chains.ton]
rpc_url = "https://toncenter.com/api/v2"
vault_address = "EQDv6cYW9nNiKjN3Nwl8D6ABjUiH1gYfWVGZhfP7-9tZskTO"
api_key = ""  # Optional toncenter API key

[chains.solana]
rpc_url = "https://api.mainnet-beta.solana.com"
vault_address = "AzN7uPhQZgThxsRvhNGHPUPRjdEjScTbqQdf5gt6Fqby"

[chains.xrpl]
rpc_url = "wss://xrplcluster.com"
vault_address = "ryHSvxUqpcTjoESHbCkMJoqzenjFgPQSf"

[consensus]
quorum_threshold = 0.67
transfer_timeout_secs = 600

[rpc]
listen_addr = "127.0.0.1:9001"
```

### Become a Validator

1. Run the bridge node
2. Register your node via the [GSTD Platform](https://app.gstdtoken.com)
3. Stake **10,000+ GSTD** (Provider tier) in the NaaS registry
4. The DAO votes to add you to the active validator set

With 3+ validators online, the bridge is live.

---

## RPC Endpoints

The bridge node exposes a local HTTP API:

| Endpoint | Method | Description |
|---|---|---|
| `/api/bridge/status` | GET | Node status, peer count, uptime |
| `/api/bridge/transfers` | GET | Recent bridge transfers |
| `/api/bridge/transfer/:id` | GET | Single transfer details + votes |
| `/api/bridge/vaults` | GET | Vault balances per chain |

---

## Module Structure

```
src/
├── main.rs             # Main event loop
├── config.rs           # Configuration (TOML)
├── p2p/
│   ├── mod.rs          # libp2p Swarm (Kademlia + Gossipsub)
│   ├── gossip.rs       # Gossipsub message handler
│   └── discovery.rs    # Kademlia DHT
├── chains/
│   ├── mod.rs          # ChainMonitor trait
│   ├── ton.rs          # TON watcher (toncenter API)
│   ├── solana.rs       # Solana watcher (RPC)
│   └── xrpl.rs         # XRPL watcher (WebSocket)
├── consensus/
│   ├── mod.rs          # Voting engine (67% quorum, timeout, dedup)
│   └── state.rs        # Shared transfer state table
├── mpc/
│   ├── mod.rs          # Threshold signature scheme
│   └── keygen.rs       # Distributed key generation
├── bridge/
│   ├── mod.rs          # BridgeMessage enum (Gossipsub payloads)
│   └── vault.rs        # Lock/Unlock vault accounting
└── rpc/
    └── mod.rs          # Axum HTTP server
```

---

## Security Model

| Threat | Mitigation |
|---|---|
| Single validator colludes | MPC threshold — needs 67% to sign |
| Validator goes offline | Transfer requeued after timeout, other validators continue |
| Double spend | Transfer ID is globally unique, consensus records prevent replay |
| Vault drain | On-chain contracts enforce lock logic; only valid MPC sig unlocks |
| Network partition | Gossipsub retries + Kademlia rerouting; quorum drops gracefully |

---

## Ecosystem

| Repo | Description |
|---|---|
| [gstdcoin/contracts](https://github.com/gstdcoin/contracts) | Smart contracts (TON vault, NaaS staking) |
| [gstdcoin/gstdbot](https://github.com/gstdcoin/gstdbot) | Node OS (bridge runs alongside it) |
| [gstdcoin/ai](https://github.com/gstdcoin/ai) | Platform dashboard |
| **gstdcoin/gstd-bridge** | **This repo — bridge validators** |

---

## License

MIT
