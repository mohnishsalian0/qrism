# Paper

LaTeX source for the qrism colour-recovery paper (IEEE conference template).

## Build

Numbers, tables and figures are generated from the colour-bench outputs, never typed by hand:

```sh
cd paper
python3 scripts/make_tables.py     # generated/numbers.tex and generated/tab_*.tex
python3 scripts/make_figures.py    # generated/fig_beta.pdf (needs matplotlib)
latexmk -pdf main.tex
```

Inputs:

- `../benches/color/`: `results.txt` and `accuracy_hiq_photos.csv` from `-- accuracy`,
  `qrism_photos.csv` from `-- qrism` (with `MODULE_SIZE = None`), `sweep*.csv` and
  `sweep.txt` from `-- sweep`, `shrink*.csv` from `-- shrink`, `timing.txt` from `-- timing`, all via
  `cargo bench --features benchmark --bench color -- <pass>`.
- `../../hiq/HiQ/hiq-decoder/bench/results/full.csv`: the HiQ decoder's per-photo results.
- `data/qrism_dataset_counts.csv`: `python3 scripts/count_dataset.py > data/qrism_dataset_counts.csv`.
- `data/cuhk_cqrc_split.csv`: CUHK-CQRC metadata with HiQ's test split,
  `python3 scripts/cuhk_split.py > data/cuhk_cqrc_split.csv` (needs scipy).

Both data scripts need the datasets under `../benches/dataset/high_capacity/`.

The script checks that every pass localized the same photos and that the paper's claims
hold (for example that every baseline decodes fewer symbols than ours); it stops with an
error if not.

Open items are marked `\todo{...}` and print in red; `grep -n todo sections/*.tex` lists them.
