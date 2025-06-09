# HyperChain: A Heterogeneous, Post-Quantum DLT Framework

![HyperChain Banner](https://placehold.co/1200x300/1a1a2e/e0e0e0?text=HyperChain)

**Repository for the official Rust implementation of the HyperChain Protocol.**

**Author**: trvorth | **License**: MIT

---

## Abstract

HyperChain is a next-generation Layer-0 protocol featuring a heterogeneous, multi-chain architecture designed for scalability, interoperability, and post-quantum security. The framework natively supports two distinct but interoperable ledger models: high-throughput **DAG Shards** for parallel transaction processing and application-specific **Execution Chains**. Consensus is achieved via a hybrid Proof-of-Work and Proof-of-Stake mechanism, and security is hardened with post-quantum cryptography.

For a comprehensive academic and technical overview, please refer to the official [**HyperChain Whitepaper**](./docs/hyperchain-whitepaper.pdf).

## Key Features

* **Heterogeneous Architecture**: Natively supports both DAG-based shards and linear PoW/PoS chains within one interoperable ecosystem.
* **Dynamic Sharding**: The network autonomously adjusts the number of active DAG shards based on real-time transactional load, ensuring scalability.
* **Hybrid Consensus (PoW-DF)**: Combines permissionless Proof-of-Work for block proposal with Proof-of-Stake for deterministic finality.
* **Post-Quantum Security**: Implements a lattice-based signature scheme (modeled after NIST standard CRYSTALS-Dilithium) for all validator attestations, ensuring long-term security.
* **On-Chain Governance**: A decentralized, stake-weighted governance mechanism allows for protocol upgrades and parameter changes without contentious hard forks.
* **Advanced Cryptography**: Includes specifications for Zero-Knowledge Proofs (zk-SNARKs) and Homomorphic Encryption for future privacy-preserving features.
* **Advanced Security**: Features a novel on-chain Intrusion Detection System (IDS) that economically penalizes validators for anomalous behavior.

## Project Structure

The `hyperchain` repository is a Cargo workspace containing two primary crates:
* `src/`: The core implementation of the DAG shards, P2P networking, consensus, and node runtime.
* `myblockchain/`: The implementation for the linear Execution Chains, featuring the unique Reliable Hashing Algorithm (RHA).

## Prerequisites

To build and run a HyperChain node, you will need to have the following installed on your system:

* **Rust Toolchain**: Install the latest stable version of Rust via `rustup`.
    ```bash
    curl --proto '=https' --tlsv1.2 -sSf [https://sh.rustup.rs](https://sh.rustup.rs) | sh
    ```
* **Git**: Required for cloning the repository.
* **Build Dependencies**: A C++ compiler and the RocksDB library are required. Please follow the instructions specific to your operating system below.

## Build Instructions (Linux & macOS)

1.  **Install Build Essentials**:
    * **Debian/Ubuntu**: `sudo apt-get update && sudo apt-get install build-essential clang librocksdb-dev`
    * **macOS (with Homebrew)**: `xcode-select --install && brew install rocksdb`
    * **Fedora/CentOS**: `sudo dnf groupinstall "Development Tools" && sudo dnf install rocksdb-devel`

2.  **Clone and Compile**:
    ```bash
    git clone [https://github.com/trvorth/hyperchain.git](https://github.com/trvorth/hyperchain.git)
    cd hyperchain
    cargo build --release
    ```

## Build Instructions (Windows)

Building on Windows requires the MSVC C++ toolchain and manual installation of RocksDB via `vcpkg`.

1.  **Install Microsoft C++ Build Tools**:
    * Download the [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/).
    * Run the installer and select the **"C++ build tools"** workload. Make sure the latest Windows SDK and English language pack are included.

2.  **Install and Configure `vcpkg`**:
    * Open PowerShell and clone the `vcpkg` repository.
        ```powershell
        git clone [https://github.com/Microsoft/vcpkg.git](https://github.com/Microsoft/vcpkg.git)
        cd vcpkg
        ./bootstrap-vcpkg.bat
        ./vcpkg integrate install
        ```

3.  **Install RocksDB via `vcpkg`**:
    * Use `vcpkg` to install the required 64-bit RocksDB library. This may take some time.
        ```powershell
        ./vcpkg.exe install rocksdb:x64-windows
        ```

4.  **Set Environment Variables**:
    * You must set an environment variable to tell Cargo where to find the RocksDB library files. Open PowerShell as an **Administrator** and run the following command (adjust the path if you installed `vcpkg` elsewhere):
        ```powershell
        [System.Environment]::SetEnvironmentVariable('ROCKSDB_LIB_DIR', 'C:\path\to\vcpkg\installed\x64-windows\lib', [System.EnvironmentVariableTarget]::Machine)
        ```
    * **IMPORTANT**: You will need to **restart your terminal or IDE** for this environment variable to take effect.

5.  **Clone and Compile**:
    * Open a **new** terminal window.
    ```bash
    git clone [https://github.com/trvorth/hyperchain.git](https://github.com/trvorth/hyperchain.git)
    cd hyperchain
    cargo build --release
    ```
The compiled binary will be located at `target/release/hyperdag.exe`.

## Running a Node: A Quick Start Guide

1.  **Generate Your Wallet:**
    If this is your first time running a node, or if you intend to be a genesis validator, you must generate a wallet. The `keygen` utility creates a new keypair and saves it to `wallet.key` in the root directory.
    ```bash
    cargo run --bin keygen
    ```
    **IMPORTANT**: The output will display your `Private Key`, `Public Address`, and `Mnemonic Phrase`. Back up your mnemonic phrase in a secure, offline location. **The public address is what you will use as the `genesis_validator` in the config file.**

2.  **Configure Your Node:**
    The repository includes an example configuration file. Copy it to create your own `config.toml`.
    ```bash
    cp config.toml.example config.toml
    ```
    Open `config.toml` in your text editor. At a minimum, you must set the `genesis_validator` field to the public address you generated in the previous step.

3.  **Launch the Node:**
    Start the HyperChain node, pointing it to your configuration file.
    ```bash
    # On Linux/macOS
    cargo run --bin hyperdag --config-path config.toml

    # On Windows
    cargo run --bin hyperdag --config-path config.toml
    ```
    Your node will initialize, start its P2P services, and attempt to connect to peers or begin mining the genesis block if it's the first node.

## Testnet Participation

For details on joining the public testnet, including hardware requirements, incentive programs, and bootnode addresses, please refer to the [**Testnet Launch Plan**](./docs/testnet-plan.md).

## Security

The security of the network is our highest priority. We have a formal plan for a comprehensive third-party audit. For more details, please see our [**Security Audit Plan**](./docs/security-audit-plan.md).

## Contributing

We welcome contributions from the community. Please feel free to open issues or submit pull requests. All contributions should adhere to our code of conduct and contribution guidelines (to be published).

## License

This project is licensed under the MIT License. See the [LICENSE](./LICENSE) file for details.

