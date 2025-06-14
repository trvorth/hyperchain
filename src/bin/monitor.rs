use std::time::Duration;
use tokio::time;

struct Node {
    name: String,
    api_url: String, 
}

impl Node {
    fn new(name: &str, url: &str) -> Self {
        Node {
            name: name.to_string(),
            api_url: url.to_string(),
        }
    }

    async fn fetch_status(&self) -> Result<serde_json::Value, reqwest::Error> {
        reqwest::get(&self.api_url).await?.json().await
    }
}

#[tokio::main]
async fn main() {
    // You would replace these with the public IPs of your nodes
    let nodes = vec![
        Node::new("Node 1 (Asia)", "http://34.126.103.74:8080/status"),
        Node::new("Node 2 (US)", "http://104.197.0.155:8080/status"),
        Node::new("Node 3 (EU)", "http://35.195.207.69:8080/status"),
    ];

    let mut interval = time::interval(Duration::from_secs(5));

    loop {
        interval.tick().await;
        print!("\x1B[2J\x1B[1;1H");

        println!("--- HyperChain Global Testnet Monitor (API) ---");
        println!("{:<22} | {:<20} | {:<15}", "Node", "Peer ID", "Total Blocks");
        println!("{:-<23}|{:-<22}|{:-<16}", "", "", "");

        for node in &nodes {
            match node.fetch_status().await {
                Ok(status) => {
                    let peer_id = status["peer_id"].as_str().unwrap_or("N/A");
                    let blocks = status["total_blocks"].as_u64().unwrap_or(0);
                    
                    let short_peer_id = if peer_id.len() > 12 {
                        format!("...{}", &peer_id[peer_id.len() - 12..])
                    } else {
                        peer_id.to_string()
                    };
                    
                    println!("{:<22} | {:<20} | {:<15}", node.name, short_peer_id, blocks);
                }
                Err(e) => {
                    println!("{:<22} | Error fetching API: {}", node.name, e);
                }
            }
        }
        println!("\nLast updated: {}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"));
    }
}
