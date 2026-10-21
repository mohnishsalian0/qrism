#!/usr/bin/env python3
"""Writes the CUHK-CQRC metadata as a CSV, so the paper can use HiQ's own test split. Run
from `paper/` on a machine that has the dataset (needs scipy):

    python3 scripts/cuhk_split.py > data/cuhk_cqrc_split.csv

`properties.mat`, shipped with the dataset, has one record per image. Its `isactive` and
`istraining` flags mark HiQ's split: active and not training is the test set the HiQ paper
evaluates on; active and training are the 65 images its models were trained on.
"""
import csv
import os
import sys

import scipy.io

ROOT = "../benches/dataset/high_capacity/hiq"


def main():
    props = scipy.io.loadmat(f"{ROOT}/properties.mat", squeeze_me=True, struct_as_record=False)
    by_name = {str(e.filename).lower(): e for e in props["properties"]}
    w = csv.writer(sys.stdout)
    w.writerow(["folder", "file", "type", "device", "lighting", "print_resolution", "size", "version",
                "isactive", "istraining"])
    for folder in sorted(os.listdir(ROOT)):
        d = os.path.join(ROOT, folder)
        if folder == "source" or not os.path.isdir(d):
            continue
        for f in sorted(os.listdir(d)):
            if not f.lower().endswith((".jpg", ".jpeg", ".png")):
                continue
            e = by_name[f.lower()]
            w.writerow([folder, f, e.type, e.capture_device, e.lighting_condition, e.print_resolution,
                        e.size, e.version, int(e.isactive), int(e.istraining)])


if __name__ == "__main__":
    main()
