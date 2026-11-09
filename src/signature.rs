//! The fingerprint Shazam reads: peaks of the sound's spectrum, found and written the way its own apps do.
//!
//! The algorithm is the one SongRec (GPL-3.0, like Noctorium) worked out from Shazam's Android app: the
//! sound at 16 kHz in one channel, a 2048-point FFT every 128 samples under a Hann window, each spectrum
//! spread over its neighbours in frequency and in time, and the points that stand out from everything around
//! them kept as peaks, in four bands. The peaks are then packed into Shazam's binary signature format, and
//! that is all that is ever sent: never the sound itself.

use rustfft::{num_complex::Complex, Fft, FftPlanner};
use std::sync::Arc;

pub const SAMPLE_RATE: u32 = 16_000;
/// What Shazam needs to be sure, and no more is sent.
pub const MAX_SECONDS: u32 = 12;

/// One peak: when (in FFT passes of 128 samples), how loud, and where (in 64ths of an FFT bin).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Peak {
    pub pass: u32,
    pub magnitude: u16,
    pub bin: u16,
}

/// The four frequency bands Shazam keeps peaks in: 250-520 Hz, 520-1450, 1450-3500 and 3500-5500.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Signature {
    pub samples: u32,
    pub bands: [Vec<Peak>; 4],
}

impl Signature {
    pub fn peak_count(&self) -> usize {
        self.bands.iter().map(Vec::len).sum()
    }

    /// The signature in Shazam's binary format.
    pub fn encode(&self) -> Vec<u8> {
        let mut out: Vec<u8> = Vec::with_capacity(48 + 8 + self.peak_count() * 5 + 32);
        let put = |out: &mut Vec<u8>, v: u32| out.extend_from_slice(&v.to_le_bytes());
        put(&mut out, 0xcafe_2580); // magic
        put(&mut out, 0); // CRC-32 of everything after it, written last
        put(&mut out, 0); // size of everything after the header, written last
        put(&mut out, 0x9411_9c00); // magic
        for _ in 0..3 {
            put(&mut out, 0);
        }
        put(&mut out, 3 << 27); // 16 kHz, as Shazam numbers its sample rates
        for _ in 0..2 {
            put(&mut out, 0);
        }
        put(&mut out, self.samples + (SAMPLE_RATE as f32 * 0.24) as u32);
        put(&mut out, (15 << 19) + 0x40000);
        debug_assert_eq!(out.len(), 48);

        put(&mut out, 0x4000_0000);
        put(&mut out, 0); // the size again, written last

        for (band, peaks) in self.bands.iter().enumerate() {
            if peaks.is_empty() {
                continue;
            }
            let mut packed = Vec::with_capacity(peaks.len() * 5);
            let mut pass = 0u32;
            for peak in peaks {
                if peak.pass - pass >= 255 {
                    packed.push(0xff);
                    packed.extend_from_slice(&peak.pass.to_le_bytes());
                    pass = peak.pass;
                }
                packed.push((peak.pass - pass) as u8);
                packed.extend_from_slice(&peak.magnitude.to_le_bytes());
                packed.extend_from_slice(&peak.bin.to_le_bytes());
                pass = peak.pass;
            }
            put(&mut out, 0x6003_0040 + band as u32);
            put(&mut out, packed.len() as u32);
            out.extend_from_slice(&packed);
            while !out.len().is_multiple_of(4) {
                out.push(0);
            }
        }

        let size = (out.len() - 48) as u32;
        out[8..12].copy_from_slice(&size.to_le_bytes());
        out[52..56].copy_from_slice(&size.to_le_bytes());
        let crc = crc32fast::hash(&out[8..]);
        out[4..8].copy_from_slice(&crc.to_le_bytes());
        out
    }

    /// As Shazam's request carries it.
    pub fn uri(&self) -> String {
        use base64::Engine;
        format!(
            "data:audio/vnd.shazam.sig;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(self.encode())
        )
    }

    pub fn milliseconds(&self) -> u32 {
        (self.samples as u64 * 1000 / SAMPLE_RATE as u64) as u32
    }
}

const FFT_SIZE: usize = 2048;
const BINS: usize = FFT_SIZE / 2 + 1;
const HISTORY: usize = 256;

/// Works through 16 kHz mono samples 128 at a time, as they arrive or all at once.
pub struct Generator {
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    ring: Vec<i16>,
    ring_at: usize,
    spectra: Vec<Vec<f32>>,
    spectra_at: usize,
    spread: Vec<Vec<f32>>,
    spread_at: usize,
    spread_written: u32,
    buffer: Vec<Complex<f32>>,
    pending: Vec<i16>,
    signature: Signature,
}

impl Default for Generator {
    fn default() -> Self {
        Self::new()
    }
}

impl Generator {
    pub fn new() -> Self {
        let window = (0..FFT_SIZE)
            .map(|i| {
                0.5 - 0.5
                    * ((2.0 * std::f64::consts::PI * (i + 1) as f64) / (FFT_SIZE + 1) as f64).cos()
            })
            .map(|w| w as f32)
            .collect();
        Generator {
            fft: FftPlanner::new().plan_fft_forward(FFT_SIZE),
            window,
            ring: vec![0; FFT_SIZE],
            ring_at: 0,
            spectra: vec![vec![0.0; BINS]; HISTORY],
            spectra_at: 0,
            spread: vec![vec![0.0; BINS]; HISTORY],
            spread_at: 0,
            spread_written: 0,
            buffer: vec![Complex::new(0.0, 0.0); FFT_SIZE],
            pending: Vec::with_capacity(128),
            signature: Signature::default(),
        }
    }

    /// The whole of a recording, in one go. Only the first [MAX_SECONDS] are used.
    pub fn of(samples: &[i16]) -> Signature {
        let mut generator = Generator::new();
        let most = (SAMPLE_RATE * MAX_SECONDS) as usize;
        generator.feed(&samples[..samples.len().min(most)]);
        generator.finish()
    }

    pub fn feed(&mut self, samples: &[i16]) {
        for &sample in samples {
            self.pending.push(sample);
            if self.pending.len() == 128 {
                let chunk = std::mem::take(&mut self.pending);
                self.process(&chunk);
                self.pending = chunk;
                self.pending.clear();
            }
        }
    }

    pub fn samples(&self) -> u32 {
        self.signature.samples
    }

    pub fn finish(self) -> Signature {
        self.signature
    }

    fn process(&mut self, chunk: &[i16]) {
        self.signature.samples += chunk.len() as u32;
        self.do_fft(chunk);
        self.spread_peaks();
        if self.spread_written >= 46 {
            self.recognise_peaks();
        }
    }

    fn do_fft(&mut self, chunk: &[i16]) {
        self.ring[self.ring_at..self.ring_at + chunk.len()].copy_from_slice(chunk);
        self.ring_at = (self.ring_at + chunk.len()) % FFT_SIZE;
        for i in 0..FFT_SIZE {
            let sample = self.ring[(self.ring_at + i) % FFT_SIZE] as f32;
            self.buffer[i] = Complex::new(sample * self.window[i], 0.0);
        }
        self.fft.process(&mut self.buffer);
        let out = &mut self.spectra[self.spectra_at];
        for (i, value) in out.iter_mut().enumerate() {
            let c = self.buffer[i];
            *value = ((c.re * c.re + c.im * c.im) / (1 << 17) as f32).max(0.000_000_000_1);
        }
        self.spectra_at = (self.spectra_at + 1) % HISTORY;
    }

    fn back(at: usize, by: i32) -> usize {
        (at as i32 + by).rem_euclid(HISTORY as i32) as usize
    }

    fn spread_peaks(&mut self) {
        let mut spread = self.spectra[Self::back(self.spectra_at, -1)].clone();
        for i in 0..=BINS - 3 {
            spread[i] = spread[i].max(spread[i + 1]).max(spread[i + 2]);
        }
        for (i, &value) in spread.iter().enumerate() {
            for before in [1, 3, 6] {
                let at = Self::back(self.spread_at, -before);
                let former = &mut self.spread[at][i];
                *former = former.max(value);
            }
        }
        self.spread[self.spread_at] = spread;
        self.spread_at = (self.spread_at + 1) % HISTORY;
        self.spread_written += 1;
    }

    fn recognise_peaks(&mut self) {
        let minus_46 = &self.spectra[Self::back(self.spectra_at, -46)];
        let minus_49 = &self.spread[Self::back(self.spread_at, -49)];
        let pass = self.spread_written - 46;
        let loudness = |v: f32| (v.max(1.0 / 64.0)).ln() * 1477.3 + 6144.0;

        for bin in 10..=1014usize {
            let here = minus_46[bin];
            if here < 1.0 / 64.0 || here < minus_49[bin - 1] {
                continue;
            }
            let mut around = 0f32;
            for offset in [-10i32, -7, -4, -3, 1, 2, 5, 8] {
                around = around.max(minus_49[(bin as i32 + offset) as usize]);
            }
            if here <= around {
                continue;
            }
            for offset in [
                -53i32, -45, 165, 172, 179, 186, 193, 200, 214, 221, 228, 235, 242, 249,
            ] {
                around = around.max(self.spread[Self::back(self.spread_at, offset)][bin - 1]);
            }
            if here <= around {
                continue;
            }

            let magnitude = loudness(here);
            let before = loudness(minus_46[bin - 1]);
            let after = loudness(minus_46[bin + 1]);
            let variation_1 = magnitude * 2.0 - before - after;
            if variation_1 <= 0.0 {
                continue;
            }
            let variation_2 = (after - before) * 32.0 / variation_1;
            let corrected = (bin as i32 * 64 + variation_2 as i32) as u16;
            let hz = corrected as f32 * (SAMPLE_RATE as f32 / 2.0 / 1024.0 / 64.0);
            let band = match hz {
                hz if (250.0..520.0).contains(&hz) => 0,
                hz if (520.0..1450.0).contains(&hz) => 1,
                hz if (1450.0..3500.0).contains(&hz) => 2,
                hz if (3500.0..=5500.0).contains(&hz) => 3,
                _ => continue,
            };
            self.signature.bands[band].push(Peak {
                pass,
                magnitude: magnitude as u16,
                bin: corrected,
            });
        }
    }
}

#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;

    /// A chord that moves, so there are peaks to find in more than one place and time.
    pub fn tune(seconds: u32) -> Vec<i16> {
        let notes = [440.0f32, 554.37, 659.25, 880.0, 1174.66, 2349.3, 4186.0];
        (0..SAMPLE_RATE * seconds)
            .map(|i| {
                let t = i as f32 / SAMPLE_RATE as f32;
                let step = (t * 4.0) as usize;
                let mut v = 0.0;
                for (n, f) in notes.iter().enumerate() {
                    if !(step + n).is_multiple_of(3) {
                        v += (2.0 * std::f32::consts::PI * f * t).sin() / notes.len() as f32;
                    }
                }
                (v * 20_000.0) as i16
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::tests_support::tune;
    use super::*;

    #[test]
    fn silence_has_no_peaks() {
        let signature = Generator::of(&vec![0; 16_000 * 5]);
        assert_eq!(signature.peak_count(), 0);
        assert_eq!(signature.samples, 16_000 * 5);
    }

    #[test]
    fn a_tune_has_peaks_in_its_bands() {
        let signature = Generator::of(&tune(6));
        assert!(signature.peak_count() > 20, "{}", signature.peak_count());
        assert!(signature.bands.iter().filter(|b| !b.is_empty()).count() >= 3);
        for band in &signature.bands {
            assert!(band.windows(2).all(|w| w[0].pass <= w[1].pass));
        }
    }

    #[test]
    fn the_binary_is_shaped_as_shazam_reads_it() {
        let signature = Generator::of(&tune(4));
        let bytes = signature.encode();
        let word = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        assert_eq!(word(0), 0xcafe2580);
        assert_eq!(word(12), 0x94119c00);
        assert_eq!(word(8) as usize, bytes.len() - 48);
        assert_eq!(word(52), word(8));
        assert_eq!(word(4), crc32fast::hash(&bytes[8..]));
        assert_eq!(bytes.len() % 4, 0);
        assert!(signature
            .uri()
            .starts_with("data:audio/vnd.shazam.sig;base64,"));
    }

    #[test]
    fn only_twelve_seconds_are_used() {
        let signature = Generator::of(&vec![0; 16_000 * 30]);
        assert_eq!(signature.samples, 16_000 * 12);
        assert_eq!(signature.milliseconds(), 12_000);
    }

    #[test]
    fn fed_in_pieces_is_the_same_as_all_at_once() {
        let samples = tune(5);
        let mut generator = Generator::new();
        for piece in samples.chunks(333) {
            generator.feed(piece);
        }
        assert_eq!(generator.finish(), Generator::of(&samples));
    }
}
