#!/usr/bin/env python3
"""Builds the paper's tables and inline numbers from the colour-bench outputs.

Every number in the paper comes from here, so a rerun of the benches followed by

    python3 scripts/make_tables.py

regenerates `generated/` and nothing is typed by hand. Run from `paper/`.

Inputs (paths relative to `paper/`, overridable with flags):
  ../benches/color/results.txt          accuracy pass (HiQ dataset, all pipelines)
  ../benches/color/sweep_{hiq,qrism}.csv            beta sweep rates
  ../benches/color/sweep_{hiq,qrism}_photos.csv     beta sweep per photo
  ../benches/color/shrink_{hiq,qrism}.csv           shrink x beta grid rates
  ../benches/color/shrink_{hiq,qrism}_photos.csv    shrink x beta grid per photo
  ../benches/color/accuracy_hiq_photos.csv          accuracy pass per photo and pipeline
  ../benches/color/qrism_photos.csv                 qrism report per photo and pipeline
  ../../hiq/HiQ/hiq-decoder/bench/results/full.csv  HiQ decoder per photo
  data/cuhk_cqrc_split.csv                          CUHK-CQRC metadata (HiQ's test split)
  ../benches/color/timing.txt                       timing pass (per-module cost)
  ../benches/color/hiq_parity/parity.txt            HiQ classifier ports against the HiQ decoder
  ../../hiq/HiQ/hiq-decoder/bench/results/timing.txt  HiQ bench timing, same machine

Outputs, under `generated/`:
  numbers.tex             \\newcommand macros for every number quoted in the text
  tab_baselines.tex       colour-recovery pipelines on both datasets
  tab_shrink.tex          RDA shrink at beta 0 and 0.06, both datasets
  tab_breakdown.tex       qrism decode rate by module size and capture condition
  tab_paired.tex          paired comparison with the HiQ decoder
  tab_ported.tex          HiQ's classifiers on our samples (classifier-only comparison)
"""
import argparse
import csv
import math
import os
import re
from collections import defaultdict
from types import SimpleNamespace

CHOSEN = "b=0.06"  # sweep label of the chosen setting
CHOSEN_GRID = "b=0.06 s=0.5"  # same setting in the shrink grid
NODEBLUR = "b=0.00"


# Statistics
# ------------------------------------------------------------------------------


def wilson(k, n, z=1.96):
    if n == 0:
        return 0.0, 0.0
    p = k / n
    d = 1 + z * z / n
    c = (p + z * z / (2 * n)) / d
    h = z * math.sqrt(p * (1 - p) / n + z * z / (4 * n * n)) / d
    return 100 * (c - h), 100 * (c + h)


def mcnemar(b, c):
    """Exact two-sided McNemar p from the discordant counts."""
    n = b + c
    if n == 0:
        return 1.0
    k = min(b, c)
    # log-space sum keeps large n from underflowing
    terms = []
    lc = 0.0
    for i in range(k + 1):
        if i:
            lc += math.log(n - i + 1) - math.log(i)
        terms.append(lc - n * math.log(2))
    m = max(terms)
    return min(1.0, 2 * math.exp(m) * sum(math.exp(t - m) for t in terms))


def p_exp(p):
    """The smallest power of ten above p, as an exponent."""
    return math.floor(math.log10(p)) + 1 if p > 0 else -300


def fmt_p(p):
    """p as the right-hand side of a LaTeX relation, for use as `$p\\Macro$`: `=0.294` above
    0.001, otherwise an order-of-magnitude bound such as `<10^{-65}`."""
    if p >= 0.001:
        return f"={p:.3f}"
    return f"<10^{{{p_exp(p)}}}"


# Loading
# ------------------------------------------------------------------------------


def rates(path):
    """Rates CSV keyed by (group_kind, group, setting)."""
    out = {}
    for r in csv.DictReader(open(path)):
        out[(r["group_kind"], r["group"], r["setting"])] = r
    return out


def photos(path):
    return list(csv.DictReader(open(path)))


def discordant(rows, a, b):
    x = sum(1 for r in rows if r[a] == "1" and r[b] == "0")
    y = sum(1 for r in rows if r[a] == "0" and r[b] == "1")
    return x, y


def accuracy_pass(path):
    """The summary table of the accuracy pass: pipeline -> (data%, layer%, code%)."""
    text = open(path).read()
    body = text.split("module accuracy and layer decode by pipeline")[1].split("layer decode failures")[0]
    out = {}
    for line in body.splitlines():
        m = re.match(r"\s{4}(\S.*?)\s{2,}([\d.]+)\s+([\d.]+)\s+([+-][\d.]+)\s+([\d.]+)\s+([\d.]+)\s", line)
        if m:
            out[m.group(1)] = (float(m.group(2)), float(m.group(5)), float(m.group(6)))
    photos_m = re.search(r"photos: (\d+) attempted, (\d+) scored \| skipped: (\d+) no symbol", text)
    m = re.search(r"localized at a version with no reference \(misread\):(.*)", text)
    misread = sum(int(n) for n in re.findall(r"v\d+x(\d+)", m.group(1))) if m else 0
    accuracy_pass.no_symbol, accuracy_pass.misread = int(photos_m.group(3)), misread
    return out, int(photos_m.group(1)), int(photos_m.group(2))


# Output helpers
# ------------------------------------------------------------------------------


class Numbers:
    """Collects \\newcommand macros. Names are letters only, as LaTeX requires."""

    def __init__(self):
        self.lines = []

    def add(self, name, value):
        assert re.fullmatch(r"[A-Za-z]+", name), name
        self.lines.append(f"\\newcommand{{\\{name}}}{{{value}}}")

    def write(self, path):
        with open(path, "w") as f:
            f.write("% Generated by scripts/make_tables.py. Do not edit.\n")
            f.write("\n".join(self.lines) + "\n")


def f1(x):
    return f"{float(x):.1f}"


def f2(x):
    return f"{float(x):.2f}"


def write(path, text):
    with open(path, "w") as f:
        f.write("% Generated by scripts/make_tables.py. Do not edit.\n" + text)


# Tables
# ------------------------------------------------------------------------------

# (paper label, accuracy-pass / qrism-report pipeline name or None, family). The chosen
# setting is added last from the sweep (CUHK-CQRC) and the qrism report (qrism). The qrism
# report has no optical-beta run, so that cell comes from the beta sweep (SWEEP_OPTICAL).
# Rows of family "ours" are variants of our method, not baselines: they are left out of the
# "ours decodes more than every baseline" check and of its p-value.
SWEEP_OPTICAL = "sweep:optical"
BASELINES = [
    ("Nearest centroid", "black/white euclid", "black/white euclid", "prior"),
    ("Interference cancellation", "per-colorant int + adp", "per-colorant int + adp", "prior"),
    ("JAB Code classifier", "jabcode (balanced)", "jabcode (balanced)", "prior"),
    ("RDA, no leak inversion", "mahalanobis rda (raw)", "mahalanobis rda (raw)", "da"),
    ("Ours, $\\beta$ per photograph", "rda + deblur optical", SWEEP_OPTICAL, "ours"),
]
CENTROID = "black/white euclid"  # the nearest-centroid baseline cited in the text
QRISM_CHOSEN = "rda + deblur b=.06"  # the chosen setting's name in the qrism report


def long_photos(path):
    """A long per-photo CSV (one row per photo and pipeline) as {pipeline: {path: row}},
    localized photos only."""
    out = defaultdict(dict)
    for r in csv.DictReader(open(path)):
        if r["localized"] == "1":
            out[r["pipeline"]][r["path"]] = r
    return out


class Stat:
    """Pooled module accuracy, bit error rate and decode rate over a set of photos."""

    def __init__(self, rows):
        rows = list(rows)
        self.n = len(rows)
        self.decoded = sum(int(r["decoded"]) for r in rows)
        dt = sum(int(r["data_total"]) for r in rows)
        self.data = 100 * sum(int(r["data_correct"]) for r in rows) / max(dt, 1)
        be = sum(int(r[f"bit_err_{c}"]) for r in rows for c in "rgb")
        self.ber = 100 * be / max(3 * dt, 1)
        self.code = 100 * self.decoded / max(self.n, 1)


def decoded_by_path(rows_by_path):
    return {p: r["decoded"] == "1" for p, r in rows_by_path.items()}


def tab_baselines(acc_ph, q_ph, sw_h, ph_h, sw_q, ph_q):
    """Color recovery pipelines on both datasets, every row over the same localized photos
    of its dataset. Returns the table and the largest McNemar p of any baseline against the
    chosen setting, for the caption and the text."""
    ours_h = {r["path"]: r[CHOSEN] == "1" for r in ph_h}
    ours_q = decoded_by_path(q_ph[QRISM_CHOSEN])
    worst = 0.0
    rows, prev = [], None
    stats_h, stats_q = {}, {}

    def cell(ph, name, ours, stats, baseline):
        nonlocal worst
        if name is None:
            return "-- & --"
        if name == SWEEP_OPTICAL:
            # qrism optical beta: rates and per-photo decodes from the beta sweep
            r = sw_q[("all", "all", "optical")]
            s = SimpleNamespace(ber=float(r["ber_pct"]), code=float(r["code_pct"]))
            mine = {row["path"]: row["optical"] == "1" for row in ph_q}
            assert set(mine) == set(ours), "beta sweep and qrism report localized different photos"
        else:
            s = Stat(ph[name].values())
            mine = decoded_by_path(ph[name])
        if baseline:
            b = sum(ours[p] and not mine.get(p, False) for p in ours)
            c = sum(mine.get(p, False) and not ours[p] for p in ours)
            worst = max(worst, mcnemar(b, c))
            stats[name] = s
        return f"{s.ber:.2f} & {s.code:.1f}"

    for label, kh, kq, fam in BASELINES:
        if prev is not None and fam != prev:
            rows.append("\\midrule")
        prev = fam
        base = fam != "ours"
        rows.append(f"{label} & {cell(acc_ph, kh, ours_h, stats_h, base)} & {cell(q_ph, kq, ours_q, stats_q, base)} \\\\")
    a = sw_h[("all", "all", CHOSEN)]
    sq = Stat(q_ph[QRISM_CHOSEN].values())
    rows.append(
        f"\\textbf{{Ours, $\\beta=0.06$}} & \\textbf{{{f2(a['ber_pct'])}}} & "
        f"\\textbf{{{f1(a['code_pct'])}}} & \\textbf{{{sq.ber:.2f}}} & \\textbf{{{sq.code:.1f}}} \\\\"
    )
    # The caption claims ours beats every baseline on both datasets; check it rather than trust it.
    for stats, ours_code in ((stats_h, float(a["code_pct"])), (stats_q, sq.code)):
        for name, st in stats.items():
            assert st.code < ours_code, f"{name} decodes at least as many symbols as ours"
    assert worst < 0.05, "a baseline is not significantly worse than ours; fix the caption"
    n_h = len(ours_h)
    n_q = len(ours_q)
    body = "\n".join(rows)
    table = f"""\\begin{{table}}[t]
\\centering
\\small
\\caption{{Color recovery pipelines, all fed the same module samples: bit error rate before
Reed-Solomon correction and symbol decode rate, in percent, over the photographs localized
at the right version ({n_h} on CUHK-CQRC, {n_q} on qrism). The last block is our method with
$\\beta$ estimated per photograph from the blur of the finder edges and with the fixed
$\\beta$, which decodes more symbols than every row above that block, with exact McNemar
$p<10^{{{p_exp(worst)}}}$.}}
\\label{{tab:baselines}}
\\setlength{{\\tabcolsep}}{{4pt}}
\\begin{{tabular}}{{@{{}}lrrrr@{{}}}}
\\toprule
 & \\multicolumn{{2}}{{c}}{{CUHK-CQRC}} & \\multicolumn{{2}}{{c}}{{qrism}} \\\\
\\cmidrule(lr){{2-3}}\\cmidrule(l){{4-5}}
Pipeline & BER & Symbol & BER & Symbol \\\\
\\midrule
{body}
\\bottomrule
\\end{{tabular}}
\\end{{table}}
"""
    return table, worst, stats_h, stats_q, sq


def tab_shrink(sh_hiq, sh_q):
    lambdas = ["0.0", "0.2", "0.4", "0.5", "0.6", "0.8", "1.0"]
    rows = []
    for lam in lambdas:
        cells = []
        for g in (sh_hiq, sh_q):
            for beta in ("0.00", "0.06"):
                cells.append(f1(g[("all", "all", f"b={beta} s={lam}")]["code_pct"]))
        name = {"0.0": "0.0 (QDA)", "1.0": "1.0 (LDA)"}.get(lam, lam)
        if lam == "0.5":
            cells = [f"\\textbf{{{c}}}" for c in cells]
            name = "0.5 (used)"
        rows.append(f"{name} & " + " & ".join(cells) + " \\\\")
    body = "\n".join(rows)
    return f"""\\begin{{table}}[t]
\\centering
\\small
\\caption{{Symbol decode rate (\\%) against the RDA shrink $\\lambda$, without leak
inversion ($\\beta=0$) and with it ($\\beta=0.06$).}}
\\label{{tab:shrink}}
\\begin{{tabular}}{{@{{}}lrrrr@{{}}}}
\\toprule
 & \\multicolumn{{2}}{{c}}{{CUHK-CQRC}} & \\multicolumn{{2}}{{c}}{{qrism}} \\\\
\\cmidrule(lr){{2-3}}\\cmidrule(l){{4-5}}
$\\lambda$ & $\\beta=0$ & $0.06$ & $\\beta=0$ & $0.06$ \\\\
\\midrule
{body}
\\bottomrule
\\end{{tabular}}
\\end{{table}}
"""


SIZE_NAMES = {"08px": "0.34 mm", "12px": "0.5 mm", "20px": "0.84 mm"}
COND_ORDER = ["regular", "daylight", "flashlight", "monitor", "close", "wb-cool", "wb-neutral", "wb-warm"]


def best_beta(sw, kind, group):
    cands = [(k[2], r) for k, r in sw.items() if k[0] == kind and k[1] == group and k[2].startswith("b=")]
    # highest decode rate; ties go to the smaller beta, as in the bench
    return min(cands, key=lambda kr: (-float(kr[1]["code_pct"]), kr[0]))


def tab_breakdown(sw_q):
    rows = []
    for kind, groups, names in (
        ("size", ["08px", "12px", "20px"], SIZE_NAMES),
        ("heuristic", COND_ORDER, {}),
    ):
        if rows:
            rows.append("\\midrule")
        for g in groups:
            r0 = sw_q.get((kind, g, NODEBLUR))
            if r0 is None or int(r0["photos"]) == 0:
                continue
            r6 = sw_q[(kind, g, CHOSEN)]
            lab, rb = best_beta(sw_q, kind, g)
            rows.append(
                f"{names.get(g, g)} & {r0['photos']} & {f1(r0['code_pct'])} & {f1(r6['code_pct'])} "
                f"& {lab[2:]} & {f1(rb['code_pct'])} \\\\"
            )
    body = "\n".join(rows)
    return f"""\\begin{{table}}[t]
\\centering
\\small
\\caption{{qrism dataset: symbol decode rate (\\%) by module size and capture condition,
without leak inversion, at $\\beta=0.06$, and at the best fixed $\\beta$ for that
group. $n$ is localized photos.}}
\\label{{tab:breakdown}}
\\begin{{tabular}}{{@{{}}lrrrrr@{{}}}}
\\toprule
Group & $n$ & $\\beta=0$ & $0.06$ & best $\\beta$ & at best \\\\
\\midrule
{body}
\\bottomrule
\\end{{tabular}}
\\end{{table}}
"""


def timing_pass(path, marker="colour pipeline timing"):
    """A timing table, ours or the HiQ bench's: pipeline -> (ns per module, ms per v40 symbol).
    `marker` is the heading the table follows."""
    out = {}
    text = open(path).read().split(marker)[1]
    for line in text.splitlines():
        m = re.match(r"\s{4}(\S.*?)\s{2,}([\d.]+)\s+([\d.]+)\s+([\d.]+)\s+([\d.]+)x\s*$", line)
        if m:
            out[m.group(1)] = (float(m.group(3)), float(m.group(4)))
    return out


def load_split(path):
    """CUHK-CQRC metadata as {(folder, file): row}. Active and not training is HiQ's test set."""
    return {(r["folder"], r["file"]): r for r in csv.DictReader(open(path))}


def is_test(r):
    return r["isactive"] == "1" and r["istraining"] == "0"


def paired(full_csv, sweep_photos, split):
    """Joins the HiQ decoder's per-photo results with ours on HiQ's own test split, so no photo
    HiQ was trained on is scored. Returns the common set (both localized at the same version),
    every test photo, and lookup helpers."""
    H = defaultdict(dict)
    for r in csv.DictReader(open(full_csv)):
        H[(r["folder"], r["file"])][r["config"]] = r
    Q = {}
    for r in csv.DictReader(open(sweep_photos)):
        p = r["path"].split("/")
        Q[(p[-2], p[-1])] = r
    assert set(H) == set(split), "HiQ results and split metadata list different photos"
    keys = sorted(k for k in H if is_test(split[k]))
    h_ok = {k for k in keys if H[k]["qda+cmi-fix"]["status"] == "scored"}
    q_ok = {k for k in keys if k in Q}
    both = h_ok & q_ok
    common = sorted(k for k in both if H[k]["qda+cmi-fix"]["version"] == Q[k]["version"])

    def hd(k, cfg):
        r = H[k][cfg]
        return r["status"] == "scored" and r["code_ok"] == "1"

    def qd(k, s):
        return k in Q and Q[k][s] == "1"

    return dict(H=H, Q=Q, keys=keys, h_ok=h_ok, q_ok=q_ok, both=both, common=common, hd=hd, qd=qd)


HIQ_CONFIGS = [
    ("qda", "QDA"),
    ("qda+cmi-fix", "QDA + CMI, corrected"),
    ("lsvm-ip6plus", "layered SVM"),
]


def tab_paired(P):
    rows = []
    worst = 0.0
    for scope, keys in (("common", P["common"]), ("all", P["keys"])):
        n = len(keys)
        ours = [P["qd"](k, CHOSEN) for k in keys]
        lo, hi = wilson(sum(ours), n)
        rows.append(
            f"\\multicolumn{{4}}{{@{{}}l}}{{\\emph{{{'Localized by both' if scope == 'common' else 'All photos'} "
            f"($n={n}$)}}}} \\\\"
        )
        rows.append(f"Ours, $\\beta=0.06$ & {100 * sum(ours) / n:.1f} & [{lo:.1f}, {hi:.1f}] & \\\\")
        for cfg, label in HIQ_CONFIGS:
            theirs = [P["hd"](k, cfg) for k in keys]
            b = sum(x and not y for x, y in zip(ours, theirs))
            c = sum(y and not x for x, y in zip(ours, theirs))
            lo, hi = wilson(sum(theirs), n)
            worst = max(worst, mcnemar(b, c))
            rows.append(
                f"HiQ {label} & {100 * sum(theirs) / n:.1f} & [{lo:.1f}, {hi:.1f}] & {b}/{c} \\\\"
            )
        if scope == "common":
            rows.append("\\midrule")
    body = "\n".join(rows)
    return f"""\\begin{{table}}[t]
\\centering
\\small
\\caption{{End-to-end decoders on HiQ's test split of CUHK-CQRC, paired by photo. Each decoder uses its own
localization and sampling. CMI, corrected: all five CMI weights, with the neighbors HiQ's
sampler gathers (two of four on most modules). Disc.: photos only ours decoded / only HiQ
decoded; every difference has exact McNemar $p<10^{{{p_exp(worst)}}}$.}}
\\label{{tab:paired}}
\\setlength{{\\tabcolsep}}{{4pt}}
\\begin{{tabular}}{{@{{}}lrcr@{{}}}}
\\toprule
Decoder & Symbol \\% & 95\\% CI & Disc. \\\\
\\midrule
{body}
\\bottomrule
\\end{{tabular}}
\\end{{table}}
"""


# HiQ's shipped classifiers, ported to run on our samples (benches/color/recovery/hiq.rs), as
# (accuracy-pass pipeline, paper label, macro tag, HiQ bench config run end to end or None).
# HiQ's own `qda+cmi-fix` gathers neighbours with the sampler's `else if`, so the row that
# reproduces it is the one with HiQ's neighbours; all four neighbours has no HiQ counterpart.
HIQ_PORTS = [
    ("hiq qda", "QDA", "Qda", "qda"),
    ("hiq qda+cmi fixed", "QDA + CMI, corrected", "Cmi", None),
    ("hiq qda+cmi hiq nbrs", "\\quad with HiQ's neighbors", "CmiShipped", "qda+cmi-fix"),
    ("hiq lsvm", "Layered SVM", "Lsvm", "lsvm-ip6plus"),
]
ACC_CHOSEN = "rda + deblur b=.06"  # the chosen setting's name in the accuracy pass


def ported(acc_ph, P, split):
    """HiQ's classifiers and ours on the same samples, on HiQ's test split: every test photo
    our localizer found at a known version, and the common set of Table tab:paired."""
    def by_key(rows):
        return {tuple(p.split("/")[-2:]): r for p, r in rows.items()}

    ours = by_key(acc_ph[ACC_CHOSEN])
    test = sorted(k for k in ours if is_test(split[k]))
    # Same photos and same decodes as the sweep the end-to-end table is built from, or the two
    # tables would not be about the same thing.
    assert set(test) == P["q_ok"], "accuracy pass and sweep localized different test photos"
    assert all((ours[k]["decoded"] == "1") == P["qd"](k, CHOSEN) for k in test), \
        "accuracy pass and sweep disagree on the chosen setting"
    R = {"test": test, "common": P["common"], "ours": ours, "port": {}}
    for name, _, _, _ in HIQ_PORTS:
        R["port"][name] = by_key(acc_ph[name])
        assert set(R["port"][name]) == set(ours), f"{name} scored different photos"
    return R


def port_stats(R, name, keys):
    """Stat over `keys` for a port (or ours when `name` is None), with the photos only ours
    decoded and only it decoded, and the exact McNemar p between them."""
    rows = R["ours"] if name is None else R["port"][name]
    s = Stat(rows[k] for k in keys)
    if name is not None:
        s.b = sum(R["ours"][k]["decoded"] == "1" and rows[k]["decoded"] == "0" for k in keys)
        s.c = sum(R["ours"][k]["decoded"] == "0" and rows[k]["decoded"] == "1" for k in keys)
        s.p = mcnemar(s.b, s.c)
    return s


def tab_ported(R, P):
    """HiQ's classifiers on our samples next to ours, then the end-to-end figure HiQ's decoder
    reaches with the same classifier on its own samples, on the common set."""
    test, common = R["test"], R["common"]
    worst = 0.0
    ours, ours_c = port_stats(R, None, test), port_stats(R, None, common)
    rows = [
        f"Ours, $\\beta=0.06$ & {ours.data:.1f} & {ours.ber:.2f} & {ours.code:.1f} & & {ours_c.code:.1f} & \\\\",
        "\\midrule",
    ]
    for name, label, _, cfg in HIQ_PORTS:
        s, sc = port_stats(R, name, test), port_stats(R, name, common)
        worst = max(worst, s.p)
        # The caption says ours decodes more symbols than every HiQ classifier on the same samples.
        assert s.code < ours.code and sc.code < ours_c.code, f"{name} decodes as many symbols as ours"
        e2e = "--" if cfg is None else f"{100 * sum(P['hd'](k, cfg) for k in common) / len(common):.1f}"
        # The text says each HiQ classifier decodes about as many symbols on our samples as in
        # its own decoder, which is what puts the end-to-end gap on the classifier.
        assert cfg is None or abs(sc.code - float(e2e)) < 1.5, f"{name}: ported and end-to-end differ"
        rows.append(
            f"{label} & {s.data:.1f} & {s.ber:.2f} & {s.code:.1f} & {s.b}/{s.c} & {sc.code:.1f} & {e2e} \\\\"
        )
    assert worst < 0.05, "a HiQ classifier is not significantly worse than ours; fix the caption"
    body = "\n".join(rows)
    table = f"""\\begin{{table}}[t]
\\centering
\\footnotesize
\\caption{{HiQ's shipped classifiers run on our module samples, HiQ's test split of CUHK-CQRC:
module accuracy, bit error rate and symbol decode rate in percent over the {len(test)} test
photographs our localizer finds. CMI, corrected: all five weights and all four neighbors.
Disc.: photographs only ours decoded / only the HiQ classifier
decoded, exact McNemar $p<10^{{{p_exp(worst)}}}$ for every row. The last two columns are the
symbol decode rate on the {len(common)} photographs both decoders localize (Table~\\ref{{tab:paired}}),
on our samples and end to end in the HiQ decoder.}}
\\label{{tab:ported}}
\\setlength{{\\tabcolsep}}{{3pt}}
\\begin{{tabular}}{{@{{}}lrrrrrr@{{}}}}
\\toprule
 & \\multicolumn{{4}}{{c}}{{Test, our samples}} & \\multicolumn{{2}}{{c}}{{Common set}} \\\\
\\cmidrule(lr){{2-5}}\\cmidrule(l){{6-7}}
Classifier & Module & BER & Symbol & Disc. & Ours & HiQ \\\\
\\midrule
{body}
\\bottomrule
\\end{{tabular}}
\\end{{table}}
"""
    return table, worst


def parity_pass(path):
    """The totals row of the parity pass. The ported-classifier table means nothing unless the
    ports reproduced HiQ on HiQ's own inputs, so a failed check stops the build."""
    text = open(path).read()
    assert "parity holds" in text, "HiQ ports do not reproduce the HiQ decoder; see parity.txt"
    m = re.search(r"^\s{4}all\s+(\d+)\s+(\d+)\s+(\d+)\s+(\d+)\s+(\d+)\s+(\d+)\s+(\d+)\s*$", text, re.M)
    modules, classify, features, px, far, binar, colour = (int(x) for x in m.groups())
    assert classify == features == binar == far == 0, "parity totals changed; fix the text"
    return modules, colour


def numbers_ported(N, R, P, t, parity):
    """Macros for the classifier-only comparison and the ports' per-module cost."""
    test, common = R["test"], R["common"]
    N.add("PortTest", len(test))
    ours = port_stats(R, None, test)
    N.add("PortOursCode", f1(ours.code))
    N.add("PortOursBer", f2(ours.ber))
    N.add("PortOursData", f1(ours.data))
    N.add("PortOursCodeCommon", f1(port_stats(R, None, common).code))
    # The abstract and introduction quote the corrected CMI as the best HiQ classifier on our samples.
    best = max(HIQ_PORTS, key=lambda h: port_stats(R, h[0], test).code)
    assert best[2] == "Cmi", "best ported HiQ classifier changed; fix the abstract and introduction"
    for name, _, tag, cfg in HIQ_PORTS:
        s, sc = port_stats(R, name, test), port_stats(R, name, common)
        N.add(f"Port{tag}Code", f1(s.code))
        N.add(f"Port{tag}Ber", f2(s.ber))
        N.add(f"Port{tag}Data", f1(s.data))
        N.add(f"Port{tag}OnlyOurs", s.b)
        N.add(f"Port{tag}OnlyHiq", s.c)
        N.add(f"Port{tag}P", fmt_p(s.p))
        N.add(f"Port{tag}CodeCommon", f1(sc.code))
        N.add(f"Port{tag}PCommon", fmt_p(sc.p))
        # Timed in the same pass as ours; absent from a timing.txt older than the ports.
        if name in t:
            N.add(f"Port{tag}Ns", f"{t[name][0]:.1f}")
        if cfg is not None:
            N.add(f"Port{tag}EndToEnd", f"{100 * sum(P['hd'](k, cfg) for k in common) / len(common):.1f}")
    # Correcting the neighbour set is a classifier-side change, so test it on its own pairs.
    a, b = R["port"]["hiq qda+cmi fixed"], R["port"]["hiq qda+cmi hiq nbrs"]
    x = sum(a[k]["decoded"] == "1" and b[k]["decoded"] == "0" for k in test)
    y = sum(a[k]["decoded"] == "0" and b[k]["decoded"] == "1" for k in test)
    N.add("PortNbrsGainOnly", x)
    N.add("PortNbrsLossOnly", y)
    N.add("PortNbrsP", fmt_p(mcnemar(x, y)))
    modules, colour = parity
    N.add("PortParityModules", f"{modules:,}".replace(",", "{,}"))
    N.add("PortParityJpegPct", f1(100 * colour / modules))


COND_DESC = {
    "regular": "Reference capture",
    "daylight": "Outdoor daylight",
    "flashlight": "Phone flash on",
    "monitor": "Shown on a display",
    "perspective": "Oblique view",
    "close": "Close range",
    "small": "0.34 mm modules",
    "wb-cool": "White balance cool",
    "wb-neutral": "White balance neutral",
    "wb-warm": "White balance warm",
}
MODULE_MM = {8: "0.34", 12: "0.5", 20: "0.84"}  # printed at 600 dpi
COND_TABLE_ORDER = ["regular", "daylight", "flashlight", "monitor", "perspective", "close", "small",
                    "wb-cool", "wb-neutral", "wb-warm"]


def tab_dataset(counts_csv, sw_q):
    """The qrism dataset by capture condition: phones, module sizes, photos, localized."""
    by = {}
    for r in csv.DictReader(open(counts_csv)):
        d = by.setdefault(r["condition"], {"phones": set(), "px": set(), "n": 0})
        d["phones"].add(r["phone"])
        d["px"].add(int(r["module_px"]))
        d["n"] += int(r["photos"])
    rows, tot, tot_loc = [], 0, 0
    for c in COND_TABLE_ORDER:
        d = by[c]
        loc = sw_q.get(("heuristic", c, CHOSEN))
        loc = int(loc["photos"]) if loc else 0
        tot += d["n"]
        tot_loc += loc
        mm = ", ".join(MODULE_MM[x] for x in sorted(d["px"]))
        rows.append(f"{COND_DESC[c]} & {len(d['phones'])} & {mm} & {d['n']} & {loc} \\\\")
    rows.append("\\midrule")
    phones = set().union(*(d["phones"] for d in by.values()))
    rows.append(f"Total & {len(phones)} & & {tot} & {tot_loc} \\\\")
    body = "\n".join(rows)
    return f"""\\begin{{table}}[t]
\\centering
\\small
\\caption{{The qrism dataset by capture condition: phones used, printed module
sizes in mm, photos, and photos the localizer found at the right version.}}
\\label{{tab:dataset}}
\\begin{{tabular}}{{@{{}}lrlrr@{{}}}}
\\toprule
Condition & Phones & mm & Photos & Localized \\\\
\\midrule
{body}
\\bottomrule
\\end{{tabular}}
\\end{{table}}
"""


# Inline numbers
# ------------------------------------------------------------------------------


def numbers(N, sw_h, sw_q, ph_h, ph_q, sh_h, sh_q, shp_h, shp_q, P, bs_h, bs_q, worst, split,
            hiq_attempted, sweep_log):
    for tag, sw, ph, sh, shp, attempted in (
        ("Hiq", sw_h, ph_h, sh_h, shp_h, hiq_attempted),
        ("Qr", sw_q, ph_q, sh_q, shp_q, None),
    ):
        a6 = sw[("all", "all", CHOSEN)]
        a0 = sw[("all", "all", NODEBLUR)]
        n = int(a6["photos"])
        N.add(f"{tag}Localized", n)
        N.add(f"{tag}CodeChosen", f1(a6["code_pct"]))
        N.add(f"{tag}CodeChosenLo", f1(a6["code_ci_lo"]))
        N.add(f"{tag}CodeChosenHi", f1(a6["code_ci_hi"]))
        N.add(f"{tag}CodeNoDeblur", f1(a0["code_pct"]))
        N.add(f"{tag}DataChosen", f1(a6["data_pct"]))
        N.add(f"{tag}DataNoDeblur", f1(a0["data_pct"]))
        N.add(f"{tag}BerChosen", f2(a6["ber_pct"]))
        N.add(f"{tag}BerNoDeblur", f2(a0["ber_pct"]))
        for ch, name in (("r", "R"), ("g", "G"), ("b", "B")):
            N.add(f"{tag}BerChosen{name}", f2(a6[f"ber_{ch}"]))
        N.add(f"{tag}DfrChosen", f1(100 - float(a6["code_pct"])))
        x, y = discordant(ph, CHOSEN, NODEBLUR)
        N.add(f"{tag}DeblurGainOnly", x)
        N.add(f"{tag}DeblurLossOnly", y)
        N.add(f"{tag}DeblurP", fmt_p(mcnemar(x, y)))
        for other, nm in (("b=0.05", "Five"), ("b=0.04", "Four"), ("b=0.07", "Seven")):
            r = sw[("all", "all", other)]
            N.add(f"{tag}Code{nm}", f1(r["code_pct"]))
            x, y = discordant(ph, CHOSEN, other)
            N.add(f"{tag}P{nm}", fmt_p(mcnemar(x, y)))
        # beta where pooled BER and data% are best
        fixed = [(k[2], r) for k, r in sw.items() if k[:2] == ("all", "all") and k[2].startswith("b=")]
        bmin = min(fixed, key=lambda kr: float(kr[1]["ber_pct"]))
        dmax = max(fixed, key=lambda kr: float(kr[1]["data_pct"]))
        N.add(f"{tag}BerMinBeta", bmin[0][2:])
        N.add(f"{tag}BerMin", f2(bmin[1]["ber_pct"]))
        N.add(f"{tag}DataMaxBeta", dmax[0][2:])
        op = sw[("all", "all", "optical")]
        N.add(f"{tag}CodeOptical", f1(op["code_pct"]))
        N.add(f"{tag}BerOptical", f2(op["ber_pct"]))
        x, y = discordant(ph, CHOSEN, "optical")
        N.add(f"{tag}POptical", fmt_p(mcnemar(x, y)))
        hi = sw[("all", "all", "b=0.12")]
        N.add(f"{tag}CodeTwelve", f1(hi["code_pct"]))
        # shrink grid
        for lam, nm in (("0.0", "Qda"), ("1.0", "Lda")):
            r = sh[("all", "all", f"b=0.06 s={lam}")]
            N.add(f"{tag}Code{nm}", f1(r["code_pct"]))
            x, y = discordant(shp, CHOSEN_GRID, f"b=0.06 s={lam}")
            N.add(f"{tag}P{nm}", fmt_p(mcnemar(x, y)))
        grid = [(k[2], r) for k, r in sh.items() if k[:2] == ("all", "all")]
        best = max(grid, key=lambda kr: (float(kr[1]["code_pct"]), -grid.index(kr)))
        N.add(f"{tag}GridBest", best[0].replace("b=", "$\\beta=").replace(" s=", "$, $\\lambda=") + "$")
        N.add(f"{tag}GridBestCode", f1(best[1]["code_pct"]))
        x, y = discordant(shp, best[0], CHOSEN_GRID)
        N.add(f"{tag}GridBestP", fmt_p(mcnemar(x, y)))
        if attempted:
            N.add(f"{tag}Attempted", attempted)

    # The per-photo CSVs list localized photos only, so the attempted count comes from the log.
    m = re.search(r"qrism dataset.*?(\d+) photos attempted, (\d+) localized", open(sweep_log).read(), re.S)
    N.add("QrAttempted", m.group(1))

    # module-size optima
    for g, nm in (("08px", "Eight"), ("12px", "Twelve"), ("20px", "Twenty")):
        lab, r = best_beta(sw_q, "size", g)
        N.add(f"QrBestBetaPx{nm}", lab[2:])
        N.add(f"QrBestCodePx{nm}", f1(r["code_pct"]))
        N.add(f"QrCodeChosenPx{nm}", f1(sw_q[("size", g, CHOSEN)]["code_pct"]))
        N.add(f"QrLocalizedPx{nm}", sw_q[("size", g, CHOSEN)]["photos"])
    for g, nm in (("daylight", "Daylight"), ("wb-neutral", "WbNeutral"), ("flashlight", "Flashlight")):
        lab, r = best_beta(sw_q, "heuristic", g)
        N.add(f"QrBestBeta{nm}", lab[2:])
        N.add(f"QrBestCode{nm}", f1(r["code_pct"]))
        N.add(f"QrCodeChosen{nm}", f1(sw_q[("heuristic", g, CHOSEN)]["code_pct"]))

    N.add("QrNotLocalized", int(m.group(1)) - int(m.group(2)))

    # baselines on CUHK-CQRC
    N.add("HiqCodeEuclid", f1(bs_h[CENTROID].code))
    N.add("HiqCodePerColorant", f1(bs_h["per-colorant int + adp"].code))
    N.add("HiqCodeJab", f1(bs_h["jabcode (balanced)"].code))
    N.add("HiqCodeRda", f1(bs_h["mahalanobis rda (raw)"].code))
    N.add("HiqDataEuclid", f1(bs_h[CENTROID].data))
    N.add("QrCodeEuclid", f1(bs_q[CENTROID].code))
    N.add("QrCodePerColorant", f1(bs_q["per-colorant int + adp"].code))
    N.add("QrCodeJab", f1(bs_q["jabcode (balanced)"].code))
    N.add("QrCodeRda", f1(bs_q["mahalanobis rda (raw)"].code))
    N.add("QrDataEuclid", f1(bs_q[CENTROID].data))
    N.add("BaselineWorstP", fmt_p(worst))

    # HiQ's split of CUHK-CQRC, from the dataset's metadata
    rows = list(split.values())
    N.add("SplitTest", sum(is_test(r) for r in rows))
    N.add("SplitTrain", sum(r["isactive"] == "1" and r["istraining"] == "1" for r in rows))
    N.add("SplitUnused", sum(r["isactive"] == "0" for r in rows))

    # pooled BER is minimized at a larger beta than decoding, more so for small modules
    for g, nm in (("08px", "Eight"), ("12px", "Twelve"), ("20px", "Twenty")):
        fixed = [(k[2], r) for k, r in sw_q.items() if k[:2] == ("size", g) and k[2].startswith("b=")]
        N.add(f"QrBerMinBetaPx{nm}", min(fixed, key=lambda kr: float(kr[1]["ber_pct"]))[0][2:])

    # best beta of each shrink column with lambda >= 0.3, over both datasets
    betas = set()
    for sh in (sh_h, sh_q):
        for j in range(3, 11):
            col = [(k[2], r) for k, r in sh.items() if k[:2] == ("all", "all") and k[2].endswith(f"s={j / 10:.1f}")]
            betas.add(min(col, key=lambda kr: (-float(kr[1]["code_pct"]), kr[0]))[0].split()[0][2:])
    lo_b, hi_b = min(betas), max(betas)
    N.add("GridBestBetaRange", lo_b if lo_b == hi_b else f"{lo_b} to {hi_b}")

    # paired comparison with the HiQ decoder
    common, keys = P["common"], P["keys"]
    N.add("PairTest", len(keys))
    N.add("PairHiqLocalized", len(P["h_ok"]))
    N.add("PairOursLocalized", len(P["q_ok"]))
    N.add("PairBoth", len(P["both"]))
    N.add("PairCommon", len(common))
    N.add("PairVersionDisagree", len(P["both"]) - len(common))
    for scope, ks, nm in (("common", common, "Common"), ("all", keys, "All")):
        n = len(ks)
        ours = [P["qd"](k, CHOSEN) for k in ks]
        nod = [P["qd"](k, NODEBLUR) for k in ks]
        theirs = [P["hd"](k, "qda+cmi-fix") for k in ks]
        N.add(f"Pair{nm}Ours", f"{100 * sum(ours) / n:.1f}")
        N.add(f"Pair{nm}OursNoDeblur", f"{100 * sum(nod) / n:.1f}")
        N.add(f"Pair{nm}Hiq", f"{100 * sum(theirs) / n:.1f}")
        b = sum(x and not y for x, y in zip(ours, theirs))
        c = sum(y and not x for x, y in zip(ours, theirs))
        N.add(f"Pair{nm}OnlyOurs", b)
        N.add(f"Pair{nm}OnlyHiq", c)
        N.add(f"Pair{nm}P", fmt_p(mcnemar(b, c)))
        for cfg, cn in (("lsvm-ip6plus", "Lsvm"), ("qda", "Qda")):
            N.add(f"Pair{nm}{cn}", f"{100 * sum(P['hd'](k, cfg) for k in ks) / n:.1f}")

    # does ours lead in every version and capture folder of the common set?
    for key, nm in ((lambda k: P["Q"][k]["version"], "Versions"), (lambda k: k[0], "Folders")):
        groups = defaultdict(list)
        for k in common:
            groups[key(k)].append(k)
        lead = sum(
            sum(P["qd"](k, CHOSEN) for k in ks) > sum(P["hd"](k, "qda+cmi-fix") for k in ks)
            for ks in groups.values()
        )
        N.add(f"Pair{nm}Lead", lead)
        N.add(f"Pair{nm}Total", len(groups))


# Driver
# ------------------------------------------------------------------------------


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bench", default="../benches/color")
    ap.add_argument("--hiq-full", default="../../hiq/HiQ/hiq-decoder/bench/results/full.csv")
    ap.add_argument("--counts", default="data/qrism_dataset_counts.csv")
    ap.add_argument("--split", default="data/cuhk_cqrc_split.csv")
    ap.add_argument("--hiq-timing", default="../../hiq/HiQ/hiq-decoder/bench/results/timing.txt")
    ap.add_argument("--out", default="generated")
    a = ap.parse_args()
    b = a.bench
    os.makedirs(a.out, exist_ok=True)

    acc, hiq_attempted, hiq_scored = accuracy_pass(f"{b}/results.txt")
    acc_ph = long_photos(f"{b}/accuracy_hiq_photos.csv")
    q_ph = long_photos(f"{b}/qrism_photos.csv")
    split = load_split(a.split)
    sw_h, sw_q = rates(f"{b}/sweep_hiq.csv"), rates(f"{b}/sweep_qrism.csv")
    ph_h, ph_q = photos(f"{b}/sweep_hiq_photos.csv"), photos(f"{b}/sweep_qrism_photos.csv")
    sh_h, sh_q = rates(f"{b}/shrink_hiq.csv"), rates(f"{b}/shrink_qrism.csv")
    shp_h, shp_q = photos(f"{b}/shrink_hiq_photos.csv"), photos(f"{b}/shrink_qrism_photos.csv")
    P = paired(a.hiq_full, f"{b}/sweep_hiq_photos.csv", split)

    # The accuracy pass and the sweep must have scored the same photos, or the baseline table
    # mixes denominators.
    assert hiq_scored == int(sw_h[("all", "all", CHOSEN)]["photos"]), "accuracy pass and sweep disagree"
    # beta = 0 in the sweep is the RDA row of the accuracy pass and of the qrism report, and
    # all three runs must have localized the same photos, or the tables mix denominators.
    for ph, sw, name in ((acc_ph, sw_h, "accuracy pass"), (q_ph, sw_q, "qrism report")):
        rda = ph["mahalanobis rda (raw)"]
        r0 = sw[("all", "all", NODEBLUR)]
        assert len(rda) == int(r0["photos"]), f"{name} and sweep localized different photos"
        assert abs(Stat(rda.values()).code - float(r0["code_pct"])) < 0.001, f"{name} and sweep disagree"

    table, worst, bs_h, bs_q, _ = tab_baselines(acc_ph, q_ph, sw_h, ph_h, sw_q, ph_q)
    # The CUHK-CQRC optical row comes from the accuracy pass, the text's optical numbers
    # from the sweep; they must be the same run.
    op_acc = Stat(acc_ph["rda + deblur optical"].values()).code
    assert abs(op_acc - float(sw_h[("all", "all", "optical")]["code_pct"])) < 0.001, "optical rows disagree"
    write(f"{a.out}/tab_baselines.tex", table)
    write(f"{a.out}/tab_shrink.tex", tab_shrink(sh_h, sh_q))
    write(f"{a.out}/tab_breakdown.tex", tab_breakdown(sw_q))
    write(f"{a.out}/tab_paired.tex", tab_paired(P))
    R = ported(acc_ph, P, split)
    table, _ = tab_ported(R, P)
    write(f"{a.out}/tab_ported.tex", table)

    write(f"{a.out}/tab_dataset.tex", tab_dataset(a.counts, sw_q))

    N = Numbers()
    N.add("HiqNoSymbol", accuracy_pass.no_symbol)
    N.add("HiqMisread", accuracy_pass.misread)
    t = timing_pass(f"{b}/timing.txt")
    ours, rda = t["rda + deblur b=.06"], t["mahalanobis rda (raw)"]
    N.add("TimeChosenNs", f"{ours[0]:.1f}")
    N.add("TimeChosenVforty", f"{ours[1]:.2f}")
    N.add("TimeRdaNs", f"{rda[0]:.1f}")
    N.add("TimeDeblurNs", f"{ours[0] - rda[0]:.1f}")
    # HiQ's classifiers are timed as Rust ports in the same pass as ours (numbers_ported); the
    # Java timings in a.hiq_timing are no longer used in the text.
    numbers(N, sw_h, sw_q, ph_h, ph_q, sh_h, sh_q, shp_h, shp_q, P, bs_h, bs_q, worst, split,
            hiq_attempted, f"{b}/sweep.txt")
    numbers_ported(N, R, P, t, parity_pass(f"{b}/hiq_parity/parity.txt"))
    N.write(f"{a.out}/numbers.tex")
    print(f"wrote {a.out}/: numbers.tex ({len(N.lines)} macros) and 6 tables")


if __name__ == "__main__":
    main()
