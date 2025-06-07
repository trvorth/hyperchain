import React, { useState, useEffect } from 'react';
import axios from 'axios';
import './Dashboard.css';

const Dashboard = () => {
  const [minerStats, setMinerStats] = useState([]);
  const [balance, setBalance] = useState(0);
  const [walletAddress, setWalletAddress] = useState('');
  const [privateKey, setPrivateKey] = useState('');
  const [error, setError] = useState(null);

  const fetchInfo = async () => {
    try {
      const response = await axios.get('http://127.0.0.1:9001/info');
      setWalletAddress(response.data.wallet_address);
      setError(null);
      return response.data.wallet_address;
    } catch (err) {
      setError('Failed to fetch node info');
      console.error(err);
      return null;
    }
  };

  const fetchBalance = async (address) => {
    if (!address) return;
    try {
      const response = await axios.get(`http://127.0.0.1:9001/balance/${address}`);
      setBalance(response.data);
      setError(null);
    } catch (err) {
      setError('Failed to fetch wallet balance');
      console.error(err);
    }
  };

  const fetchMinerStats = async () => {
    try {
      const response = await axios.get('http://127.0.0.1:9001/miner_stats');
      setMinerStats(response.data);
      setError(null);
    } catch (err) {
      setError('Failed to fetch miner stats');
      console.error(err);
    }
  };

  const handleSetWallet = async () => {
    if (!privateKey) {
      setError('Please enter a private key');
      return;
    }
    try {
      const response = await axios.post('http://127.0.0.1:9001/set_wallet', {
        private_key: privateKey,
      });
      setWalletAddress(response.data);
      setPrivateKey('');
      setError(null);
      fetchBalance(response.data);
      console.log('Wallet updated:', response.data);
    } catch (err) {
      setError('Failed to set wallet');
      console.error(err);
    }
  };

  useEffect(() => {
    const fetchData = async () => {
      const address = await fetchInfo();
      await fetchBalance(address);
      await fetchMinerStats();
    };
    fetchData();
    const interval = setInterval(fetchData, 5000); // Poll every 5 seconds
    return () => clearInterval(interval);
  }, []);

  const handleSubmitShare = async () => {
    try {
      const response = await axios.post('http://127.0.0.1:9001/submit_share', {
        address: 'test_miner',
        shares: 1,
        hashrate: 1000,
        last_share_time: Math.floor(Date.now() / 1000),
      });
      console.log(response.data); // "Share accepted"
      fetchMinerStats(); // Refresh stats
    } catch (err) {
      setError('Failed to submit test share');
      console.error(err);
    }
  };

  const formatTimestamp = (timestamp) => {
    if (timestamp === 0) return '01/01/1970, 08:00:00';
    const date = new Date(timestamp * 1000);
    return date.toLocaleDateString('en-GB') + ', ' + date.toLocaleTimeString('en-GB');
  };

  return (
    <div className="p-6">
      <h2 className="text-2xl font-bold mb-4">Dashboard</h2>
      {error && <p className="text-red-500 mb-4">{error}</p>}

      <div className="mb-6">
        <h3 className="text-xl font-semibold">Wallet</h3>
        <p className="text-lg">Address: {walletAddress || 'Loading...'}</p>
        <p className="text-lg">Balance: {balance} coins</p>
        <div className="mt-2">
          <input
            type="text"
            value={privateKey}
            onChange={(e) => setPrivateKey(e.target.value)}
            placeholder="Enter private key"
            className="border border-gray-300 p-2 rounded w-full max-w-md"
          />
          <button
            onClick={handleSetWallet}
            className="mt-2 bg-blue-500 text-white px-4 py-2 rounded hover:bg-blue-600"
          >
            Set Wallet
          </button>
        </div>
      </div>

      <div className="mb-6">
        <h3 className="text-xl font-semibold">Miner Stats</h3>
        {minerStats.length === 0 ? (
          <p>No miners connected</p>
        ) : (
          <table className="w-full border-collapse border border-gray-300">
            <thead>
              <tr className="bg-gray-100">
                <th className="border border-gray-300 p-2">Address</th>
                <th className="border border-gray-300 p-2">Shares</th>
                <th className="border border-gray-300 p-2">Hashrate (H/s)</th>
                <th className="border border-gray-300 p-2">Last Share Time</th>
              </tr>
            </thead>
            <tbody>
              {minerStats.map((stat) => (
                <tr key={stat.address}>
                  <td className="border border-gray-300 p-2">{stat.address}</td>
                  <td className="border border-gray-300 p-2">{stat.shares}</td>
                  <td className="border border-gray-300 p-2">{stat.hashrate.toFixed(2)}</td>
                  <td className="border border-gray-300 p-2">{formatTimestamp(stat.last_share_time)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>

      <button
        onClick={handleSubmitShare}
        className="bg-blue-500 text-white px-4 py-2 rounded hover:bg-blue-600"
      >
        Submit Test Share
      </button>
    </div>
  );
};

export default Dashboard;
