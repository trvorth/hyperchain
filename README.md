# HyperChain: A Heterogeneous, Post-Quantum DLT Framework

![HyperChain Banner](https://placehold.co/1200x300/1a1a2e/e0e0e0?text=HyperChain)

**Repository for the official Rust implementation of the HyperChain Protocol.**  

**Author**: trvorth | **License**: MIT | **Status**: Phase 1 \- Foundation (In Progress)

---

## **About HyperChain**

**Website**: https://hyperchain.pro (coming soon)  

**Topics**: blockchain, layer-0, dag, rust, post-quantum-cryptography, fintech, decentralized-finance  
HyperChain is a next-generation Layer-0 protocol implemented in Rust, designed to provide a scalable, interoperable, and secure foundation for decentralized applications and finance. Its heterogeneous architecture integrates Directed Acyclic Graphs (DAGs) for high-throughput parallel transaction processing with traditional PoW/PoS chains for robust security. Key features include dynamic sharding to adapt to transaction loads, a hybrid consensus mechanism combining Proof-of-Work for block proposals and Proof-of-Stake for deterministic finality, and post-quantum cryptography using lattice-based signatures (modeled after CRYSTALS-Dilithium) for long-term security. HyperChain also supports on-chain governance to enable seamless protocol upgrades and plans to incorporate advanced cryptographic tools like zk-SNARKs and homomorphic encryption for privacy-preserving features.

While primarily a Layer-0 protocol facilitating interoperability across its ecosystem of shards and chains, HyperChain can host Layer-1-like chains within its framework, processing transactions and smart contracts independently. Additionally, its planned zk-SNARKs integration could enable Layer-2 scaling solutions, such as rollups, on its shards, enhancing throughput while leveraging HyperChain’s interoperability. Currently in Phase 1 (Foundation), the repository includes core components like the DAG ledger, node orchestrator, P2P networking, and wallet functionality, alongside documentation for local setup and testnet participation. Licensed under the MIT License, HyperChain welcomes community contributions to drive its vision of a future-proof decentralized ecosystem.

For a comprehensive academic and technical overview, please refer to the official [**HyperChain Whitepaper**](./hyperchain-whitepaper.pdf).

## **Core Architectural Tenets**

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

## **Developer & Research Materia**

* **Formal Specification (Whitepaper)**: [docs//whitepaper/hyperchain-whitepaper.md](./docs/whitepaper/hyperchain-whitepaper.md)
* **System Architecture Overview**: [Architecture.md](./Architecture.md)
* **API Documentation**: A complete specification for the public RPC and REST Application Programming Interfaces is slated for publication prior to the mainnet launch.
* **Command-Line Interface (CLI) Wallet**: The `hyperwallet` executable furnishes a command-line interface for all requisite wallet and cryptographic key management operations.

## Procedural Guide for Local Instantiation

The subsequent instructions delineate the procedures for obtaining and operating a functional instance of the project on a local machine for purposes of development and testing.

## **Getting Started: Running a Local Node**

These instructions will get you a copy of the project up and running on your local machine for development and testing purposes.

### **Prerequisites**

To build and run a HyperChain node, you will need to have the following installed on your system:

* **Rust Toolchain**: The latest stable release of the Rust programming language and its associated Cargo build system must be installed via `rustup`.
    ```bash
    curl --proto '=https' --tlsv1.2 -sSf [https://sh.rustup.rs](https://sh.rustup.rs) | sh
    ```
* **Git**: The Git version control system is required for cloning the source code repository.
* **Build-System Dependencies**: A C++ compiler toolchain and the RocksDB library constitute essential build dependencies. The requisite installation procedures vary by operating system.

### **Build Instructions (Linux & macOS)**

1. **Install Build Essentials**:  
    * For Debian-based distributions (e.g., Ubuntu):
        ```bash
        sudo apt-get update && sudo apt-get install build-essential clang librocksdb-dev
        ```
    * For macOS systems utilizing the Homebrew package manager:
        ```bash
        xcode-select --install && brew install rocksdb
        ```
    * For Fedora, CentOS, or RHEL-based distributions:
        ```bash
        sudo dnf groupinstall "Development Tools" && sudo dnf install rocksdb-devel
        ```
 
2. **Clone and Compile**:  
    ```bash
    git clone [https://github.com/trvorth/hyperchain.git](https://github.com/trvorth/hyperchain.git)
    cd hyperchain
    cargo build --release
    ```
    Upon successful compilation, the resultant binaries will be situated in the `target/release/` directory.

### **Build Instructions (Windows)**

Building on Windows requires the MSVC C++ toolchain and manual installation of RocksDB via vcpkg.

1.  **Installation of Microsoft C++ Build Tools**:
    * It is required to download the [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/).
    * The installer must be executed, and the **"C++ build tools"** workload is to be selected for installation. It should be ensured that the latest Windows SDK and the English language pack are included.

2.  **Installation and Configuration of `vcpkg`**:
    * An instance of the PowerShell terminal is to be opened for the purpose of cloning the `vcpkg` repository.
        ```powershell
        git clone [https://github.com/Microsoft/vcpkg.git](https://github.com/Microsoft/vcpkg.git)
        cd vcpkg
        ./bootstrap-vcpkg.bat
        ./vcpkg integrate install
        ```

3.  **Installation of RocksDB via `vcpkg`**:
    * The `vcpkg` utility shall be used to install the 64-bit version of the RocksDB library. This operation may require a considerable amount of time to complete.
        ```powershell
        ./vcpkg.exe install rocksdb:x64-windows
        ```

4.  **Configuration of Environment Variables**:
    * An environment variable must be established to inform the Cargo build system of the location of the RocksDB library files. A PowerShell terminal with administrative privileges must be utilized to execute the following command, with the file path adjusted to correspond to the `vcpkg` installation directory:
        ```powershell
        [System.Environment]::SetEnvironmentVariable('ROCKSDB_LIB_DIR', 'C:\path\to\vcpkg\installed\x64-windows\lib', [System.EnvironmentVariableTarget]::Machine)
        ```
    * **Note Bene**: A restart of the terminal or Integrated Development Environment (IDE) is mandatory for this environment variable modification to take effect.

5.  **Repository Cloning and Compilation**:
    * A new terminal instance must be opened.
    ```bash
    git clone [https://github.com/trvorth/hyperchain.git](https://github.com/trvorth/hyperchain.git)
    cd hyperchain
    cargo build --release
    ```
    The compiled binaries will be located at `target/release/`.

### **Operational Quick Start**

1.  **Wallet Credential Generation**:
    The `hyperwallet` utility is provided for the creation of a new keypair. Upon execution, the operator will be prompted to supply a secure passphrase for the encryption of the resultant wallet file, `wallet.key`.
    ```bash
    cargo run --release --bin hyperwallet new
    ```
   **Critical Notice**: The `Public Address` emitted by this operation must be copied. Furthermore, the associated `Mnemonic Phrase` must be transcribed and stored in a secure, offline medium for recovery purposes.

2.  **Node Configuration**:
    An exemplary configuration file is provided within the repository. A local copy must be created for operational use.
    ```bash
    cp config.toml.example config.toml
    ```
    The newly created `config.toml` file must be edited to substitute the placeholder value of the `initial_validators` field with the public address generated in the preceding step.

3.  **Node Instantiation**:
    The HyperChain node may be initiated by executing the `start_node` binary. The system will automatically load the configuration and wallet files.
    ```bash
    cargo run --release --bin start_node
    ```
    The operator will be prompted to supply the wallet passphrase, after which the node will initialize its services and commence network operations.

## **Testnet Participation**

For details on joining the public testnet, including hardware requirements, incentive programs, and bootnode addresses, please refer to the [**Testnet Launch Plan**](./docs/testnet-plan.md).

## **Security**

The security of the network is our highest priority. We have a formal plan for a comprehensive third-party audit. For more details, please see our [**Security Audit Plan**](./docs/security-audit-plan.md).

## **Contribution Protocol**

We welcome contributions from the community\! This project thrives on collaboration and outside feedback. Please read our [**Contribution Guidelines**](./CONTRIBUTING.md) to get started.  
All participants are expected to follow our [**Code of Conduct**](./CODE_OF_CONDUCT.md).

## **License**

This project is licensed under the MIT License. See the [LICENSE](./LICENSE) file for details.

