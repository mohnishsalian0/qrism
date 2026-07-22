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

### Current sub-stage profile (this branch, 536 images, rayon-parallel, M4)
Measured by temporarily instrumenting each pass with `Instant`-based atomics:

| stage | ms/image | sub-stage | ms/image |
| --- | --- | --- | --- |
| `prepare` | **9.5** (was 18.7) | block-stat accumulate | 2.2 (was 9.9) |
| | | threshold calc | 0.01 |
| | | binarize | 6.8 (was 8.2) |
| `locate_finders` | **20.4** | horizontal scan | 3.3 |
| | | verify: vertical crosscheck | 1.9 |
| | | verify: stone+ring flood fills | **13.9** |
| `group_finders` | 1.2 | | |
| `locate_symbols` | 4.2 | | |

Two facts fell out of this: `prepare` was **not** `get_pixel`/abstraction-bound (swapping
`img.get_pixel` for raw-slice indexing moved it <0.5 ms), and `locate_finders` is now **65% capped
flood fills** — the crosscheck the earlier S2 work targeted is already cheap (1.9 ms).

### S0. `prepare` was pixel-indexed, not block-tiled — DONE
Both hot passes looped in global raster order and did a per-pixel indexed **read-modify-write into
the heap `stats` array** (accumulate) / per-pixel `x >> block_pow` + `threshold[idx][i]` reindex
(binarize). rxing's `HybridBinarizer` instead works **block-by-block**: `calculateBlackPoints`
accumulates each 8×8 block's sum/min/max into locals and stores once; `thresholdBlock` loads one
threshold per block and walks it with `offset += stride`. Restructuring qrism's two passes the same
way — accumulate into a local `[Stat;4]` and store once per block; hoist the block threshold out of
the inner loop — took **`prepare` 18.7 → 9.5 ms** (accumulate 9.9 → 2.2, a 4.5× drop), median
**59.1 → 49.9 ms**, accuracy **789 → 789** (bit-identical binarization, all 148 lib tests pass).
The `prepare` signature changed from `GenericImageView` to `&ImageBuffer<P, Vec<u8>>` so the block
loop can index the raw byte slice (`img.as_raw()`); every caller already passes an `ImageBuffer`.
Remaining `prepare` floor is ~6.3 ms just to *read* the source luminance twice under parallel L2
contention (the `buffer.put` RMW is only ~0.5–1.4 ms of binarize; batching bit-writes per word is
the only lever left and it's small). Independent of S1 — this is loop structure, not buffer size.

### S1. `Pixel` is 16 bytes → 22.9–45.5 MB image buffer — DONE (BitMatrix landed)
**Superseded:** the buffer is now a 1-bit (B&W) / 4-bit (color) `BitMatrix` plane plus a separate
`Vec<u16>` label plane (`px_reg`), exactly the split recommended below. Original analysis kept for
context.

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

**Follow-up (capped flood fills):** profiling `verify_and_mark_finder` after the above split
`locate_finders` (43.5 ms) into horizontal scan **7.5 ms (17%)**, vertical crosschecks **11.4 ms
(27%)**, and the post-crosscheck **flood-fill + region checks 24 ms (56%)**. So the crosscheck is
*not* the bottleneck — the fills are: ~1030 candidates/image pass the crosscheck but aren't finders,
and the stone/ring `get_region` fills **3.7 M px/image** (2.6× the image), dominated by spurious
"stones" that bleed into big background blobs. Fix: `get_region_capped` bounds each fill and, on
overflow, relabels the visited pixels with a reserved `OVERSIZED_LABEL` sentinel — cached so later
capped fills skip the blob, but invisible to `get_region_id`, so it never masquerades as a region
(uncapped fills in symbol location reclaim it, so it can't leak). Caps: stone `max_run²`, ring
`10·stone_area` — both **row-stable**, which the sentinel memo requires (an earlier `extent²` stone
cap used the *vertical* crosscheck extent, which varies per row, so a under-measured row cached a
wrong "oversized" verdict and lost finders). Measured: `locate_finders` **43.5 → 25.4 ms**, overall
median **79.4 → 62.7 ms**, fill px **3.7 M → 1.6 M**, accuracy **789 → 789** (lossless). Dead ends
worth remembering: (i) memoising the *crosscheck* per column saved only ~0.6 ms — crosschecks are
cheap fast-fails; (ii) rolling back labels on overflow instead of the sentinel is correct but
re-fills the blob per candidate, erasing the win; (iii) keeping the partial fill labelled with a
real region id (no sentinel) corrupts nearby finders' region lookups (lost 10).

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

### S3. `group_finders` is O(n³) with no pruning — DONE (scale gates)
Triple-nested loop over all finder candidates, with `acos` + two `sqrt` in the innermost loop,
returning every passing triple and sorting them all. Measured on `lots`: **180 finders → 2.9 M
triples → 828,797 groups** (28% of all triples survive as allocated `FinderGroup`s), costing ~50 ms
for grouping and ~70 ms more in `locate_symbols` walking those groups.

rxing's `GenerateFinderPatternSets` is effectively **O(n)**: sort by size, **spatially bin** centres,
spiral only over nearby bins within `max_dist`, and **cap candidates at 15** with a `break`. It also
accepts a tighter 60°–120° angle window and compares **cosines directly against precomputed
`cos(60°)`/`cos(120°)`** — no `acos` at all — plus size-ratio and module-count gates.

**Resolution (this branch):** the missing ingredient was *scale* — a candidate was a bare `Point`,
so `group_finders` couldn't reject a pair whose separation is an impossible module count or whose two
finders are different sizes, and the only gates (symmetry + angle) are scale-free, letting 28% of all
cross-symbol triples through. Landed: (a) `locate_finders` now returns `Finder { c, mod_size }`, with
`mod_size = sqrt(stone.area / 9)` (the stone is the central 3×3-module block — free, already
computed); (b) `group_finders` builds a per-vertex **arm list** filtered by a **size-ratio gate**
(`MOD_SIZE_RATIO = 2.0`) and a **module-count / max_dist gate** (centre-to-centre span
∈ [10, 185]·`mod_size`, loosened from the true [14, 170] so no real symbol is clipped), then pairs
only those arms; (c) the angle is gated on the **cosine** (`dot² ≤ COS_45_SQ·|ab|²·|cb|²`, no `acos`
in the reject path), and the exact `acos` score is computed **only on survivors**, so the sort order
— and thus `locate_symbols`' greedy selection — is bit-identical. `FinderGroup.finders` stays
`[Point; 3]`, so nothing downstream changed. No spatial grid was needed: with n in the low hundreds,
the O(n²) arm scans (~32 K pairs on `lots`) are already negligible once the gates shrink the emitted
groups. Measured (536 images, rayon, M4): `group_finders` **1.22 → 0.37 ms/image** (−70%),
`locate_symbols` **3.95 → 3.54 ms/image** (fewer junk groups to `locate`), **`lots` median 312 → 215
ms** (−31%), overall median **46.1 → 38.5 ms**, accuracy **789 → 789** (lossless), all 148 lib tests
pass. Not done (unnecessary at this n): the spatial grid, sort-by-size, and the hard candidate cap —
noted as future steps only if candidate counts reach the thousands.

### S4. Finder flood fills are now the `locate_finders` ceiling — IN PROGRESS (low-risk step DONE)

**Update — cheaper fill inner loop landed.** `fill_and_accumulate` compared colours by converting
every scanned pixel to a `Color` enum (`self.get` → `BitMatrix::get` + `elem_bits` branch + enum
construction + `Option`); it now compares packed bits straight from the `BitMatrix`
(`self.buffer.get(x,y) == clr_bits`), since the fill only ever tests colour equality and equal
colour ⟺ equal bits. Structure-preserving — same pixels filled, caps/sentinel untouched. Measured:
`locate_finders` **20.4 → 17.9 ms**, and `locate_symbols` (which shares the uncapped fill path)
**4.2 → 3.7 ms**, median total **49.9 → 43.7 ms**, accuracy **789 → 789** (lossless, all 148 lib
tests pass). The remaining fill cost is now the raw pixel-visit count; further wins need the gate or
the rearchitecture below. Original analysis:

With prepare halved (S0) and the crosscheck cheap (S2), `locate_finders` (20.4 ms) is **68% the two
capped flood fills** in `verify_and_mark_finder` — the stone `get_region_capped((s,y))` and ring
`get_region_capped((r,y))` together are **13.9 ms/image**. Every candidate that clears the horizontal
ratio + vertical crosscheck pays two BFS fills over `px_reg`/`BitMatrix`, and most candidates are not
finders, so the fills run far more often than the ~3 real finders/symbol. The capped-fill +
`OVERSIZED_LABEL` memo (S2 follow-up) already stopped these from filling whole background blobs; what
remains is the sheer *count* of fills on legitimate small dark blobs.

**How rxing avoids it entirely:** rxing's `FinderPatternFinder` never flood-fills. It confirms a
1:1:3:1:1 row run with cheap **pixel-walk crosschecks** (`crossCheckVertical`/`Horizontal`, and a
diagonal check) that count runs along a line — O(module size), not O(area) — then **clusters centres
by proximity** (`foundPatternCross` + `haveMultiplyConfirmedCenters`) instead of measuring
stone/ring areas by connected component. qrism's area-ratio test (stone ≈ 37.5% of ring, and
ring/stone-not-connected) is what buys its precision 1.00, and that test needs the fills.

Fix directions, cheapest first:
- ~~**Cheaper per-pixel fill inner loop.**~~ **DONE** (see update above).
- **Gate fills behind a diagonal crosscheck** — **DONE.** See the funnel + gate results below.
- ~~**Gate fills behind a size sanity check**~~ (h vs v module size) — **TRIED, REJECTED.** See below.
- **Bigger:** adopt rxing's fill-free crosscheck + centre-clustering and keep the area-ratio test only
  as a final confirmation on the ~handful of clustered centres. This removes fills from the hot path
  entirely but is a real rearchitecture of `verify_and_mark_finder`.

**Follow-up — profiled the fill funnel, added a diagonal gate, rejected the size gate.**
Instrumenting `verify_and_mark_finder` (avg per image): **6171 datums** (1:1:3:1:1 rows) → 6109 past
the `is_finder` early-exit → **1095 past the vertical crosscheck** → 374 past the stone fill → 333
past the ring fill → **6.2 confirmed**. Phase timing: **stone fill 10.1 ms (52% of the stage)**,
scan 3.3, vertical crosscheck 1.9, ring fill 1.3, everything else ~0. So the cost is the *stone*
fill running on ~1095 mostly-spurious candidates; the ring fill and the area arithmetic are cheap.

Two pre-fill gates were tried on those 1095 candidates:
- **Diagonal crosscheck (kept).** `verify_finder_diagonal` confirms the 1:1:3:1:1 ratio along the
  main diagonal through the estimated centre — a third independent axis, O(module size). Diagonal
  run lengths are noisier than axis-aligned ones, so it uses `matches_finder_ratio_scaled` with
  `DIAGONAL_TOLERANCE_SCALE = 2.0` (mirrors rxing's looser `foundPatternDiagonal`). Back-to-back A/B:
  `locate_finders` **≈19.7 → ≈17.1 ms (−2.5, ~13%)**, accuracy **789 → 789** (lossless). Tightening
  the scale filters more but starts dropping real finders (scale 1.5 → 788, scale 1.0 → 785); the
  speedup and the finder loss rise together, so 2.0 is the lossless knee.
- **Size gate (rejected).** Comparing horizontal (`(r-l)/6`) vs vertical (`(b-t+1)/7`) module size and
  rejecting non-square candidates: measured **~0 ms speedup and −4 finders**. The spurious candidates
  that reach the stone fill are roughly *square* dark blobs, so squareness doesn't discriminate them —
  it only clips real finders skewed by perspective/rotation. Not worth it.

Net: the diagonal gate is a modest lossless win, but confirms the ceiling — the ~1095 blobs that
clear the vertical crosscheck are genuinely finder-*shaped*, so cheap axis gates can't cull many
without the multi-row **voting/clustering** rxing uses (a candidate hit on only one row never reaches
quorum). Halving the fills needs that rearchitecture, not another gate.

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

1. ~~**S1** (Pixel 16 B → 1 bit / byte-plane split).~~ **DONE** (BitMatrix + `px_reg` label plane).
2. ~~**S2** (finder tolerance: tighten + decouple bar/space + row-skip).~~ **DONE.**
3. ~~**S0** (block-tile `prepare`'s two passes, rxing-style).~~ **DONE** — `prepare` 18.7 → 9.5 ms.
4. **S4** (kill/cheapen finder flood fills) — cheaper fill loop DONE (−2.5 ms) + diagonal gate DONE
   (−2.5 ms); size gate rejected. Halving the fills now needs the fill-free voting/clustering
   rearchitecture — the biggest speed item left.
5. ~~**S3** (group_finders: bin + cap + gate + drop `acos`)~~ **DONE** — scale gates (size-ratio +
   module-count) + cosine reject; `group_finders` 1.22 → 0.37 ms, `lots` median 312 → 215 ms.
6. **A1 + A2** (module-proportional block size + re-enable low-variance rule) — the binarizer.
7. **A4** (multi-alignment-pattern piecewise sampling) — the real high-version accuracy fix.

Priorities 3–5 are pure speed and mostly independent; 6–7 are the accuracy story.
