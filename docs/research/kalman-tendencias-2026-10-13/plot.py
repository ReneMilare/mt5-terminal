"""Comparação visual em sessões escolhidas antes dos resultados; requer matplotlib."""
from pathlib import Path
import csv
import datetime as dt
import os

os.environ.setdefault("MPLCONFIGDIR", "/tmp/kalman-tendencias-matplotlib")
import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
from matplotlib.collections import LineCollection
from matplotlib.lines import Line2D
from matplotlib.patches import Rectangle, Patch

HERE = Path(__file__).resolve().parent
RESULTS = HERE / "empirico" / "results"
OUTPUT = HERE / "figuras"
OUTPUT.mkdir(exist_ok=True)
COLORS = {0: "#17805c", 1: "#c44343", 2: "#c7992c"}
plt.rcParams.update({
    "font.family": "DejaVu Sans", "font.size": 9,
    "axes.spines.top": False, "axes.spines.right": False,
    "figure.facecolor": "#faf9f5", "axes.facecolor": "#faf9f5",
    "axes.labelcolor": "#333333", "text.color": "#242424",
})


def read_series(symbol, kernel):
    rows = list(csv.DictReader((RESULTS / f"series_{symbol}_{kernel}.csv").open()))
    return {int(r["server_time"]): r for r in rows}


def date_of(row):
    return dt.datetime.fromtimestamp(int(row["ny_time"]), dt.timezone.utc).date()


fig = plt.figure(figsize=(16, 9), layout="constrained")
grid = fig.add_gridspec(2, 3)
for row, symbol in enumerate(["UsaTec", "UsaInd"]):
    baseline = read_series(symbol, "baseline_exact")
    proposed = read_series(symbol, "innovation_default_g2")
    for col, day in enumerate([5, 6, 7]):
        date = dt.date(2026, 10, day)
        times = [t for t, r in proposed.items() if date_of(r) == date]
        assert times and all(t in baseline for t in times)
        values = [proposed[t] for t in times]
        sub = grid[row, col].subgridspec(4, 1, height_ratios=[12, 0.7, 0.7, 0.7], hspace=0.04)
        ax = fig.add_subplot(sub[0])
        current_ax = fig.add_subplot(sub[1], sharex=ax)
        m5_ax = fig.add_subplot(sub[2], sharex=ax)
        h1_ax = fig.add_subplot(sub[3], sharex=ax)
        low = min(float(r["low"]) for r in values)
        high = max(float(r["high"]) for r in values)
        padding = (high - low) * 0.05
        for i, r in enumerate(values):
            o, h, l, c = [float(r[k]) for k in ["open", "high", "low", "close"]]
            color = "#a2a9ae" if c >= o else "#656f77"
            ax.plot([i, i], [l, h], color=color, linewidth=0.6, alpha=0.8)
            ax.add_patch(Rectangle((i - 0.29, min(o, c)), 0.58,
                                   max(abs(c - o), (high - low) * 0.001),
                                   facecolor=color, edgecolor=color, linewidth=0.4, alpha=0.65))
        ax.plot(range(len(values)), [float(baseline[t]["level"]) for t in times],
                color="#7c6d93", linewidth=1.15, linestyle="--", alpha=0.95)
        segments = [[(i - 1, float(values[i - 1]["level"])),
                     (i, float(values[i]["level"]))] for i in range(1, len(values))]
        ax.add_collection(LineCollection(segments, colors=[COLORS[int(r["M5"])]
                                                          for r in values[1:]], linewidths=2.0))
        ax.set(title=f"{symbol} · {day:02d}/10/2026", ylabel="Preço", ylim=(low - padding, high + padding))
        ax.grid(axis="y", linewidth=0.5, alpha=0.18)
        ax.tick_params(axis="x", labelbottom=False, bottom=False)
        for ribbon, key, series, label in [(current_ax, "M5", [baseline[t] for t in times], "Atual"),
                                           (m5_ax, "M5", values, "M5"),
                                           (h1_ax, "H1", values, "H1")]:
            ribbon.bar(range(len(values)), [1] * len(values), width=1.0,
                       color=[COLORS[int(r[key])] for r in series], edgecolor="none")
            ribbon.set(ylim=(0, 1), yticks=[], ylabel=label, xlim=(-1, len(values)))
            ribbon.yaxis.label.set_rotation(0)
            ribbon.yaxis.label.set_fontsize(8)
            ribbon.yaxis.label.set_horizontalalignment("right")
            ribbon.yaxis.label.set_verticalalignment("center")
            ribbon.yaxis.set_label_coords(-0.012, 0.5)
            for spine in ribbon.spines.values():
                spine.set_visible(False)
        m5_ax.tick_params(axis="x", labelbottom=False, bottom=False)
        current_ax.tick_params(axis="x", labelbottom=False, bottom=False)
        ticks = list(range(0, len(values), 12))
        labels = [dt.datetime.fromtimestamp(int(values[i]["ny_time"]), dt.timezone.utc).strftime("%H:%M")
                  for i in ticks]
        h1_ax.set(xticks=ticks, xticklabels=labels, xlabel="Horário Nova York")
        h1_ax.tick_params(axis="x", labelsize=8, length=0)
fig.suptitle("Kalman Trend no M5 | comparação em três sessões predefinidas\n"
             "Proposta: inovações g2, memória 20, limite 4, entrada 1, saída 0 · H1 nas faixas", fontsize=15)
fig.legend(handles=[Line2D([0], [0], color="#7c6d93", linestyle="--", label="Configuração em uso"),
                    Line2D([0], [0], color="#17805c", linewidth=2, label="Linha proposta: alta"),
                    Line2D([0], [0], color="#c44343", linewidth=2, label="Baixa"),
                    Patch(facecolor="#c7992c", label="Neutro")],
           loc="outside lower center", ncol=4, frameon=False)
fig.savefig(OUTPUT / "comparacao-sessoes.png", dpi=150)
fig.savefig(OUTPUT / "comparacao-sessoes.svg")
plt.close(fig)
print("Gerados figuras/comparacao-sessoes.png e .svg")
