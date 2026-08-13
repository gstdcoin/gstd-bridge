# GSTD Cross-Chain Bridge

> Trustless GSTD token transfers between TON, Solana, and XRPL.  
> Written in Rust. Runs as a lightweight validator node alongside GSTD Node OS.

> **⚠ Status: deferred, not deployed.** No validator has ever run this in
> production — no MPC key shares exist, no Solana/XRPL vault wallets have
> been created, and CI has been red since 2026-05-26. Everything below
> describes the intended design, not a working system. The platform's
> `/bridge` page currently redirects elsewhere rather than exposing this.
> See `CLAUDE.md` for what's actually implemented vs. aspirational.
>
> **This is not just "not deployed yet" — three specific claims below do
> not hold in the current code and would need a real redesign, not a
> config change, before this could ever go live:**
> - **Consensus votes are unauthenticated.** `record_vote()` accepts any
>   `voter` string with no check it belongs to a connected validator, and
>   never verifies a signature. A single attacker can currently forge quorum.
> - **The "MPC threshold signature" isn't a threshold scheme.** Each
>   validator's "share" is an independently generated random key, not a
>   point on a shared secret polynomial — there is no secret to reconstruct.
>   "Aggregation" hashes shares together; the result is not a valid
>   signature on any of the three target chains.
> - **`execute_withdrawal()` is a no-op on all three chains.** It logs and
>   returns a fabricated transaction hash without ever submitting anything
>   on-chain, and the caller treats that hash as proof of a successful
>   transfer.

---

## What It's Designed To Do

*(Not yet true of the running code — see the status banner above.)* The intent is for the bridge to move GSTD tokens between chains with no custodian and no central relayer: a decentralized set of validators watches all three chains simultaneously, and when a deposit is detected, validators vote and, once quorum is reached, release funds on the destination chain.

**Lock on source → Quorum consensus → signed release → Unlock on destination**

That last step — a signed release that can't be forged by fewer than a threshold of validators — is exactly what's missing today: votes aren't authenticated, there's no real threshold signature, and no withdrawal actually gets submitted on-chain. Until those three things are rebuilt, a single validator (or attacker impersonating one) can currently sway the vote count, and no release happens regardless.

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

### MPC Threshold Signatures — not implemented

This is the intended design, not the current behavior. Each validator today
generates its own independent Ed25519 keypair — there is no shared secret
split across validators, no DKG ceremony, and no real threshold scheme.
Each validator's own signature can be independently verified (that part is
real Ed25519), but there is no way to combine independent validators'
signatures into one on-chain-valid signature yet, and `execute_withdrawal()`
correctly refuses to submit anything until that exists. See `mpc/mod.rs`
and `CLAUDE.md` for the exact gap.

---

## Vault Addresses

**None of these exist yet.** The TON address below is the GSTD jetton
*master contract*, not a custodial vault — do not send funds expecting
a bridge to process them. Solana and XRPL vault wallets haven't been
created at all. Real vault addresses will be published here once a
validator set is actually running.

| Chain | Vault | Explorer |
|---|---|---|
| **TON** | not yet deployed (jetton master `EQDv6cYW9nNiKjN3Nwl8D6ABjUiH1gYfWVGZhfP7-9tZskTO` is not a vault) | [tonscan.org](https://tonscan.org) |
| **Solana** | not yet deployed | [solscan.io](https://solscan.io) |
| **XRPL** | not yet deployed | [xrpscan.com](https://xrpscan.com) |

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

[mpc]
share_index = 0           # This validator's key share index (0-based)
threshold = 2             # Minimum signatures required
total_parties = 3         # Total validator count
key_share_path = "./data/key_share.bin"  # Persisted Ed25519 key share

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

**Design intent** (not yet real — see status banner):

| Threat | Intended mitigation |
|---|---|
| Single validator colludes | MPC threshold — needs 67% to sign |
| Validator goes offline | Transfer requeued after timeout, other validators continue |
| Double spend | Transfer ID is globally unique, consensus records prevent replay |
| Vault drain | Only a valid threshold signature unlocks funds |
| Network partition | Gossipsub retries + Kademlia rerouting; quorum drops gracefully |

**Actual state today:** votes are not cryptographically authenticated (a
peer claiming to be a known validator's PeerId is currently trusted — see
`consensus::record_vote`'s doc comment), there is no real threshold
signature, and `execute_withdrawal()` refuses to run on all three chains
rather than move funds it can't safely authorize. Net effect: nothing can
currently be stolen through this bridge because nothing can currently be
withdrawn through it. Do not read "no MPC" as "no risk" once withdrawal
execution is implemented — the consensus authentication gap must be closed
first.

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
