#!/usr/bin/env python3
"""Builds the paper's figures from the colour-bench outputs. Run from `paper/`.

    python3 scripts/make_figures.py

Writes generated/fig_beta.pdf: symbol decode rate (with 95% Wilson band) and pre-correction
bit error rate against the leak fraction beta, both datasets, RDA shrink 0.5.
"""
import argparse
import csv

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt  # noqa: E402

# Categorical slots 1 and 2 of the validated reference palette; markers carry identity in
# grayscale print.
SERIES = [
    ("CUHK-CQRC", "sweep_hiq.csv", "#2a78d6", "o"),
    ("qrism", "sweep_qrism.csv", "#eb6834", "s"),
]
INK = "#0b0b0b"
INK_2 = "#52514e"
GRID = "#d9d8d4"
CHOSEN = 0.06


def series(path):
    rows = [r for r in csv.DictReader(open(path)) if r["group_kind"] == "all" and r["setting"].startswith("b=")]
    rows.sort(key=lambda r: float(r["beta"]))
    beta = [float(r["beta"]) for r in rows]
    return (
        beta,
        [float(r["code_pct"]) for r in rows],
        [float(r["code_ci_lo"]) for r in rows],
        [float(r["code_ci_hi"]) for r in rows],
        [float(r["ber_pct"]) for r in rows],
    )


def style(ax):
    for side in ("top", "right"):
        ax.spines[side].set_visible(False)
    for side in ("left", "bottom"):
        ax.spines[side].set_color(INK_2)
        ax.spines[side].set_linewidth(0.6)
    ax.tick_params(colors=INK_2, width=0.6, length=2.5, labelsize=7)
    ax.grid(axis="y", color=GRID, linewidth=0.5)
    ax.set_axisbelow(True)
    ax.axvline(CHOSEN, color=INK_2, linewidth=0.7, linestyle=(0, (3, 2)), zorder=0)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bench", default="../benches/color")
    ap.add_argument("--out", default="generated/fig_beta.pdf")
    a = ap.parse_args()

    plt.rcParams.update({"font.family": "serif", "font.size": 8, "pdf.fonttype": 42})
    fig, (top, bot) = plt.subplots(2, 1, figsize=(3.45, 3.4), sharex=True, gridspec_kw={"hspace": 0.12})

    for name, fname, color, marker in SERIES:
        beta, code, lo, hi, ber = series(f"{a.bench}/{fname}")
        top.fill_between(beta, lo, hi, color=color, alpha=0.15, linewidth=0)
        top.plot(beta, code, color=color, linewidth=1.2, marker=marker, markersize=3, label=name)
        bot.plot(beta, ber, color=color, linewidth=1.2, marker=marker, markersize=3, label=name)
        # direct labels at the right end, in ink rather than the series colour
        top.annotate(name, (beta[-1], code[-1]), xytext=(4, 0), textcoords="offset points",
                     va="center", fontsize=7, color=INK)

    style(top)
    style(bot)
    top.set_ylabel("Symbols decoded (%)", color=INK)
    bot.set_ylabel("Bit error rate (%)", color=INK)
    bot.set_xlabel(r"Leak fraction $\beta$", color=INK)
    top.set_ylim(bottom=0)
    top.set_xlim(-0.003, 0.172)
    top.annotate(r"$\beta=0.06$", (CHOSEN, top.get_ylim()[1]), xytext=(3, -8),
                 textcoords="offset points", fontsize=7, color=INK_2)
    bot.legend(frameon=False, fontsize=7, loc="upper center", ncol=2, handlelength=1.6)

    fig.savefig(a.out, bbox_inches="tight", pad_inches=0.02)
    fig.savefig(a.out.replace(".pdf", ".png"), dpi=200, bbox_inches="tight", pad_inches=0.02)
    print(f"wrote {a.out}")


if __name__ == "__main__":
    main()
