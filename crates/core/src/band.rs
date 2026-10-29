use crate::math::{cos, log10, pow, sin};

const FRAME: usize = 512;
const STEP: usize = 256;
// 4625 Hz; the power from there up against the total, in dB, averaged over a second of sound: -87 at most for a phone line brought to 16 kHz, -71 at least for wideband
const HIGH_BIN: usize = 148;
const NARROW_DB: f64 = -78.0;
// -65 dBFS
const SOUND: f64 = 3.2e-7;
const MIN_FRAMES: usize = 8;
const AVG_FRAMES: f64 = 62.0;
const NOISE_DB: f64 = -45.0;
// Butterworth high-pass of order 8 at 4 kHz: at a quarter of the rate each section's a1 is 0
const HP_GAIN: f64 = 0.009267285584088781;
const HP_A2: [f64; 4] = [0.009700556535263622, 0.0920192104555731, 0.2857021544554057, 0.673513677715992];

// Focus and Cue never heard a band empty over 4 kHz, as a phone line leaves it, and lose the voice: there they get a faint noise
pub(crate) struct Band {
    ring: Vec<f32>,
    at: usize,
    since: usize,
    db: f64,
    frames: usize,
    narrow: bool,
    amp: f64,
    seed: u32,
    hp: [[f64; 4]; 4],
    window: Vec<f64>,
    cos: Vec<f64>,
    sin: Vec<f64>,
    re: Vec<f64>,
    im: Vec<f64>,
}

impl Band {
    pub(crate) fn new() -> Band {
        let w = |k: usize| 2.0 * std::f64::consts::PI * k as f64 / FRAME as f64;
        Band {
            ring: vec![0.0; FRAME],
            at: 0,
            since: 0,
            db: 0.0,
            frames: 0,
            narrow: false,
            // uniform noise has a power of 1/3, and the high-pass keeps half of it
            amp: pow(10.0, NOISE_DB / 20.0) * pow(6.0, 0.5),
            seed: 1,
            hp: [[0.0; 4]; 4],
            window: (0..FRAME).map(|k| 0.5 - 0.5 * cos(w(k))).collect(),
            cos: (0..FRAME / 2).map(|k| cos(w(k))).collect(),
            sin: (0..FRAME / 2).map(|k| sin(w(k))).collect(),
            re: vec![0.0; FRAME],
            im: vec![0.0; FRAME],
        }
    }

    pub(crate) fn apply(&mut self, pcm: &mut [f32]) {
        for x in pcm {
            self.ring[self.at] = *x;
            self.at = (self.at + 1) % FRAME;
            self.since += 1;
            if self.since == STEP {
                self.since = 0;
                self.frame();
            }
            if self.narrow {
                *x = (*x as f64 + self.noise()) as f32;
            }
        }
    }

    pub(crate) fn reset(&mut self) {
        *self = Band::new();
    }

    fn frame(&mut self) {
        let mut sq = 0.0;
        for &x in &self.ring {
            sq += x as f64 * x as f64;
        }
        if sq / (FRAME as f64) <= SOUND {
            return;
        }
        let (high, total) = self.spectrum();
        self.frames += 1;
        let k = if (self.frames as f64) < AVG_FRAMES { 1.0 / self.frames as f64 } else { 1.0 / AVG_FRAMES };
        self.db += (10.0 * log10(high / total + 1e-15) - self.db) * k;
        self.narrow = self.frames >= MIN_FRAMES && self.db < NARROW_DB;
    }

    // the power from HIGH_BIN up and in all, the frame windowed
    fn spectrum(&mut self) -> (f64, f64) {
        let bits = FRAME.trailing_zeros();
        for i in 0..FRAME {
            let j = (i as u32).reverse_bits() >> (32 - bits);
            self.re[j as usize] = self.ring[(self.at + i) % FRAME] as f64 * self.window[i];
            self.im[j as usize] = 0.0;
        }
        let mut len = 2;
        while len <= FRAME {
            let (half, stride) = (len / 2, FRAME / len);
            for s in (0..FRAME).step_by(len) {
                for k in 0..half {
                    let (c, n) = (self.cos[k * stride], -self.sin[k * stride]);
                    let (a, b) = (s + k, s + k + half);
                    let tr = self.re[b] * c - self.im[b] * n;
                    let ti = self.re[b] * n + self.im[b] * c;
                    self.re[b] = self.re[a] - tr;
                    self.im[b] = self.im[a] - ti;
                    self.re[a] += tr;
                    self.im[a] += ti;
                }
            }
            len *= 2;
        }
        let (mut high, mut total) = (0.0, 0.0);
        for k in 0..=FRAME / 2 {
            let p = self.re[k] * self.re[k] + self.im[k] * self.im[k];
            total += p;
            if k >= HIGH_BIN {
                high += p;
            }
        }
        (high, total)
    }

    fn noise(&mut self) -> f64 {
        self.seed = self.seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let mut x = (self.seed as f64 / 4294967296.0 * 2.0 - 1.0) * HP_GAIN;
        for (s, a2) in self.hp.iter_mut().zip(HP_A2) {
            let y = x - 2.0 * s[0] + s[1] - a2 * s[3];
            *s = [x, s[0], y, s[2]];
            x = y;
        }
        x * self.amp
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lcg(n: usize, seed: u32, scale: f64) -> Vec<f32> {
        let mut s = seed;
        (0..n)
            .map(|_| {
                s = s.wrapping_mul(1664525).wrapping_add(1013904223);
                ((s as f64 / 4294967296.0 * 2.0 - 1.0) * scale) as f32
            })
            .collect()
    }

    // tones from 300 Hz to 3.1 kHz: a phone band, nothing above 4.6 kHz but the window's far lobes
    fn phone(n: usize) -> Vec<f32> {
        let tones = [(300.0, 0.05), (700.0, 0.03), (1300.0, 0.02), (2100.0, 0.01), (3100.0, 0.005)];
        (0..n).map(|i| tones.iter().map(|&(f, a)| a * sin(2.0 * std::f64::consts::PI * f * i as f64 / 16000.0)).sum::<f64>() as f32).collect()
    }

    fn power_db(x: &[f32]) -> f64 {
        10.0 * (x.iter().map(|&v| v as f64 * v as f64).sum::<f64>() / x.len() as f64).log10()
    }

    #[test]
    fn wideband_untouched() {
        let x = lcg(16000 * 3, 3, 0.05);
        let mut y = x.clone();
        let mut b = Band::new();
        b.apply(&mut y);
        assert!(!b.narrow);
        assert_eq!(x, y);
    }

    #[test]
    fn silence_untouched() {
        let mut y = vec![0.0f32; 16000];
        let mut b = Band::new();
        b.apply(&mut y);
        assert!(!b.narrow && y.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn phone_band_gets_noise_over_4k() {
        let x = phone(16000 * 3);
        let mut y = x.clone();
        let mut b = Band::new();
        b.apply(&mut y);
        assert!(b.narrow);
        let added: Vec<f32> = y.iter().zip(&x).skip(16000).map(|(a, b)| a - b).collect();
        let db = power_db(&added);
        assert!((db - NOISE_DB).abs() < 0.5, "{db}");
        let mut spec = Band::new();
        spec.ring.copy_from_slice(&added[..FRAME]);
        spec.at = 0;
        let (high, total) = spec.spectrum();
        assert!(high / total > 0.9);
    }

    #[test]
    fn same_output_whatever_the_blocks() {
        let x = phone(16000 * 2);
        let mut whole = x.clone();
        Band::new().apply(&mut whole);
        for size in [1, 37, 256, 480] {
            let mut y = x.clone();
            let mut b = Band::new();
            for c in y.chunks_mut(size) {
                b.apply(c);
            }
            assert_eq!(whole, y, "{size}");
        }
    }

    #[test]
    fn back_to_wideband() {
        let mut y = phone(16000 * 2);
        let mut b = Band::new();
        b.apply(&mut y);
        assert!(b.narrow);
        let mut w = lcg(16000 * 3, 5, 0.05);
        b.apply(&mut w);
        assert!(!b.narrow);
    }
}
