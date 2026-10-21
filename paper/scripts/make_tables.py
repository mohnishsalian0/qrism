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
  ../../hiq/HiQ/hiq-decoder/bench/results/timing.txt  HiQ bench timing, same machine

Outputs, under `generated/`:
  numbers.tex             \\newcommand macros for every number quoted in the text
  tab_baselines.tex       colour-recovery pipelines on both datasets
  tab_shrink.tex          RDA shrink at beta 0 and 0.06, both datasets
  tab_breakdown.tex       qrism decode rate by module size and capture condition
  tab_paired.tex          paired comparison with the HiQ decoder
"""
import argparse
import csv
import math
import os
import re
from collections import defaultdict

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
# setting is added last from the sweep (CUHK-CQRC) and the qrism report (qrism).
BASELINES = [
    ("Centroid, RGB", "euclid raw", "euclid raw", "centroid"),
    ("Centroid, optical density", "euclid intensity", "euclid intensity", "centroid"),
    ("Centroid, black/white stretch", "black/white euclid", "black/white euclid", "centroid"),
    ("Interf. canc., density", "per-colorant int + adp", "per-colorant int + adp", "channel"),
    ("Interf. canc., absorptance", "per-colorant abs + adp", None, "channel"),
    ("Interf. canc., local thr.", "per-colorant int + local", None, "channel"),
    ("Local threshold", "local standalone (raw)", None, "channel"),
    ("JAB Code classifier", None, "jabcode (balanced)", "channel"),
    ("LDA", "mahalanobis lda (raw)", "mahalanobis lda (raw)", "da"),
    ("QDA", "mahalanobis qda (raw)", "mahalanobis qda (raw)", "da"),
    ("RDA ($\\lambda=0.5$)", "mahalanobis rda (raw)", "mahalanobis rda (raw)", "da"),
    ("Ours, $\\beta$ fit per image", "rda + deblur fit", None, "ours"),
]
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


def tab_baselines(acc_ph, q_ph, sw_h, ph_h):
    """Color recovery pipelines on both datasets, every row over the same localized photos
    of its dataset. Returns the table and the largest McNemar p of any baseline against the
    chosen setting, for the caption and the text."""
    ours_h = {r["path"]: r[CHOSEN] == "1" for r in ph_h}
    ours_q = decoded_by_path(q_ph[QRISM_CHOSEN])
    worst = 0.0
    rows, prev = [], None
    stats_h, stats_q = {}, {}

    def cell(ph, name, ours, stats):
        nonlocal worst
        if name is None:
            return "-- & --"
        s = Stat(ph[name].values())
        mine = decoded_by_path(ph[name])
        b = sum(ours[p] and not mine.get(p, False) for p in ours)
        c = sum(mine.get(p, False) and not ours[p] for p in ours)
        worst = max(worst, mcnemar(b, c))
        stats[name] = s
        return f"{s.ber:.2f} & {s.code:.1f}"

    for label, kh, kq, fam in BASELINES:
        if prev is not None and fam != prev:
            rows.append("\\midrule")
        prev = fam
        rows.append(f"{label} & {cell(acc_ph, kh, ours_h, stats_h)} & {cell(q_ph, kq, ours_q, stats_q)} \\\\")
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
at the right version ({n_h} on CUHK-CQRC, {n_q} on qrism). Centroid: nearest calibration
centroid; interf.\\ canc.: interference cancellation; ours: RDA with leak inversion. Ours
decodes more symbols than every baseline, with exact McNemar $p<10^{{{p_exp(worst)}}}$.}}
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


SIZE_NAMES = {"08px": "8 px", "12px": "12 px", "20px": "20 px"}
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
localization and sampling. Disc.: photos only ours decoded / only HiQ decoded;
every difference has exact McNemar $p<10^{{{p_exp(worst)}}}$.}}
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


COND_DESC = {
    "regular": "Reference capture",
    "daylight": "Outdoor daylight",
    "flashlight": "Phone flash on",
    "monitor": "Shown on a display",
    "perspective": "Oblique view",
    "close": "Close range",
    "small": "8 px modules",
    "wb-cool": "White balance cool",
    "wb-neutral": "White balance neutral",
    "wb-warm": "White balance warm",
}
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
        px = ", ".join(str(x) for x in sorted(d["px"]))
        rows.append(f"{COND_DESC[c]} & {len(d['phones'])} & {px} & {d['n']} & {loc} \\\\")
    rows.append("\\midrule")
    phones = set().union(*(d["phones"] for d in by.values()))
    rows.append(f"Total & {len(phones)} & & {tot} & {tot_loc} \\\\")
    body = "\n".join(rows)
    return f"""\\begin{{table}}[t]
\\centering
\\small
\\caption{{The qrism dataset by capture condition: phones used, module sizes of the
source renders in pixels, photos, and photos the localizer found at the right version.}}
\\label{{tab:dataset}}
\\begin{{tabular}}{{@{{}}lrlrr@{{}}}}
\\toprule
Condition & Phones & px & Photos & Localized \\\\
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
    N.add("HiqCodeEuclid", f1(bs_h["euclid raw"].code))
    N.add("HiqCodePerColorant", f1(bs_h["per-colorant int + adp"].code))
    N.add("HiqCodeRda", f1(bs_h["mahalanobis rda (raw)"].code))
    N.add("HiqDataEuclid", f1(bs_h["euclid raw"].data))
    N.add("HiqCodeFit", f1(bs_h["rda + deblur fit"].code))
    N.add("QrCodeEuclid", f1(bs_q["euclid raw"].code))
    N.add("QrCodePerColorant", f1(bs_q["per-colorant int + adp"].code))
    N.add("QrCodeJab", f1(bs_q["jabcode (balanced)"].code))
    N.add("QrCodeRda", f1(bs_q["mahalanobis rda (raw)"].code))
    N.add("QrDataEuclid", f1(bs_q["euclid raw"].data))
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

    table, worst, bs_h, bs_q, _ = tab_baselines(acc_ph, q_ph, sw_h, ph_h)
    write(f"{a.out}/tab_baselines.tex", table)
    write(f"{a.out}/tab_shrink.tex", tab_shrink(sh_h, sh_q))
    write(f"{a.out}/tab_breakdown.tex", tab_breakdown(sw_q))
    write(f"{a.out}/tab_paired.tex", tab_paired(P))

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
    # The HiQ bench times the same span (per-module colour cost, single core, decode and
    # localization excluded), on the same machine.
    th = timing_pass(a.hiq_timing, "HiQ colour classifier timing")
    for cfg, nm in (("lsvm-ip6plus", "Lsvm"), ("qda", "Qda"), ("qda+cmi-fix", "QdaCmi")):
        N.add(f"HiqTime{nm}Ns", f"{th[cfg][0]:.1f}")
        N.add(f"HiqTime{nm}Vforty", f"{th[cfg][1]:.2f}")
    # The text places our cost between HiQ's layered SVM and its QDA.
    assert th["lsvm-ip6plus"][0] < ours[0] < th["qda"][0], "timing order changed; fix the text"
    numbers(N, sw_h, sw_q, ph_h, ph_q, sh_h, sh_q, shp_h, shp_q, P, bs_h, bs_q, worst, split,
            hiq_attempted, f"{b}/sweep.txt")
    N.write(f"{a.out}/numbers.tex")
    print(f"wrote {a.out}/: numbers.tex ({len(N.lines)} macros) and 5 tables")


if __name__ == "__main__":
    main()
