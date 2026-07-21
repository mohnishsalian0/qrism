# qrism reader — improvement backlog

Findings from benchmarking qrism's detector against rxing (the zxing Rust port) on
`benches/dataset/detection`. Baseline: qrism decodes **2/33** high-version images where rxing
decodes **32/33**, and qrism's overall median detect time is **~99–107 ms** vs rxing's **~8–9 ms**.

The two problems are independent: accuracy is a **grid-sampling architecture gap**, speed is a
**memory-layout + candidate-explosion** problem. Neither is really about "high version" per se.

---

## Accuracy — why high versions fail (2/33)

The failure is layered; fixing one stage just exposes the next. Order of discovery below.

### A1. Binarizer threshold window scales with image size, not module size — HIGH
`BinaryImage::prepare` sets `block_pow = log2(min(w,h)/20)`, giving **32 px blocks at 1196², 64 px
at 1641²**, and the threshold averages a 5×5 grid of those → a **160×160 to 320×320 px window**.
At version 40 a module is ~4–9 px, so the threshold is averaged over ~40 modules and dragged up by
the bright paper margin. The 1-module white gap between a finder's ring and stone binarizes to
black, fusing ring+gap+stone into one blob. Confirmed: stone and ring resolve to the *same*
connected region. **14/33 high-version images die here, before any QR logic runs** (only 2 finders
found → 0 groups).

Fix direction: make block size track estimated **module** size, not image size. A fixed 8 px block
alone is *not* the fix — it helps bright_spots (7→21) and glare (12→15) but destroys close (23→2),
monitor (5→0) and makes `lots` 10× slower, because close-ups have huge modules and an 8 px block
thresholds *inside* a single module. Pair any block-size change with A2.

### A2. Low-dynamic-range ("blank block") rule is commented out — MEDIUM (free win)
`binarize.rs` has the zxing blank-block rule disabled behind a `FIXME`: when a block's
`max - min <= 25`, borrow the threshold from neighbouring blocks instead of thresholding noise.
Re-enabling it **alone**, at the current block size, measured: nominal 27.3→22.1 ms and 50→**54**
decoded; blurred 50.7→38.7 ms and 22→**25**. It also backstops A1 when block size shrinks.

### A3. Provisional version estimate is too imprecise — MEDIUM
`verify_symbol_size` counts timing transitions and does `ver = floor((size-15)/4)`, which needs ±2
modules of accuracy out of 177. Measured errors: image002 → **37 vs true 40**, image003 → 36 vs 40,
image017/024 → 24 vs 25. The version *bits* read fine (they sit next to the finders that anchor the
homography), so the truth is recoverable — the estimate just can't be trusted to pick the grid size.

### A4. Single 4-point homography can't span a large symbol — HIGH (the real one)
Even with the correct version, one perspective transform fitted to 4 corners drifts across a big
grid. Measured via timing-pattern agreement (which must alternate): the 2 images that decode score
**100%**; every failure scores **49–89%** (49% = chance), with the first mismatch only ~10–30
modules from the anchored corner. Forcing the correct version *and* refitting the homography still
left it at **2/33** — the 4-point model itself is the ceiling.

`locate_alignment_pattern` finds exactly **one** alignment pattern (bottom-right, spiral search).
rxing's `SampleQR` resolves **every** alignment pattern's pixel position, fills gaps from neighbours,
and builds **one `PerspectiveTransform` per alignment-lattice cell** — 36 cells for a version 40.

Fix direction: locate all alignment patterns and do piecewise per-cell grid sampling. This is the
single highest-leverage accuracy change.

---

## Speed — why it's ~99 ms vs ~9 ms

`prepare` + `locate_finders` are essentially the whole budget (e.g. `close`: 25 + 48 of 74 ms).

### S1. `Pixel` is 16 bytes → 22.9–45.5 MB image buffer — HIGH
`Pixel::Visited(usize, Color)` is **16 bytes** (the `usize` region id forces 8-byte alignment;
`Color` is 1 byte). Buffer = **22.9 MB at 1196², 45.5 MB at 1641×1734**. zxing's BitMatrix is 1
bit/px → **0.18–0.36 MB, 128× smaller**. Theirs stays cache-resident; qrism streams from DRAM on
every pass. On this M4: 128-byte cache line, 64 KB L1d/P-core, **16 MB L2 shared across 4 P-cores**
— and the benchmark runs `rayon` across images, so several 22 MB buffers contend for that one L2.

The buffer conflates two jobs: the **binarized color** (1 bit B&W / 3 bits color) and the mutable
**flood-fill region label** (the `usize`). The 16 bytes is entirely the label.

Fix direction (recommended = split the planes):
- Immutable **image plane**: 1 bit/px for the traditional B&W path (the detection benchmark is
  entirely B&W — the 8 colors only matter for `detect_hc_qr`); 4-bit nibbles for the color path.
- Separate **label plane** (`Vec<u32>`, sentinel = unlabeled) touched only during finder flood
  fills, not on every scan read.
- Why splitting helps even though total size is similar: the hot scan reads the color plane billions
  of times, so making *that* plane 16× denser means 16× less data streamed and 16× better cache-line
  utilization; the label plane is cold (consulted only at finder-candidate points) and barely
  occupies cache.

Lowest-risk first step: `Color` byte plane + move the id to a `u32` side buffer (~5 B/px, ~7 MB,
under L2, no bit-masking) captures ~90% of the win. Go to 1-bit later for the rest.
Target ≤ ~2 MB per image for whole-buffer L2 residency under the parallel benchmark.

### S2. Finder verification dominates due to candidate explosion — DONE (partial)
**Resolution (this branch):** landed (a) a shared `matches_finder_ratio` that decouples bar/space
module sizes + tolerances (`bar: m*0.75+0.5`, `space: m/3+0.5`, plus an `M > 4m` divergence
reject), used by *both* the horizontal `is_finder_line` and the vertical crosscheck; and (b)
row-skipping in `locate_finders` (stride `clamp(1,3)`). Measured: `locate_finders` **58.3 → 43.5
ms/image**, overall median **99.2 → 79.4 ms**, accuracy **790 → 789** (−1; glare 14→13, lots
408→407 vs baseline).

Findings worth keeping: the speed win was **entirely row-skipping**; the decoupled tolerance was
speed-neutral *and* accuracy-neutral at `bar=0.75` (kept anyway — it's structurally correct for
ink-bleed and sets up A1). Pushing `bar` to 0.5 saved only ~4 ms but cost 3 finders, so not done.
The stride is capped at **3**, not rxing's full `(3h)/(4·MAX_MODULES)≈5`: a 3-module (≥3 px) stone
band is always crossed by a stride-3 scan, whereas stride 5 assumes ≥1.7 px modules and cost
**66 finders on `lots`** (many small symbols). Stride 3 costs only 2 finders vs stride 2 (glare −1,
lots −1) for ~9% more overall speed. Row-skipping is also what mitigates the
"re-verify a failed blob on every row" issue below, since a false blob is now hit half as often.
Not done: `min_module_size` / quiet-zone gates (deliberately skipped) and a proximity dedup gate
(qrism already skips *confirmed* finders via `is_finder`; the gate would only help near confirmed
finders, which is marginal).

Original analysis:
`locate_finders`: on `close` only ~15 of 48 ms is the raw scan; **~69–78% is verifying candidates**.
Cause: `FINDER_PATTERN_TOLERANCE = 0.95` with a single shared `avg` for all five runs is loose →
**~13,000–19,000 candidates/image**, each paying a vertical Bresenham crosscheck plus flood fills
over the 16 B/px buffer.

Contrast with rxing:
- rxing scans **every 3rd row** (`iSkip`), drops to 2 after a confirmed centre, and **early-exits**
  via `haveMultiplyConfirmedCenters`; qrism scans every row to completion.
- rxing cpp_port's `IsPattern` computes **separate module sizes for bars vs spaces** with different
  tolerances (`bar: m*0.75+0.5`, `space: m/3+0.5`) — exactly the ink-bleed case that fuses
  high-version finders. qrism's single `avg` structurally can't express it.
- rxing requires a quiet zone (`min_quiet_zone = 0.1`) and gates on `min_module_size`; qrism has
  neither.

Quick measured win: tightening the tolerance to 0.4 took `close`'s `locate_finders` 51 ms → 23 ms
with **no accuracy change** (23/40 throughout). A proper fix also decouples bar/space tolerance
(helps A1's fused finders too) and adds row-skipping.

### S3. `group_finders` is O(n³) with no pruning — HIGH (on dense scenes)
Triple-nested loop over all finder candidates, with `acos` + two `sqrt` in the innermost loop,
returning every passing triple and sorting them all. Measured on `lots`: **180 finders → 2.9 M
triples → 828,797 groups** (28% of all triples survive as allocated `FinderGroup`s), costing ~50 ms
for grouping and ~70 ms more in `locate_symbols` walking those groups.

rxing's `GenerateFinderPatternSets` is effectively **O(n)**: sort by size, **spatially bin** centres,
spiral only over nearby bins within `max_dist`, and **cap candidates at 15** with a `break`. It also
accepts a tighter 60°–120° angle window and compares **cosines directly against precomputed
`cos(60°)`/`cos(120°)`** — no `acos` at all — plus size-ratio and module-count gates.

Fix direction: spatial binning + candidate cap + module-count gate + compare cosines instead of
`acos`. Should take `lots`'s ~120 ms (group + locate) to near-nothing. Independent of S1, and does
not need the binarizer work.

---

## Already fixed (in tree)

### F1. `max_fitness_score` alignment-pattern count was wrong — DONE
`symbol_fitness` scores an alignment pattern at every pair of alignment coords minus the 3 finder
corners = **n²−3**; `max_fitness_score` used **n**. Error flips sign: v2–6 overcount 2×, v7+
undercount up to **6.57×** (v40: gate was 2332, should be 5842), so `jiggle_homography`'s acceptance
threshold was badly miscalibrated at high versions.

Fixed by introducing a shared `alignment_centres(ver)` iterator that both `symbol_fitness` and
`max_fitness_score` derive from, so they can't drift again. Added tests checking the count against
ISO/IEC 18004 Annex E, no duplicates, finder corners excluded, and v1 empty. No accuracy change on
the benchmark today (the newly-rejectable v40 fits were already dying downstream) — this is a latent
miscalibration that becomes load-bearing once A4 lands.

---

## Minor bugs (separate from the headline work)

- **`ring_fitness` typo** (`symbol.rs`): `cell_fitness(img, h, cx + r, cy - r + 1)` — the `1` should
  be `i`. The right edge of every ring samples one cell `2r` times. Harmless when perfectly aligned
  (same colour) but flattens the gradient `jiggle_homography` climbs.
- **Benchmark comment vs reality** (`benches/rxing.rs:20-21`): claims qrism does `to_luma8()`
  outside the timer; it actually does it inside `detect_qr`. Immaterial (2–4% of time), but the
  header is misleading — the timer boundary is *not* meaningfully unfair to qrism.

---

## Suggested order of attack

1. **S1** (Pixel 16 B → 1 bit / byte-plane split) — biggest single speed win, ~128× less memory
   traffic; unblocks everything else.
2. ~~**S2** (finder tolerance: tighten + decouple bar/space + row-skip) — speed *and* helps A1.~~ **DONE.**
3. **S3** (group_finders: bin + cap + gate + drop `acos`) — kills the `lots`/dense-scene cost.
4. **A1 + A2** (module-proportional block size + re-enable low-variance rule) — the binarizer.
5. **A4** (multi-alignment-pattern piecewise sampling) — the real high-version accuracy fix.

Priorities 1–3 are pure speed and mostly independent; 4–5 are the accuracy story.
