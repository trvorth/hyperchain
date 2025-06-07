# Hyperchain

Hyperchain is a DAG-based blockchain with Proof-of-Stake (PoS) consensus, P2P networking, and zero-knowledge proof support. It features GPU mining, secure wallet management, and a REST API.

## Features
- **DAG-Based Consensus**: Uses a HyperDAG for high throughput and scalability.
- **Proof-of-Stake**: Validators selected based on stake.
- **GPU Mining**: Supports OpenCL-based mining for performance.
- **Zero-Knowledge Proofs**: Privacy for UTXO ownership using Groth16 ZK-SNARKs.
- **Secure Wallet**: Encrypted key storage with Argon2 and BIP-39 mnemonics.
- **P2P Networking**: Uses `libp2p` with gossipsub and message compression.
- **REST API**: Authenticated endpoints for transactions, balances, and more.

## Prerequisites
- Rust (stable)
- Node.js and npm (for web frontend)
- OpenCL libraries (for GPU mining)
- RocksDB (for storage)

## Setup
1. Clone the repository:
   ```bash
   git clone <repository>
   cd hyperchain
   ```
2. Install dependencies:
   ```bash
   cargo build
   cd web
   npm install
   ```
3. Create `private_key.txt` with a hex-encoded Ed25519 private key:
   ```bash
   cargo run --bin keygen
   ```
4. Create `config.toml`:
   ```toml
   p2p_address = "/ip4/127.0.0.1/tcp/8000"
   api_address = "0.0.0.0:9000"
   peers = ["/ip4/127.0.0.1/tcp/8080", "/ip4/127.0.0.1/tcp/8081"]
   genesis_validator = "2119707c4caf16139cfb5c09c4dcc9bf9cfe6808b571c108d739f49cc14793b9"
   target_block_time = 60
   difficulty = 1
   max_amount = 1000000000
   use_gpu = false
   zk_enabled = false
   mining_threads = 4

   [logging]
   level = "info"

   [p2p]
   heartbeat_interval = 500
   mesh_n = 4
   mesh_n_low = 1
   mesh_n_high = 8
   ```

## Running a Node
- Start a testnet node:
  ```bash
  cargo run --bin hyperdag_testnet
  ```
- Run a custom node:
  ```bash
  cargo run --bin hyperdag -- --p2p_address /ip4/127.0.0.1/tcp/8000 --api_address 0.0.0.0:9000
  ```
- Run multiple nodes:
  ```bash
  cargo run --bin hyperdag_node1 &
  cargo run --bin hyperdag_node2 &
  cargo run --bin hyperdag_node3 &
  ```

## API Endpoints
- **GET /info**: Node and DAG status (JWT required).
- **GET /balance/:address**: Address balance.
- **GET /utxos/:address**: Address UTXOs.
- **POST /transaction**: Submit a transaction (JWT required).
- **GET /block/:id**: Retrieve a block.
- **GET /dag**: DAG statistics.
- **GET /mempool**: Mempool transactions.
- **GET /health**: Health check.
- **GET /metrics**: Prometheus metrics.

To authenticate, obtain a JWT token via `/auth` (not implemented; use a mock token for testing).

## Web Frontend
1. Navigate to `web/`:
   ```bash
   cd web
   ```
2. Start the development server:
   ```bash
   npm run dev
   ```
3. Access at `http://localhost:5173`.

## Testing
Run all tests:
```bash
cargo test
```

## Configuration Options
- `use_gpu`: Enable GPU mining (requires OpenCL).
- `zk_enabled`: Enable ZK-SNARKs for private transactions.
- `mining_threads`: Number of CPU threads for mining (if GPU is disabled).

## Future Improvements
- Add transaction dependency tracking.
- Implement peer banning for malicious behavior.
- Enhance API with WebSocket support.