function MiningGuide() {
    return (
        <div className="bg-white p-6 rounded-lg shadow">
            <h2 className="text-2xl font-bold mb-4">Hyper Mining Guide</h2>
            <h3 className="text-xl font-semibold mb-2">1. Set Up a Hyper Node</h3>
            <p>Follow these steps to run a Hyper mainnet node:</p>
            <ul className="list-disc pl-6 mb-4">
                <li>Install Rust: <code>curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh</code></li>
                <li>Clone the Hyperchain repository: <code>git clone https://github.com/hyperchain/hyperchain.git</code></li>
                <li>Navigate to the directory: <code>cd hyperchain</code></li>
                <li>Build the node: <code>cargo build --release</code></li>
                <li>Edit <code>config.toml</code> to set <code>p2p_address</code> and <code>api_address</code>.</li>
                <li>Run the node: <code>RUST_LOG=info cargo run --release --bin hyperdag</code></li>
            </ul>
            <h3 className="text-xl font-semibold mb-2">2. Solo Mining</h3>
            <p>To mine solo, use the node’s built-in miner:</p>
            <ul className="list-disc pl-6 mb-4">
                <li>Generate a wallet: <code>cargo run --bin wallet -- generate</code></li>
                <li>Save the private key to a file (e.g., <code>key.txt</code>).</li>
                <li>Run the node with mining enabled (default in <code>config.toml</code>).</li>
                <li>Monitor mining progress via logs or the web interface.</li>
            </ul>
            <h3 className="text-xl font-semibold mb-2">3. Join a Mining Pool</h3>
            <p>Join a Hyper mining pool for better reward consistency:</p>
            <ul className="list-disc pl-6 mb-4">
                <li>Find a pool at <a href="https://pools.hyperchain.org" className="text-blue-600">pools.hyperchain.org</a>.</li>
                <li>Install a mining client compatible with Hyper (e.g., HyperMiner).</li>
                <li>Configure the client with the pool’s address and your wallet address.</li>
                <li>Start mining and submit shares to the pool.</li>
            </ul>
            <h3 className="text-xl font-semibold mb-2">4. Tips for Success</h3>
            <ul className="list-disc pl-6">
                <li>Use a stable internet connection.</li>
                <li>Monitor your hashrate and shares on this dashboard.</li>
                <li>Join the Hyper community on Discord for support.</li>
            </ul>
        </div>
    );
}

export default MiningGuide;
