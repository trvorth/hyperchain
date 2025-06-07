import { useState, useEffect } from 'react';
import Dashboard from './components/Dashboard.jsx';
import MiningGuide from './components/MiningGuide.jsx';
import './App.css';

function App() {
    const [activeTab, setActiveTab] = useState('dashboard');

    return (
        <div className="min-h-screen bg-gray-100">
            <nav className="bg-blue-600 p-4">
                <div className="container mx-auto flex justify-between items-center">
                    <h1 className="text-white text-2xl font-bold">Hyper Mainnet</h1>
                    <div>
                        <button
                            className={`text-white px-4 py-2 ${activeTab === 'dashboard' ? 'bg-blue-800' : ''}`}
                            onClick={() => setActiveTab('dashboard')}
                        >
                            Dashboard
                        </button>
                        <button
                            className={`text-white px-4 py-2 ${activeTab === 'guide' ? 'bg-blue-800' : ''}`}
                            onClick={() => setActiveTab('guide')}
                        >
                            Mining Guide
                        </button>
                    </div>
                </div>
            </nav>
            <div className="container mx-auto p-4">
                {activeTab === 'dashboard' ? <Dashboard /> : <MiningGuide />}
            </div>
        </div>
    );
}

export default App;
