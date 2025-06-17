use reqwest::Client;
use std::time::Duration;
use tokio::time;

struct Node {
    name: String,
    api_url: String,
    client: Client,
}

impl Node {
    fn new(name: &str, url: &str) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(4)) // Set a request timeout
            .build()
            .expect("Failed to build reqwest client");

        Node {
            name: name.to_string(),
            api_url: url.to_string(),
            client,
        }
    }

    async fn fetch_status(&self) -> Result<serde_json::Value, reqwest::Error> {
        self.client.get(&self.api_url).send().await?.json().await
    }
}

#[tokio::main]
async fn main() {
    // SECURITY NOTE: In a production environment, use HTTPS endpoints.
    let nodes = vec![
        Node::new("Node 1 (Asia)", "https://34.126.103.74:8080/status"),
        Node::new("Node 2 (US)", "https://104.197.0.155:8080/status"),
        Node::new("Node 3 (EU)", "https://35.195.207.69:8080/status"),
    ];

    let mut interval = time::interval(Duration::from_secs(5));

    loop {
        interval.tick().await;
        print!("\x1B[2J\x1B[1;1H"); // Clear screen and move cursor to top-left

        println!("--- HyperChain Global Testnet Monitor (API) ---");
        println!(
            "{:<22} | {:<20} | {:<15}",
            "Node", "Peer ID", "Total Blocks"
        );
        println!("{:-<23}|{:-<22}|{:-<16}", "", "", "");

        for node in &nodes {
            match node.fetch_status().await {
                Ok(status) => {
                    let peer_id = status["peer_id"].as_str().unwrap_or("N/A");
                    let blocks = status["total_blocks"].as_u64().unwrap_or(0);

                    // Display a shortened, more readable version of the peer ID
                    let short_peer_id = if peer_id.len() > 12 {
                        format!("...{}", &peer_id[peer_id.len() - 12..])
                    } else {
                        peer_id.to_string()
                    };

                    println!("{:<22} | {:<20} | {:<15}", node.name, short_peer_id, blocks);
                }
                Err(e) => {
                    // Provide more specific error feedback
                    let error_msg = if e.is_timeout() {
                        "Request timed out".to_string()
                    } else if e.is_connect() {
                        "Connection refused".to_string()
                    } else {
                        "API fetch error".to_string()
                    };
                    println!("{:<22} | Error: {}", node.name, error_msg);
                }
            }
        }
        println!(
            "\nLast updated: {}",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
        );
    }
}
