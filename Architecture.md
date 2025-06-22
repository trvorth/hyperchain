# Hyperchain Architecture

This document provides a high-level overview of the Hyperchain node architecture, detailing its primary components and their interactions.

## 1. Core Components

The Hyperchain software is built as a modular collection of Rust crates, each responsible for a distinct piece of functionality.

### 1.1. Node (`node.rs`)
The `Node` is the central orchestrator. It initializes and manages all other components, including the P2P networking layer, the HyperDAG ledger, the mempool, and the miner. It handles startup, shutdown, and the main event loop that drives the system.

### 1.2. P2P Networking (`p2p.rs`)
Networking is built on the `libp2p` framework, providing a robust and flexible foundation for peer-to-peer communication.
- **Protocol:** Uses TCP for transport, with plans to integrate QUIC.
- **Discovery:** Employs mDNS for local peer discovery and a Kademlia DHT for discovering and connecting to peers on the wider internet.
- **Communication:** Implements a request-response protocol for direct communication and a gossipsub protocol for broadcasting blocks and transactions efficiently across the network.

### 1.3. HyperDAG Ledger (`hyperdag.rs`)
Instead of a single-chain blockchain, Hyperchain uses a Directed Acyclic Graph (DAG) of blocks, which we call a **HyperDAG**.
- **Parallel Chains:** The DAG consists of multiple parallel "chains" (shards), allowing for concurrent block production and higher transaction throughput.
- **Structure:** Each block can have multiple parents, linking not just to the previous block in its own chain but also to blocks in other chains. This cross-linking creates the DAG structure and ensures eventual consistency across the entire ledger.
- **Storage:** Block data is persisted to disk using `RocksDB`, a high-performance key-value store.

### 1.4. Consensus (`consensus.rs`)
The consensus mechanism is designed to be simple and efficient, leveraging the DAG structure.
- **Proof-of-Work:** A basic PoW algorithm is used to control the rate of block creation and secure the network. The difficulty is adjusted dynamically based on block times.
- **Tip Selection:** Miners select the "tips" (leaf nodes) of the DAG as parents for new blocks. The selection algorithm will be refined to favor tips that confirm the most transactions.
- **Finality:** Transaction finality is probabilistic and increases as more blocks are added to the DAG on top of it.

### 1.5. Mempool (`mempool.rs`)
The `Mempool` stores unconfirmed transactions that have been received by the node but not yet included in a block. It manages transaction ordering, fee prioritization, and eviction of old transactions.

### 1.6. Wallet (`wallet.rs`)
The wallet is a command-line interface (CLI) tool for managing user accounts.
- **Cryptography:** It uses `ed25519-dalek` for digital signatures (signing and verifying transactions) and `bip39` for mnemonic seed phrases.
- **Encryption:** Wallet files are encrypted at rest using AES-256-GCM, with the key derived from a user-provided passphrase via Argon2.

### 1.7. Executable Bins (`/src/bin/`)
- **`start_node`**: The main entry point for running a Hyperchain node.
- **`hyperwallet`**: The CLI for wallet creation, imports, and other account management tasks.

This modular architecture allows for individual components to be upgraded and improved independently as the project evolves.
