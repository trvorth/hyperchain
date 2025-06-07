#[cfg(test)]
mod tests {
    use hyperdag::config::Config;
    use hyperdag::hyperdag::{HyperBlock, HyperDAG};
    use hyperdag::mempool::Mempool;
    use hyperdag::p2p::P2PServer;
    use hyperdag::transaction::{Transaction, UTXO};
    use hyperdag::wallet::HyperWallet;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::Mutex;
    use tokio::time::{sleep, Duration};

    #[tokio::test]
    async fn test_p2p_block_propagation() {
        let wallet = HyperWallet::new();
        let config1 = Config {
            p2p_address: "/ip4/127.0.0.1/tcp/8083".to_string(),
            api_address: "0.0.0.0:9004".to_string(),
            peers: vec!["/ip4/127.0.0.1/tcp/8084".to_string()],
            genesis_validator: wallet.get_address(),
            target_block_time: 30,
            difficulty: 1,
            max_amount: 1_000_000_000,
            use_gpu: false,
            zk_enabled: false,
            mining_threads: 4,
            num_chains: 2,
            mining_chain_id: 0,
            logging: Default::default(),
            p2p: Default::default(),
        };
        let config2 = Config {
            p2p_address: "/ip4/127.0.0.1/tcp/8084".to_string(),
            api_address: "0.0.0.0:9005".to_string(),
            peers: vec!["/ip4/127.0.0.1/tcp/8083".to_string()],
            genesis_validator: config1.genesis_validator.clone(),
            target_block_time: 30,
            difficulty: 1,
            max_amount: 1_000_000_000,
            use_gpu: false,
            zk_enabled: false,
            mining_threads: 4,
            num_chains: 2,
            mining_chain_id: 1,
            logging: Default::default(),
            p2p: Default::default(),
        };

        let dag1 = Arc::new(Mutex::new(HyperDAG::new(&config1.genesis_validator, 30, 1, 2)));
        let dag2 = Arc::new(Mutex::new(HyperDAG::new(&config2.genesis_validator, 30, 1, 2)));
        let mempool1 = Arc::new(Mutex::new(Mempool::new(3600)));
        let mempool2 = Arc::new(Mutex::new(Mempool::new(3600)));
        let utxos1 = Arc::new(Mutex::new(HashMap::new()));
        let utxos2 = Arc::new(Mutex::new(HashMap::new()));
        let proposals1 = Arc::new(Mutex::new(Vec::new()));
        let proposals2 = Arc::new(Mutex::new(Vec::new()));

        let (tx1, rx1) = tokio::sync::mpsc::channel(100);
        let (tx2, rx2) = tokio::sync::mpsc::channel(100);

        let mut p2p1 = P2PServer::new(
            "hyperdag",
            vec![config1.p2p_address.clone()],
            config1.peers.clone(),
            dag1.clone(),
            mempool1.clone(),
            utxos1.clone(),
            proposals1.clone(),
        )
        .await
        .unwrap();
        let mut p2p2 = P2PServer::new(
            "hyperdag",
            vec![config2.p2p_address.clone()],
            config2.peers.clone(),
            dag2.clone(),
            mempool2.clone(),
            utxos2.clone(),
            proposals2.clone(),
        )
        .await
        .unwrap();

        let p2p_handle1 = tokio::spawn(async move { p2p1.start(rx1).await });
        let p2p_handle2 = tokio::spawn(async move { p2p2.start(rx2).await });

        sleep(Duration::from_secs(2)).await; // Wait for peers to connect

        let block = HyperBlock {
            chain_id: 0,
            id: "test_block".to_string(),
            parents: vec!["genesis_0".to_string()],
            transactions: vec![],
            difficulty: 1,
            validator: config1.genesis_validator.clone(),
            nonce: 0,
            timestamp: 0,
            reward: 1000,
            cross_chain_references: vec![(1, "genesis_1".to_string())],
        };

        tx1.send(hyperdag::p2p::P2PCommand::BroadcastBlock(block.clone()))
            .await
            .unwrap();

        sleep(Duration::from_secs(2)).await;

        let dag2 = dag2.lock().await;
        assert!(dag2.blocks.contains_key("test_block"), "Block not propagated to peer");
        assert_eq!(dag2.blocks.get("test_block").unwrap().chain_id, 0, "Incorrect chain_id");
        assert_eq!(
            dag2.blocks.get("test_block").unwrap().cross_chain_references,
            vec![(1, "genesis_1".to_string())],
            "Cross-chain references not propagated"
        );

        p2p_handle1.abort();
        p2p_handle2.abort();
    }

    #[tokio::test]
    async fn test_p2p_state_sync() {
        let wallet = HyperWallet::new();
        let config1 = Config {
            p2p_address: "/ip4/127.0.0.1/tcp/8085".to_string(),
            api_address: "0.0.0.0:9006".to_string(),
            peers: vec!["/ip4/127.0.0.1/tcp/8086".to_string()],
            genesis_validator: wallet.get_address(),
            target_block_time: 30,
            difficulty: 1,
            max_amount: 1_000_000_000,
            use_gpu: false,
            zk_enabled: false,
            mining_threads: 4,
            num_chains: 2,
            mining_chain_id: 0,
            logging: Default::default(),
            p2p: Default::default(),
        };
        let config2 = Config {
            p2p_address: "/ip4/127.0.0.1/tcp/8086".to_string(),
            api_address: "0.0.0.0:9007".to_string(),
            peers: vec!["/ip4/127.0.0.1/tcp/8085".to_string()],
            genesis_validator: config1.genesis_validator.clone(),
            target_block_time: 30,
            difficulty: 1,
            max_amount: 1_000_000_000,
            use_gpu: false,
            zk_enabled: false,
            mining_threads: 4,
            num_chains: 2,
            mining_chain_id: 1,
            logging: Default::default(),
            p2p: Default::default(),
        };

        let dag1 = Arc::new(Mutex::new(HyperDAG::new(&config1.genesis_validator, 30, 1, 2)));
        let dag2 = Arc::new(Mutex::new(HyperDAG::new(&config2.genesis_validator, 30, 1, 2)));
        let mempool1 = Arc::new(Mutex::new(Mempool::new(3600)));
        let mempool2 = Arc::new(Mutex::new(Mempool::new(3600)));
        let utxos1 = Arc::new(Mutex::new(HashMap::new()));
        let utxos2 = Arc::new(Mutex::new(HashMap::new()));
        let proposals1 = Arc::new(Mutex::new(Vec::new()));
        let proposals2 = Arc::new(Mutex::new(Vec::new()));

        // Add a UTXO to node 1
        utxos1.lock().await.insert(
            "test_utxo".to_string(),
            UTXO {
                address: config1.genesis_validator.clone(),
                amount: 1000,
                tx_id: "test_tx".to_string(),
                output_index: 0,
            },
        );

        let (tx1, rx1) = tokio::sync::mpsc::channel(100);
        let (tx2, rx2) = tokio::sync::mpsc::channel(100);

        let mut p2p1 = P2PServer::new(
            "hyperdag",
            vec![config1.p2p_address.clone()],
            config1.peers.clone(),
            dag1.clone(),
            mempool1.clone(),
            utxos1.clone(),
            proposals1.clone(),
        )
        .await
        .unwrap();
        let mut p2p2 = P2PServer::new(
            "hyperdag",
            vec![config2.p2p_address.clone()],
            config2.peers.clone(),
            dag2.clone(),
            mempool2.clone(),
            utxos2.clone(),
            proposals2.clone(),
        )
        .await
        .unwrap();

        let p2p_handle1 = tokio::spawn(async move { p2p1.start(rx1).await });
        let p2p_handle2 = tokio::spawn(async move { p2p2.start(rx2).await });

        sleep(Duration::from_secs(2)).await; // Wait for peers to connect

        // Trigger state sync
        tx2.send(hyperdag::p2p::P2PCommand::RequestState)
            .await
            .unwrap();

        sleep(Duration::from_secs(2)).await;

        let utxos2 = utxos2.lock().await;
        assert!(utxos2.contains_key("test_utxo"), "UTXO not synced");
        assert_eq!(
            utxos2.get("test_utxo").unwrap().amount,
            1000,
            "Incorrect UTXO amount"
        );

        p2p_handle1.abort();
        p2p_handle2.abort();
    }

    #[tokio::test]
    async fn test_wallet_status_command() {
        let wallet = HyperWallet::new();
        let config = Config {
            p2p_address: "/ip4/127.0.0.1/tcp/8087".to_string(),
            api_address: "0.0.0.0:9008".to_string(),
            peers: vec![],
            genesis_validator: wallet.get_address(),
            target_block_time: 30,
            difficulty: 1,
            max_amount: 1_000_000_000,
            use_gpu: false,
            zk_enabled: false,
            mining_threads: 4,
            num_chains: 2,
            mining_chain_id: 0,
            logging: Default::default(),
            p2p: Default::default(),
        };

        let node = hyperdag::node::Node::new(config).await.unwrap();

        let tx = Transaction::new(
            wallet.get_address(),
            "receiver".to_string(),
            100,
            1,
            vec!["genesis_utxo_0_genesis_0".to_string()],
            vec![UTXO {
                address: "receiver".to_string(),
                amount: 100,
                tx_id: "".to_string(),
                output_index: 0,
            }],
            &wallet.get_signing_key(),
        );

        let mut mempool = node.mempool.lock().await;
        let utxos = node.utxos.lock().await;
        let mut dag = node.dag.lock().await;
        mempool.add_transaction(tx.clone(), &utxos, &mut *dag).unwrap();

        let client = reqwest::Client::new();
        let mempool_response = client
            .get("http://0.0.0.0:9008/mempool")
            .send()
            .await
            .unwrap()
            .json::<HashMap<String, Transaction>>()
            .await
            .unwrap();
        assert!(mempool_response.contains_key(&tx.id), "Transaction not found in mempool");
    }
}