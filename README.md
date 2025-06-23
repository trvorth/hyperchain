# HyperChain: A Heterogeneous, Post-Quantum DLT Framework

![HyperChain Banner](https://placehold.co/1200x300/1a1a2e/e0e0e0?text=HyperChain)

**Repository for the official Rust implementation of the HyperChain Protocol.**  

**Author**: trvorth | **License**: MIT | **Status**: Phase 1 \- Foundation (In Progress)

---

## **About HyperChain**

**Website**: https://hyperchain.pro (coming soon)  
**Topics**: blockchain, layer-0, dag, rust, post-quantum-cryptography, fintech, decentralized-finance  
HyperChain is a next-generation Layer-0 protocol designed for scalability, interoperability, and long-term security. It features a heterogeneous, multi-chain architecture that combines the speed of Directed Acyclic Graphs (DAGs) with the security of traditional blockchains. By supporting parallel transaction processing and implementing post-quantum cryptography, HyperChain provides a robust and future-proof foundation for decentralized applications, finance, and more.  
For a comprehensive academic and technical overview, please refer to the official [**HyperChain Whitepaper**](http://docs.google.com/docs/hyperchain-whitepaper.md).

## **Key Features**

* **Heterogeneous Architecture**: Natively supports both DAG-based shards and linear PoW/PoS chains within one interoperable ecosystem.  
* **Dynamic Sharding**: The network autonomously adjusts the number of active DAG shards based on real-time transactional load, ensuring scalability.  
* **Hybrid Consensus (PoW-DF)**: Combines permissionless Proof-of-Work for block proposal with Proof-of-Stake for deterministic finality.  
* **Post-Quantum Security**: Implements a lattice-based signature scheme (modeled after NIST standard CRYSTALS-Dilithium) for all validator attestations, ensuring long-term security.  
* **On-Chain Governance**: A decentralized, stake-weighted governance mechanism allows for protocol upgrades and parameter changes without contentious hard forks.  
* **Advanced Cryptography**: Includes specifications for Zero-Knowledge Proofs (zk-SNARKs) and Homomorphic Encryption for future privacy-preserving features.  
* **Advanced Security**: Features a novel on-chain Intrusion Detection System (IDS) that economically penalizes validators for anomalous behavior.

## **Project Structure**

The hyperchain repository is a Cargo workspace containing several key components:

* **/src**: The main library crate containing the core logic.  
  * **hyperdag.rs**: The DAG-based ledger implementation.  
  * **node.rs**: The main node orchestrator.  
  * **p2p.rs**: Peer-to-peer networking using libp2p.  
  * **consensus.rs**: Consensus rules and Proof-of-Work logic.  
  * **wallet.rs**: Wallet generation, encryption, and signing logic.  
* **/src/bin**: Executable crates for the node (start\_node.rs) and wallet (hyperwallet.rs).  
* **/docs**: Project documentation, including the whitepaper and launch plans.  
* **config.toml.example**: An example configuration file for the node.

## **Getting Started: Running a Local Node**

These instructions will get you a copy of the project up and running on your local machine for development and testing purposes.

### **Prerequisites**

To build and run a HyperChain node, you will need to have the following installed on your system:

* **Rust Toolchain**: Install the latest stable version of Rust via rustup.  
  curl \--proto '=https' \--tlsv1.2 \-sSf \[https://sh.rustup.rs\](https://sh.rustup.rs) | sh

* **Git**: Required for cloning the repository.  
* **Build Dependencies**: A C++ compiler and the RocksDB library are required. Please follow the instructions specific to your operating system below.

### **Build Instructions (Linux & macOS)**

1. **Install Build Essentials**:  
   * **Debian/Ubuntu**: sudo apt-get update && sudo apt-get install build-essential clang librocksdb-dev  
   * **macOS (with Homebrew)**: xcode-select \--install && brew install rocksdb  
   * **Fedora/CentOS**: sudo dnf groupinstall "Development Tools" && sudo dnf install rocksdb-devel  
2. **Clone and Compile**:  
   git clone \[https://github.com/trvorth/hyperchain.git\](https://github.com/trvorth/hyperchain.git)  
   cd hyperchain  
   cargo build \--release

The compiled binaries will be located at target/release/.

### **Build Instructions (Windows)**

Building on Windows requires the MSVC C++ toolchain and manual installation of RocksDB via vcpkg.

1. **Install Microsoft C++ Build Tools**:  
   * Download the [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/).  
   * Run the installer and select the **"C++ build tools"** workload. Make sure the latest Windows SDK and English language pack are included.  
2. **Install and Configure vcpkg**:  
   * Open PowerShell and clone the vcpkg repository.  
     git clone \[https://github.com/Microsoft/vcpkg.git\](https://github.com/Microsoft/vcpkg.git)  
     cd vcpkg  
     ./bootstrap-vcpkg.bat  
     ./vcpkg integrate install

3. **Install RocksDB via vcpkg**:  
   * Use vcpkg to install the required 64-bit RocksDB library. This may take some time.  
     ./vcpkg.exe install rocksdb:x64-windows

4. **Set Environment Variables**:  
   * You must set an environment variable to tell Cargo where to find the RocksDB library files. Open PowerShell as an **Administrator** and run the following command (adjust the path if you installed vcpkg elsewhere):  
     \[System.Environment\]::SetEnvironmentVariable('ROCKSDB\_LIB\_DIR', 'C:\\path\\to\\vcpkg\\installed\\x64-windows\\lib', \[System.EnvironmentVariableTarget\]::Machine)

   * **IMPORTANT**: You will need to **restart your terminal or IDE** for this environment variable to take effect.  
5. **Clone and Compile**:  
   * Open a **new** terminal window.

git clone \[https://github.com/trvorth/hyperchain.git\](https://github.com/trvorth/hyperchain.git)  
cd hyperchain  
cargo build \--release

The compiled binaries will be located at target/release/.

### **Quick Start**

1. **Generate a Wallet:** The hyperwallet utility creates a new keypair. You will be prompted to set a secure passphrase.  
   cargo run \--release \--bin hyperwallet new

   **IMPORTANT**: Copy the Public Address from the output. You will need it for the next step.  
2. **Configure Your Node:** Copy the example configuration file.  
   cp config.toml.example config.toml

   Open config.toml and set the initial\_validators field to the public address you just generated.  
3. **Launch the Node:** Start the HyperChain node. You will be prompted for your wallet passphrase.  
   cargo run \--release \--bin start\_node

## **Developer Resources**

* **Whitepaper**: [docs/hyperchain-whitepaper.md](http://docs.google.com/docs/hyperchain-whitepaper.md)  
* **Architecture Overview**: [Architecture.md](http://docs.google.com/Architecture.md)  
* **API Documentation**: (Coming Soon) A full specification for the public RPC and REST APIs will be published ahead of the mainnet launch.  
* **CLI Wallet**: The hyperwallet binary provides a command-line interface for all wallet and key management operations. See "Running a Node" for details.

## **Testnet Participation**

For details on joining the public testnet, including hardware requirements, incentive programs, and bootnode addresses, please refer to the [**Testnet Launch Plan**](./docs/testnet-plan.md).

## **Security**

The security of the network is our highest priority. We have a formal plan for a comprehensive third-party audit. For more details, please see our [**Security Audit Plan**](./docs/security-audit-plan.md).

## **Contributing**

We welcome contributions from the community\! This project thrives on collaboration and outside feedback. Please read our [**Contribution Guidelines**](http://docs.google.com/CONTRIBUTING.md) to get started.  
All participants are expected to follow our [**Code of Conduct**](http://docs.google.com/CODE_OF_CONDUCT.md).

## **License**

This project is licensed under the MIT License. See the [LICENSE](http://docs.google.com/LICENSE) file for details.

