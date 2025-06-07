use hyperdag::node::Node;
use hyperdag::config::Config;

#[tokio::main]
async fn main() {
    env_logger::init();
    log::info!("Starting HyperDAG testnet node");

    let config = Config {
        p2p_address: "/ip4/127.0.0.1/tcp/8082".to_string(),
        api_address: "0.0.0.0:9002".to_string(),
        peers: vec!["/ip4/127.0.0.1/tcp/8081".to_string(), "/ip4/127.0.0.1/tcp/8080".to_string()],
        genesis_validator: "2119707c4caf16139cfb5c09c4dcc9bf9cfe6808b571c108d739f49cc14793b9".to_string(),
        target_block_time: 60,
        difficulty: 1,
        max_amount: 1_000_000_000,
        use_gpu: false,
    };

    let node = Node::new(config).await.expect("Failed to create node");
    node.start().await.expect("Failed to start node");
}