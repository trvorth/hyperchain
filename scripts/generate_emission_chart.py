import matplotlib.pyplot as plt
import numpy as np
import os
import matplotlib as mpl

# Configure professional whitepaper style
mpl.rcParams.update({
    'font.family': 'serif',
    'font.size': 10,
    'axes.titlesize': 12,
    'axes.labelsize': 10,
    'xtick.labelsize': 9,
    'ytick.labelsize': 9,
    'axes.titleweight': 'bold',
    'axes.labelweight': 'bold',
    'grid.color': '#e0e0e0',
    'grid.linestyle': '--',
    'grid.linewidth': 0.7,
    'figure.dpi': 300
})

# Emission curve data
years = np.array([0, 2, 4, 8, 16])
coins = np.array([250, 122.5, 60, 14.5, 0.85])

# Create figure
fig, ax = plt.subplots(figsize=(6, 3.5))

# Plot with logarithmic scale
ax.semilogy(years, coins, 'o-', color='#2c6fbb', linewidth=1.5,
            markersize=5, markerfacecolor='white', markeredgewidth=1.2)

# Annotate data points
for i, (x, y) in enumerate(zip(years, coins)):
    va = 'bottom' if y > 10 else 'top'
    offset = 10 if y > 10 else -15
    ax.annotate(f"{y:.1f}" if y > 1 else f"{y:.2f}",
               (x, y),
               xytext=(0, offset),
               textcoords='offset points',
               fontsize=8,
               ha='center',
               va=va,
               bbox=dict(boxstyle="round,pad=0.2", fc="white", ec="none", alpha=0.8))

# Formatting
ax.set_title("HyperCoin (HCN) Emission Schedule", pad=12)
ax.set_xlabel("Time (Years)")
ax.set_ylabel("New Coins per Checkpoint")
ax.set_xticks(years)
ax.grid(True, alpha=0.4)
ax.set_axisbelow(True)

# Create output directory
output_dir = "docs/whitepaper/assets"
os.makedirs(output_dir, exist_ok=True)

# Save files
plt.tight_layout(pad=1.0)
plt.savefig(f"{output_dir}/hcn_emission_curve.png", dpi=300)
plt.savefig(f"{output_dir}/hcn_emission_curve.pdf")
plt.savefig(f"{output_dir}/hcn_emission_curve.svg")
print(f"✅ Emission charts saved to {output_dir}/")
