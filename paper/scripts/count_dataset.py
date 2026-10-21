#!/usr/bin/env python3
"""Counts the qrism dataset's photos by phone, capture condition and module size. Run from
`paper/` on a machine that has the dataset:

    python3 scripts/count_dataset.py > data/qrism_dataset_counts.csv

The dataset itself is too large for the repository, so the paper reads these counts instead.
Layout: benches/dataset/high_capacity/qrism/{phone}/{condition}/{version}-{ec}-{px}_{shot}.png
"""
import csv
import os
import re
import sys
from collections import Counter

ROOT = "../benches/dataset/high_capacity/qrism"
NAME = re.compile(r"^(\d+)-([LMQH])-(\d+)_(\d+)\.(png|jpe?g)$", re.I)


def main():
    counts = Counter()
    for phone in sorted(os.listdir(ROOT)):
        pdir = os.path.join(ROOT, phone)
        if phone == "source" or not os.path.isdir(pdir):
            continue
        for cond in sorted(os.listdir(pdir)):
            cdir = os.path.join(pdir, cond)
            if not os.path.isdir(cdir):
                continue
            for f in os.listdir(cdir):
                m = NAME.match(f)
                if m:
                    counts[(phone, cond, int(m.group(3)))] += 1
    w = csv.writer(sys.stdout)
    w.writerow(["phone", "condition", "module_px", "photos"])
    for (phone, cond, px), n in sorted(counts.items()):
        w.writerow([phone, cond, px, n])


if __name__ == "__main__":
    main()
