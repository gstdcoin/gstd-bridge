# GSTD Bridge — Development Guide

## Stack
- **Language**: Rust (stable)
- **P2P**: libp2p (Kademlia DHT + Gossipsub)
- **Crypto**: intended to be Ed25519 threshold signatures (Shamir t-of-n) — **not implemented as such today**.
  `mpc/keygen.rs` generates each validator's "share" as an independent random key with no shared
  secret polynomial. The old fake aggregation path (`aggregate_shares`/`lagrange_coefficient`, which
  hashed shares together in f64 float math and produced bytes that were not a valid signature on any
  target chain) was removed. `mpc/mod.rs`'s `try_collect_quorum()` is what's actually called
  (from `main.rs`) once threshold votes are in — it honestly returns the set of independently-verified
  Ed25519 signature shares without combining them into one signature. Do not treat this as real
  threshold cryptography until a real FROST/GG20 DKG scheme replaces it.
- **Chains**: TON (toncenter API), Solana (JSON-RPC), XRPL (WebSocket) — watch/detect logic only;
  `execute_withdrawal()` on all three no longer fabricates a tx hash — it now returns an explicit
  "not implemented" error (there's no real signature behind `signature_shares` to safely submit
  on-chain), which triggers `main.rs`'s existing re-lock-the-vault error path.
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

## Consensus / vote authentication
`consensus::record_vote()` now rejects any vote whose `voter` isn't in the node's
current set of connected validators (`self.validators`) — previously it accepted
any string. This is still **not** cryptographic authentication: `signature_share`
isn't checked against a registered per-validator verification key, and the gossip
layer's authenticated message source isn't threaded through to this call yet. A
peer that knows another validator's PeerId string can still forge a vote in its
name. See the doc comment on `record_vote` before treating quorum as spoof-proof.

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
