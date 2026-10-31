# HiQ classifiers on qrism's samples

What HiQ's shipped colour classifiers consume, read from `~/Projects/hiq/HiQ/hiq-decoder`
(commit `1456970`) before any of the port was written. File references are to that tree.
The port is `recovery/hiq.rs`, `normalization/hiq_white.rs` and the `hiq …` rows of
`analysis.rs`; parameters are in `hiq_models.txt`, exported by `hiq_export.py`.

## Inputs

**Colour space.** Raw 8-bit sRGB as `ImageIO.read` + `BufferedImage.getRGB` return it. No
linearization, no gamma, no YUV (the YUV paths in `RGBColorWrapper` are commented out).

**Module value.** One pixel, not an average. `DefaultGridSampler.colorSampleGrid` maps the
module centre `(x + 0.5, y + 0.5)` through the perspective transform and rounds with
`(int)(p + 0.5f)`. qrism's `sample_module_rgb` is also one pixel at the module centre,
rounded, so the two take the same kind of sample, at the positions each localizer finds.

**Normalization.** Per-channel division by a white point, nothing else:
`feature_k = (pixel_k & 0xff) / white[k]`. `white` is a `float[]`, so the division is done
in single precision and then widened to `double` (`RGBColorWrapper.colorClassify`). No
clipping, no black point.

**White point** (`DetectorColor.estimateWhiteProjection` → `DefaultGridSampler.getWhiteSamplePoints`
→ `RGBColorWrapper.estimateWhiteRGB(int[])`). Twelve single pixels at the module centres
of the corners of each finder's white ring: grid `(x, y)` =
`(1,1) (1,5) (5,1) (5,5)`, `(d-2,1) (d-2,5) (d-6,1) (d-6,5)`, `(1,d-2) (5,d-2) (1,d-6) (5,d-6)`
for a `d`-module symbol. A sample is dropped if HiQ's binarizer marks it black; the rest
are averaged in `float`. No surviving sample gives `(255, 255, 255)`. Versions 21 to 40
always take this path (the cached-transform branch needs fewer than four alignment centres).

**Binarizer** (`RGBColorWrapper.rgb2Binary`). The image is cut into an 8×8 grid of blocks
of `width/8 + 1` by `height/8 + 1` pixels. Per block and channel the threshold is
`(int)((max + min)/2.0 + 0.5)`. A pixel is black if `R <= tR || G <= tG || B <= 0.95·tB`.

## Classifiers

**QDA** (`color/QDA.java`, model `candidate_classifiers/Qmodel_CMI.txt`, identical to
`models/model_QmodelCMI.txt`). File layout: `classNum dim`; group labels; log priors; one
mean per class; per class a 3×3 matrix that is already the *inverse* covariance
(`saveModel` writes `pooledInverseCovMats`); one `log(det Σ)/2` per class. Score
`g_c = logprior_c − ½ dᵀ Σ_c⁻¹ d − halflogdet_c`, `d = x − μ_c`; the arg max wins, first
class on ties (strict `>`). The group label is the colour with bit 2 = R, bit 1 = G,
bit 0 = B, 1 = channel lit, which is exactly qrism's `Color` encoding. A trailing line of
CMI weights in the model file is not read by the parser.

**CMI** (`color/CMIModel.java`, weights `candidate_classifiers/Qmodel_CMI_coef.txt`:
`0.9993 −0.0266 −0.0150 −0.0192 −0.0060`). The 15-value input is five RGB triples in slot
order centre, left, right, up, down (`colorSampleGrid`). The cancelled feature is
`x_k = Σ_m w_m · v[3m + k]`, fed to the QDA above.

- As shipped, `cancelCMI` bounds the module loop by `module_weights.length` (1 row) and so
  keeps only `0.9993 · centre`. The HiQ bench's `qda+cmi-fix` (`bench/src/bench/CMIModelFixed.java`)
  sums all five; that is what `hiq qda+cmi fixed` ports.
- The sampler has a second slip the bench kept: the neighbour gathering uses `else if`, so
  for an interior module only left and up are fetched. Right and down stay at pixel
  `(0, 0)`, which the `x <= 0` guard in `colorClassify` turns into a zero triple. Right is
  fetched only in column 0 (where left is zero) and down only in row 0. So HiQ's end-to-end
  `qda+cmi-fix` numbers ran with two of the four neighbour terms zero on almost every module.
- Border handling as written: a neighbour outside the symbol is a zero triple (the `{0,0}`
  default), not a quiet-zone sample.

The port reads all four edge neighbours from qrism's sampled grid, zero outside the symbol
(`hiq qda+cmi fixed`). A second row, `hiq qda+cmi hiq nbrs`, applies HiQ's shipped
neighbour rule to the same grid, so the effect of that slip can be separated from the
classifier.

**Layered SVM** (`color/LayeredSVM.java`, model
`candidate_classifiers/layeredSVMs_ip6plus_mixed_new_aug.txt`, the `lsvm-ip6plus` row of
Table V). One linear SVM per layer on the 3-value feature: `y = w·φ(x) − b`, channel dark
if `y < 0`. `φ` is the identity for 3 weights, the quadratic expansion `convt2poly` for 10
and the cubic `convt2poly3` for 20. In this model R and G are linear and B is cubic. The
parser reads the first three layers and ignores the text after them. The LSVM+CMI model is
not ported: its feature layout is undocumented and its reconstruction scores 59.7% module
accuracy in HiQ's own bench (`bench/README.md`).

## Where the port cannot match HiQ exactly

- **Sample positions.** HiQ's models were trained on HiQ's own sampling; here they get
  qrism's homography, so module centres and the twelve white samples land where qrism's
  localizer puts them. That is the point of the comparison, but it means the classifiers run
  off the distribution they were trained on.
- **JPEG decoding.** qrism decodes with `image` (zune-jpeg), HiQ with Java ImageIO. The
  parity pass measures how often the two disagree on a pixel.
- **Nudging.** `checkAndNudgePoints` moves a point that falls just outside the image back
  onto it; qrism clamps. Only matters for symbols touching the image edge.
