# GSTD Bridge — Development Guide

## Stack
- **Language**: Rust (stable)
- **P2P**: libp2p (Kademlia DHT + Gossipsub)
- **Crypto**: Ed25519 threshold signatures (Shamir t-of-n)
- **Chains**: TON (toncenter API), Solana (JSON-RPC), XRPL (WebSocket)
- **Config**: TOML (`bridge.toml`)
- **Logging**: `tracing` + `tracing-subscriber`

## Local Dev
```bash
cp bridge.toml.example bridge.toml   # edit RPC URLs + vault addresses
cargo build
./target/debug/gstd-bridge
```

## Key modules
- `src/config.rs` — BridgeConfig (TOML), MpcConfig
- `src/chains/` — ChainMonitor trait + TON/Solana/XRPL implementations
- `src/mpc/mod.rs` — ThresholdSigner::load_or_create() (persists key_share.bin)
- `src/mpc/keygen.rs` — load_or_generate_persistent_share()
- `src/consensus/mod.rs` — 67% quorum voting engine
- `src/p2p/mod.rs` — libp2p Swarm
- `src/main.rs` — startup, wires everything together

## MPC Key Shares
Key shares are saved to `./data/key_share.bin` (chmod 600) on first run.
**Back up this file.** If lost, the validator can no longer participate in signing.
In multi-validator setups each node has a DIFFERENT share_index in `bridge.toml`.

## Running in production
```bash
cp .env.example .env          # fill in RPC URLs + vault addresses
docker-compose up -d
docker-compose logs -f bridge
```

## DO NOT
- Do not hardcode ThresholdSigner(2,3,0) — use config.mpc values
- Do not delete data/key_share.bin — it cannot be recovered
- Do not run two validators with the same share_index
- Do not skip `verify_deposit()` before voting
