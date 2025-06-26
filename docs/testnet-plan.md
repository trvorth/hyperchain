# **HyperChain Public Testnet Launch Plan**

**Codename: Ignition** **Version: 1.0** **Status: Planning**

## **1\. Executive Summary**

This document outlines the strategic, phased launch plan for the HyperChain Public Testnet, codenamed "Ignition." The Ignition testnet is a critical pre-production environment designed to rigorously validate the protocol's architecture, security, performance, and economic incentives in a live, decentralized setting. The successful execution of this plan is a primary prerequisite for mainnet consideration, as it will provide the empirical data and community engagement necessary to ensure a stable and secure launch.

The launch is structured into three progressive phases:
* **Phase 0 (Pre-engagement): Pre-flight Checklist
* **Phase 1 (Devnet):** An internal, controlled network for core protocol validation.  
* **Phase 2 (Gladius):** An incentivized, permissioned network for external validators to test security and performance under structured challenges.  
* **Phase 3 (Agora):** A permissionless, public network for open participation, dApp development, and governance testing.

## **2\. Testnet Objectives**

The Ignition testnet is designed to achieve the following key objectives:

* **Architectural Validation:** Empirically verify the stability and performance of the heterogeneous architecture, including both the DAG shards and Execution Chains.  
* **Consensus Security:** Confirm the hybrid consensus mechanisms (PoW-DF and Weighted PoW) correctly achieve finality and are resilient to common attacks.  
* **Scalability Testing:** Analyze the performance of the autonomous dynamic sharding algorithm under sustained, high-volume transactional load.  
* **Economic Viability:** Test the tokenomic model and incentive structures to ensure they promote network security and decentralization.  
* **Vulnerability Discovery:** Proactively identify and remediate bugs, design flaws, and security vulnerabilities through community engagement and a formal audit.  
* **Community Bootstrapping:** Cultivate a skilled and engaged global community of node operators, validators, developers, and users.

## **3\. Phased Rollout Strategy**

### **Phase 0: Pre-flight Checklist**

  * [x] Core protocol implementation complete.
  * [x] Local single-node and multi-node simulations successful.
  * [x] Initial `config.toml` template created.
  * [x] Deployment scripts for cloud infrastructure developed.
  * [x] Security vulnerabilities from automated scans (Dependabot, CodeQL) are patched or acknowledged.

### **Phase 1: Devnet (Internal)**

* **Timeline:** Weeks 1-2  
* **Participants:** Core development team, contracted partners.  
* **Goals:**  
  * Deploy a stable set of geographically distributed internal bootnodes.  
  * Verify baseline network functionality: peer discovery (Kademlia DHT, mDNS), block and transaction gossip, state synchronization.  
  * Conduct initial stress tests to establish performance benchmarks (Transactions Per Second, Time to Finality).  
  * Finalize node configuration parameters and publish comprehensive documentation for external participants.  
  * **Success Metric:** Stable network operation for 48 consecutive hours with no critical failures.

### **Phase 2: "Gladius" \- Incentivized Validator Program**

* **Timeline:** Weeks 3-8  
* **Participants:** Whitelisted community members and professional staking providers who meet the specified hardware and staking requirements.  
* **Goals:**  
  * Onboard and support the first wave of external, community-run validator nodes.  
  * Test the PoS finality gadget and the economic consequences of the on-chain IDS slashing mechanism.  
  * Monitor the `adjust_difficulty` algorithm's responsiveness to significant, planned changes in network hashrate.  
  * Execute "Game Day" Scenarios:  
    * **Scenario A (Censorship Attack):** A coordinated effort by a subset of miners to ignore transactions.  
    * **Scenario B (Validator Downtime):** A planned, simultaneous shutdown of a significant percentage of validators to test liveness.  
    * **Scenario C (Throughput Spike):** A high-volume transaction generation event to trigger the `dynamic_sharding` mechanism.  
  * Reward participants with testnet tokens based on uptime, performance, and successful completion of challenges.  
  * **Success Metric:** Successful execution of all Game Day scenarios and identification of at least one non-critical consensus bug.

### **Phase 3: "Agora" \- Public Open Testnet**

* **Timeline:** Week 9 onwards  
* **Participants:** Open to all, permissionless.  
* **Goals:**  
  * Achieve maximum network decentralization by allowing permissionless node joining.  
  * Encourage organic, high-volume network usage to test long-term stability and performance of the dynamic sharding system.  
  * Launch the HyperChain Developer Grant Program and onboard the first cohort of testnet dApps.  
  * Test the full on-chain governance lifecycle: proposal submission by stakeholders, public voting (referenda), and autonomous enactment of a successful protocol parameter change.  
  * Gather long-term data on network performance, economic activity, and validator dynamics.  
  * **Success Metric:** At least one successful on-chain governance proposal enacted and a stable network with 50+ independent validator nodes.

## **4\. Technical Specifications**

### **Validator Node Hardware (Recommended Minimum):**

* **CPU:** 4-Core / 8-Thread CPU @ 3.0 GHz+ (e.g., Intel Core i7-8700, AMD Ryzen 5 3600\)  
* **RAM:** 8 GB DDR4  
* **Storage:** 256 GB High-Speed NVMe SSD (to accommodate state growth)  
* **Network:** 5 Mbps symmetric connection with a public, static IP address.

### **Software Prerequisites**
* **A modern Linux distribution (e.g., Ubuntu 22.04 LTS).
* **Git version control system.
* **Rust toolchain (install via `rustup`).
* **Essential build tools (`build-essential`, `clang`, `librocksdb-dev` on Debian/Ubuntu).

### **Network Connection Details:**

* **Bootnodes:** A list of stable bootnode multi-addresses will be published in the official project documentation one week prior to the launch of Phase 2\.  
* **P2P Ports:** Nodes must have their configured P2P TCP port (default: 8000\) open to incoming connections.

## **5\. Incentive Program**

Participants in the "Gladius" phase will be eligible for rewards from a dedicated mainnet token allocation. The reward structure will be based on a point system allocated for:

* **Uptime:** Points awarded per epoch for validator node availability.  
* **Performance:** Points awarded for successfully signing and finalizing checkpoints.  
* **Bug Bounties:** Significant points awarded for the responsible disclosure of bugs, categorized by severity.  
* **Challenge Completion:** Bonus points for active and successful participation in "Game Day" scenarios.

Full details of the incentive program, including the points-to-token conversion rate, will be released before the start of the "Gladius" phase.

