
import React, { useState, useEffect } from 'react';
import axios from 'axios';
import './App.css';
import Dashboard from './components/Dashboard';
import MiningGuide from './components/MiningGuide';

const App = () => {
    const [blockchain, setBlockchain] = useState(null);
    const [activeTab, setActiveTab] = useState('dashboard');

    useEffect(() => {
        const fetchBlockchain = async () => {
            try {
                // In a real application, you would fetch this from your backend API
                // For now, we'll use mock data
                const mockBlockchain = {
                    blocks: [
                        { index: 0, timestamp: Date.now(), transactions: [], previous_hash: '0', hash: '0' }
                    ],
                    pending_transactions: [],
                    difficulty: 2
                };
                setBlockchain(mockBlockchain);
            } catch (error) {
                console.error("Error fetching blockchain data:", error);
            }
        };

        fetchBlockchain();
    }, []);

    return (
        <div className="min-h-screen bg-gray-900 text-white">
            <header className="bg-gray-800 p-4 shadow-lg">
                <h1 className="text-3xl font-bold text-center text-teal-400">Hyperchain Dashboard</h1>
                <nav className="flex justify-center mt-4">
                    <button
                        className={`px-4 py-2 mx-2 rounded-lg transition-colors duration-300 ${activeTab === 'dashboard' ? 'bg-teal-500' : 'bg-gray-700 hover:bg-gray-600'}`}
                        onClick={() => setActiveTab('dashboard')}
                    >
                        Dashboard
                    </button>
                    <button
                        className={`px-4 py-2 mx-2 rounded-lg transition-colors duration-300 ${activeTab === 'mining' ? 'bg-teal-500' : 'bg-gray-700 hover:bg-gray-600'}`}
                        onClick={() => setActiveTab('mining')}
                    >
                        Mining Guide
                    </button>
                </nav>
            </header>
            <main className="p-4">
                {activeTab === 'dashboard' ? (
                    <Dashboard blockchain={blockchain} />
                ) : (
                    <MiningGuide />
                )}
            </main>
        </div>
    );
};

export default App;
