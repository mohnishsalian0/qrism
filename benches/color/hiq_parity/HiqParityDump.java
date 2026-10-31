package bench;

import com.google.zxing.DecodeHintType;
import com.google.zxing.color.Classifier;
import com.google.zxing.color.RGBColorWrapper;
import com.google.zxing.common.BitMatrix;
import com.google.zxing.common.DetectorResult;
import com.google.zxing.common.GridSampler;
import com.google.zxing.common.PerspectiveTransformGeneral;
import com.google.zxing.qrcode.decoder.Decoder;
import com.google.zxing.qrcode.detector.DetectorColor;

import javax.imageio.ImageIO;
import java.awt.image.BufferedImage;
import java.io.File;
import java.io.PrintWriter;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.Map;

/**
 * Dumps what HiQ's colour classifiers are fed and what they decide, module by module, for the
 * parity pass in qrism's colour bench ({@code cargo bench --features benchmark --bench color --
 * hiq_parity}).
 *
 * <p>Lives in qrism ({@code benches/color/hiq_parity/}) and compiles against the HiQ bench's
 * classes, in its package so it can reuse {@link Configs}. For each photo and configuration it
 * runs the stock HiQ localization, then rebuilds every module's feature vector the way
 * {@code DefaultGridSampler.colorSampleGrid} and {@code RGBColorWrapper.colorClassify} do
 * (the same transcription {@code Timing} verifies) and records the classifier's decision next
 * to the decoder's own channel bits, so the dump is tied to what the decoder really produced.
 * It also records the twelve white-point samples with HiQ's binarizer verdict on each.
 *
 * <pre>
 *   javac -cp OUT:LIBS -d build HiqParityDump.java
 *   java  -cp build:OUT:LIBS bench.HiqParityDump --root HIQ_DECODER --out DIR photo...
 * </pre>
 */
public final class HiqParityDump {

    private static final String[] CONFIGS = {"qda", "qda+cmi-fix", "lsvm-ip6plus"};

    public static void main(String[] args) throws Exception {
        String root = ".";
        String out = ".";
        int i = 0;
        for (; i < args.length && args[i].startsWith("--"); i += 2) {
            if (args[i].equals("--root")) {
                root = args[i + 1];
            } else if (args[i].equals("--out")) {
                out = args[i + 1];
            }
        }
        List<Configs.Config> all = Configs.build(root);
        try (PrintWriter mods = new PrintWriter(Files.newBufferedWriter(Path.of(out, "modules.csv")));
             PrintWriter whites = new PrintWriter(Files.newBufferedWriter(Path.of(out, "white.csv")))) {
            mods.println("photo,config,dim,y,x,px,py,r,g,b,features,pred,decoder");
            whites.println("photo,config,white_r,white_g,white_b,i,px,py,r,g,b,black");
            for (; i < args.length; i++) {
                File photo = new File(args[i]);
                for (String name : CONFIGS) {
                    Classifier c = all.stream().filter(x -> x.name.equals(name)).findFirst()
                            .orElseThrow().classifier;
                    String status = dump(photo, name, c, mods, whites);
                    System.out.printf("%-40s %-14s %s%n", photo, name, status);
                }
            }
        }
    }

    private static String dump(File file, String name, Classifier c, PrintWriter mods,
                               PrintWriter whites) throws Exception {
        BufferedImage img = ImageIO.read(file);
        int w = img.getWidth();
        int h = img.getHeight();
        int[] pixels = new int[w * h];
        img.getRGB(0, 0, w, h, pixels, 0, w);

        RGBColorWrapper wrapper = new RGBColorWrapper(pixels, w, h, 0, 0, w, h, false, c, null, 3);
        BitMatrix black = wrapper.getBlackMatrix();
        DetectorColor detector = new DetectorColor(black, wrapper, 3);
        DetectorResult rough = detector.detect((Map<DecodeHintType, ?>) null);
        if (rough == null || rough.getBits() == null) {
            return "no symbol";
        }
        int dim = rough.getBits().getWidth();
        int corrected = Decoder.checkDimension(rough.getBits());
        if (corrected > 0) {
            dim = corrected;
        }
        DetectorResult refined = detector.detect(rough, dim);
        if (refined == null || refined.getChannelBits() == null) {
            return "no symbol";
        }
        BitMatrix[] channels = refined.getChannelBits();
        PerspectiveTransformGeneral transform = refined.getTransform();
        int gdim = channels[0].getWidth();
        float[] white = detector.getWhite();

        // White point: the sample positions and binarizer verdicts the decoder used, and a
        // recomputation of the mean, which must equal the detector's white exactly.
        int[] pts = GridSampler.getInstance().getWhiteSamplePoints(gdim, wrapper, transform,
                rough.getModuleSize());
        float[] mean = {0, 0, 0};
        int count = 0;
        for (int k = 0; k < pts.length; k += 2) {
            int px = pts[k];
            int py = pts[k + 1];
            if (px < 0 || px >= w || py < 0 || py >= h) {
                continue;
            }
            int p = pixels[py * w + px];
            int r = (p >> 16) & 0xff;
            int g = (p >> 8) & 0xff;
            int b = p & 0xff;
            boolean isBlack = black.get(px, py);
            if (!isBlack) {
                mean[0] += r;
                mean[1] += g;
                mean[2] += b;
                count++;
            }
            whites.printf("%s,%s,%s,%s,%s,%d,%d,%d,%d,%d,%d,%d%n", file, name, white[0], white[1],
                    white[2], k / 2, px, py, r, g, b, isBlack ? 1 : 0);
        }
        if (count > 0) {
            for (int k = 0; k < 3; k++) {
                mean[k] /= count;
            }
        } else {
            mean = new float[] {255f, 255f, 255f};
        }
        boolean whiteOk = mean[0] == white[0] && mean[1] == white[1] && mean[2] == white[2];

        // Module centres, as colorSampleGrid computes them.
        int[][] cx = new int[gdim][gdim];
        int[][] cy = new int[gdim][gdim];
        float[] points = new float[gdim << 1];
        for (int y = 0; y < gdim; y++) {
            for (int k = 0; k < points.length; k += 2) {
                points[k] = (k >> 1) + 0.5f;
                points[k + 1] = y + 0.5f;
            }
            transform.transformPoints(points);
            for (int k = 0; k < points.length; k += 2) {
                cx[y][k >> 1] = clamp((int) (points[k] + 0.5f), w - 1);
                cy[y][k >> 1] = clamp((int) (points[k + 1] + 0.5f), h - 1);
            }
        }

        long mismatches = 0;
        for (int y = 0; y < gdim; y++) {
            for (int x = 0; x < gdim; x++) {
                double[] data = c.getCMIFlag()
                        ? features15(pixels, w, h, cx, cy, x, y, gdim, white)
                        : features3(pixels, w, h, cx[y][x], cy[y][x], white);
                boolean[] dark = c.predict(data);
                int pred = colour(!dark[0], !dark[1], !dark[2]);
                int dec = colour(!channels[0].get(x, y), !channels[1].get(x, y), !channels[2].get(x, y));
                if (pred != dec) {
                    mismatches++;
                }
                int p = pixels[cy[y][x] * w + cx[y][x]];
                StringBuilder f = new StringBuilder();
                for (int k = 0; k < data.length; k++) {
                    if (k > 0) {
                        f.append(' ');
                    }
                    f.append(data[k]);
                }
                mods.printf("%s,%s,%d,%d,%d,%d,%d,%d,%d,%d,%s,%d,%d%n", file, name, gdim, y, x,
                        cx[y][x], cy[y][x], (p >> 16) & 0xff, (p >> 8) & 0xff, p & 0xff, f, pred, dec);
            }
        }
        return String.format("v%d, %d modules, %d differ from the decoder, white recomputed %s",
                (gdim - 17) / 4, gdim * gdim, mismatches, whiteOk ? "exactly" : "DIFFERENTLY");
    }

    /** The single-sample path of {@code colorClassify}. */
    private static double[] features3(int[] pixels, int w, int h, int px, int py, float[] white) {
        double[] data = new double[3];
        if (px >= 0 && px < w && py >= 0 && py < h) {
            int p = pixels[py * w + px];
            data[0] = ((p >> 16) & 0xff) / white[0];
            data[1] = ((p >> 8) & 0xff) / white[1];
            data[2] = (p & 0xff) / white[2];
        }
        return data;
    }

    /** The five-sample path, with the sampler's neighbour gathering as shipped. */
    private static double[] features15(int[] pixels, int w, int h, int[][] cx, int[][] cy,
                                       int x, int y, int dim, float[] white) {
        int[] xs = new int[5];
        int[] ys = new int[5];
        xs[0] = cx[y][x];
        ys[0] = cy[y][x];
        if (x != 0) {
            xs[1] = cx[y][x - 1];
            ys[1] = cy[y][x - 1];
        } else if (x != dim - 1) {
            xs[2] = cx[y][x + 1];
            ys[2] = cy[y][x + 1];
        }
        if (y != 0) {
            xs[3] = cx[y - 1][x];
            ys[3] = cy[y - 1][x];
        } else if (y != dim - 1) {
            xs[4] = cx[y + 1][x];
            ys[4] = cy[y + 1][x];
        }
        double[] data = new double[15];
        for (int i = 0; i < 5; i++) {
            if (xs[i] <= 0 || xs[i] >= w || ys[i] <= 0 || ys[i] >= h) {
                continue;
            }
            int p = pixels[ys[i] * w + xs[i]];
            data[i * 3] = ((p >> 16) & 0xff) / white[0];
            data[i * 3 + 1] = ((p >> 8) & 0xff) / white[1];
            data[i * 3 + 2] = (p & 0xff) / white[2];
        }
        return data;
    }

    private static int colour(boolean r, boolean g, boolean b) {
        return (r ? 4 : 0) | (g ? 2 : 0) | (b ? 1 : 0);
    }

    private static int clamp(int v, int max) {
        return v < 0 ? 0 : Math.min(v, max);
    }
}
