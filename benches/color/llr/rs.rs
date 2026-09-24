//! Reed-Solomon replay: the real QR block layout, driven by recorded bit reliabilities.
//!
//! A layer's codewords are split into blocks and interleaved before placement, so a stream
//! byte `j` lands at some position of some block. [`RsLayout`] rebuilds that map from the
//! version tables, which is all that is needed to turn a recorded per-bit error pattern back
//! into per-block received words. Everything is decoded by the library's own
//! errors-and-erasures decoder through `bench_hooks`, so the replay is exactly the decoder the
//! reader runs — only the choice of *what to erase* differs between the variants here.

use qrism::bench_hooks::{rs_decode, rs_encode};
use qrism::{ECLevel, Version};
use rand::rngs::StdRng;
use rand::Rng;

use super::{BlockOutcome, Frame};

/// Fractions of a block's parity to erase, tried in order. Matches the reader's "ladder fine".
const LADDER_FINE: [f64; 9] = [0.0, 0.125, 0.25, 0.375, 0.5, 0.625, 0.75, 0.875, 1.0];

/// The shorter ladder each Chase test pattern runs, since there are 2^τ of them.
const LADDER_COARSE: [f64; 5] = [0.0, 0.25, 0.5, 0.75, 1.0];

/// How many of a block's least reliable bytes Chase tries both values of.
const CHASE_TAU: usize = 5;

/// One layer's block structure and where each stream byte lands in it.
pub(crate) struct RsLayout {
    /// `(data length, total length)` per block.
    pub(crate) blocks: Vec<(usize, usize)>,
    /// Stream codeword index -> `(block, position within block)`.
    pub(crate) map: Vec<(usize, usize)>,
    /// Data bits the layer carries, for rate-matching other codes to it.
    pub(crate) data_bits: usize,
}

impl RsLayout {
    pub(crate) fn new(ver: Version, ecl: ECLevel) -> Self {
        let (b1s, b1c, b2s, b2c) = ver.data_codewords_per_block(ecl);
        let ec_len = ver.ecc_per_block(ecl);
        let dlens: Vec<usize> =
            std::iter::repeat_n(b1s, b1c).chain(std::iter::repeat_n(b2s, b2c)).collect();
        let blocks: Vec<(usize, usize)> = dlens.iter().map(|&d| (d, d + ec_len)).collect();

        // Mirrors the builder's interleave: data columns across blocks, then parity columns.
        let mut map = Vec::with_capacity(ver.channel_codewords());
        let max_d = dlens.iter().copied().max().unwrap_or(0);
        for i in 0..max_d {
            for (b, &d) in dlens.iter().enumerate() {
                if i < d {
                    map.push((b, i));
                }
            }
        }
        for i in 0..ec_len {
            for (b, &d) in dlens.iter().enumerate() {
                map.push((b, d + i));
            }
        }
        debug_assert_eq!(map.len(), ver.channel_codewords());

        RsLayout { blocks, map, data_bits: dlens.iter().sum::<usize>() * 8 }
    }
}

/// One block as the decoder would receive it.
struct RxBlock {
    sent: Vec<u8>,
    dlen: usize,
    rx: Vec<u8>,
    /// Per byte, per bit (MSB first): LLR in favour of the received bit.
    rel: Vec<[f64; 8]>,
}

impl RxBlock {
    fn ec_len(&self) -> usize {
        self.rx.len() - self.dlen
    }

    fn byte_errors(&self) -> usize {
        let cw = rs_encode(&self.sent, self.ec_len());
        cw.iter().zip(&self.rx).filter(|(a, b)| a != b).count()
    }

    fn judge(&self, got: Option<Vec<u8>>) -> BlockOutcome {
        match got {
            Some(d) if d == self.sent => BlockOutcome::Ok,
            Some(_) => BlockOutcome::Wrong,
            None => BlockOutcome::Failed,
        }
    }
}

/// Builds every block of a frame: random data, encoded, then hit with the recorded hard-error
/// pattern. The data is random so a decoder that returns something fixed cannot pass.
fn receive(frame: &Frame, layout: &RsLayout, rng: &mut StdRng) -> Vec<RxBlock> {
    let mut blks: Vec<RxBlock> = layout
        .blocks
        .iter()
        .map(|&(dlen, len)| {
            let sent: Vec<u8> = (0..dlen).map(|_| rng.random()).collect();
            let rx = rs_encode(&sent, len - dlen);
            RxBlock { sent, dlen, rx, rel: vec![[0.0; 8]; len] }
        })
        .collect();

    for (j, &(b, p)) in layout.map.iter().enumerate() {
        for t in 0..8 {
            let i = j * 8 + t;
            if frame.hard_err[i] {
                blks[b].rx[p] ^= 0x80 >> t;
            }
            blks[b].rel[p][t] = frame.a[i];
        }
    }
    blks
}

/// How a block's bytes are ranked for erasure, least trustworthy first.
#[derive(Clone, Copy)]
pub(crate) enum Ranking {
    /// The weakest bit decides — the reader's current rule.
    MinBit,
    /// Probability the byte is wrong, from all eight bits. Sees a byte with several weak bits.
    Prob,
}

fn byte_score(rel: &[f64; 8], rank: Ranking) -> f64 {
    match rank {
        Ranking::MinBit => rel.iter().copied().fold(f64::INFINITY, f64::min),
        // log P(byte correct) = -sum softplus(-a); higher is more trustworthy.
        Ranking::Prob => -rel.iter().map(|&a| softplus(-a)).sum::<f64>(),
    }
}

fn softplus(x: f64) -> f64 {
    if x > 30.0 {
        x
    } else {
        x.exp().ln_1p()
    }
}

/// Erase-weakest ladder over one received word, returning the first decode whose parity
/// clears — the same acceptance rule the reader uses.
fn gmd(rx: &[u8], rel: &[[f64; 8]], dlen: usize, rank: Ranking, ladder: &[f64]) -> Option<Vec<u8>> {
    let ec_len = rx.len() - dlen;
    let mut order: Vec<usize> = (0..rx.len()).collect();
    let score: Vec<f64> = rel.iter().map(|r| byte_score(r, rank)).collect();
    order.sort_by(|&a, &b| score[a].total_cmp(&score[b]));

    for &frac in ladder {
        let n = ((frac * ec_len as f64).floor() as usize).min(order.len());
        let mut erased = vec![false; rx.len()];
        for &i in &order[..n] {
            erased[i] = true;
        }
        if let Some(d) = rs_decode(rx, dlen, &erased) {
            return Some(d);
        }
    }
    None
}

/// GMD, falling back to Chase-II when it gives up: the τ least reliable bytes each also try
/// the value their weakest bit flipped would give, every combination runs a coarse GMD
/// ladder, and the candidate closest to the soft observation wins.
fn chase(blk: &RxBlock) -> Option<Vec<u8>> {
    if let Some(d) = gmd(&blk.rx, &blk.rel, blk.dlen, Ranking::Prob, &LADDER_FINE) {
        return Some(d);
    }

    let score: Vec<f64> = blk.rel.iter().map(|r| byte_score(r, Ranking::Prob)).collect();
    let mut order: Vec<usize> = (0..blk.rx.len()).collect();
    order.sort_by(|&a, &b| score[a].total_cmp(&score[b]));
    let flips: Vec<(usize, usize)> = order
        .iter()
        .take(CHASE_TAU)
        .map(|&p| {
            let t = (0..8).min_by(|&x, &y| blk.rel[p][x].total_cmp(&blk.rel[p][y])).unwrap();
            (p, t)
        })
        .collect();

    let mut best: Option<(f64, Vec<u8>)> = None;
    for pat in 1u32..(1 << flips.len()) {
        let mut rx = blk.rx.clone();
        let mut rel = blk.rel.clone();
        for (k, &(p, t)) in flips.iter().enumerate() {
            if pat >> k & 1 == 1 {
                rx[p] ^= 0x80 >> t;
                rel[p][t] = -rel[p][t];
            }
        }
        let Some(d) = gmd(&rx, &rel, blk.dlen, Ranking::Prob, &LADDER_COARSE) else {
            continue;
        };
        // Soft distance: the reliability spent on every bit the candidate disagrees with.
        let cw = rs_encode(&d, blk.ec_len());
        let mut cost = 0.0;
        for (p, (&c, &r)) in cw.iter().zip(&blk.rx).enumerate() {
            let diff = c ^ r;
            for t in 0..8 {
                if diff & (0x80 >> t) != 0 {
                    cost += blk.rel[p][t];
                }
            }
        }
        if best.as_ref().is_none_or(|(c, _)| cost < *c) {
            best = Some((cost, d));
        }
    }
    best.map(|(_, d)| d)
}

/// The Reed-Solomon variants the replay compares.
#[derive(Clone, Copy)]
pub(crate) enum RsVariant {
    /// Error-only decode of the hard decisions. Must match the reader's plain decode.
    Hard,
    /// Erasure ladder, bytes ranked by `Ranking`.
    Gmd(Ranking),
    /// GMD by probability, then Chase-II on blocks it gives up on.
    Chase,
    /// Every wrong byte erased and nothing else: the ceiling for *any* erasure-flagging rule.
    Genie,
}

/// Decodes every block of one frame, returning each block's outcome and, for the failure
/// report, `(byte errors, parity length)` per block.
pub(crate) fn replay(
    frame: &Frame,
    layout: &RsLayout,
    variant: RsVariant,
    rng: &mut StdRng,
) -> (Vec<BlockOutcome>, Vec<(usize, usize)>) {
    let blks = receive(frame, layout, rng);
    let outcomes = blks
        .iter()
        .map(|b| match variant {
            RsVariant::Hard => b.judge(rs_decode(&b.rx, b.dlen, &vec![false; b.rx.len()])),
            RsVariant::Gmd(rank) => b.judge(gmd(&b.rx, &b.rel, b.dlen, rank, &LADDER_FINE)),
            RsVariant::Chase => b.judge(chase(b)),
            RsVariant::Genie => {
                if b.byte_errors() <= b.ec_len() {
                    BlockOutcome::Ok
                } else {
                    BlockOutcome::Failed
                }
            }
        })
        .collect();
    let anatomy = blks.iter().map(|b| (b.byte_errors(), b.ec_len())).collect();
    (outcomes, anatomy)
}
