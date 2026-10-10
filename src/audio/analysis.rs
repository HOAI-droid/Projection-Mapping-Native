// Musik-Analyse – rechnet wie der AnalyserNode der Web-Audio-API (FFT 2048, Blackman-Fenster,
// zeitliche Glättung 0,55, −90…−15 dB), damit Effekte und Regler genauso reagieren wie in der
// Web-Version: Pegel, Bass/Mitten/Höhen mit automatischer Pegelanpassung, 64 logarithmische Bänder
// mit Winamp-Spitzen, Wellenform, Beat-Erkennung und Tempo (BPM).

use std::collections::VecDeque;
use std::sync::Arc;

use rustfft::num_complex::Complex32;
use rustfft::{Fft, FftPlanner};

pub const FFT_SIZE: usize = 2048;
pub const BANDS: usize = 64;
pub const WAVE: usize = 128;

/// Ein Analyse-Bild (Werte 0…1, Wellenform −1…1)
#[derive(Clone, Debug)]
pub struct AudioFrame {
    pub level: f32,
    pub bass: f32,
    pub mid: f32,
    pub high: f32,
    /// Puls: springt beim Beat auf 1 und fällt dann ab
    pub beat: f32,
    pub beat_count: u64,
    pub bpm: f32,
    pub spectrum: [f32; BANDS],
    pub peaks: [f32; BANDS],
    pub wave: [f32; WAVE],
}

impl Default for AudioFrame {
    fn default() -> Self {
        Self { level: 0.0, bass: 0.0, mid: 0.0, high: 0.0, beat: 0.0, beat_count: 0, bpm: 0.0, spectrum: [0.0; BANDS], peaks: [0.0; BANDS], wave: [0.0; WAVE] }
    }
}

impl AudioFrame {
    pub fn get(&self, s: crate::model::AudioSource) -> f32 {
        use crate::model::AudioSource::*;
        match s {
            Level => self.level,
            Bass => self.bass,
            Mid => self.mid,
            High => self.high,
            Beat => self.beat,
        }
    }

    /// Daten für die Audio-Textur der Effekte (128×3, Zeile 0 Spektrum, 1 Spitzen, 2 Wellenform)
    pub fn texture_bytes(&self) -> [u8; 384] {
        let mut d = [0u8; 384];
        for i in 0..BANDS {
            d[i] = (self.spectrum[i] * 255.0).round() as u8;
            d[128 + i] = (self.peaks[i] * 255.0).round() as u8;
        }
        for i in 0..WAVE {
            d[256 + i] = ((self.wave[i] + 1.0) / 2.0 * 255.0).round().clamp(0.0, 255.0) as u8;
        }
        d
    }
}

pub struct Analyzer {
    pub sample_rate: f32,
    /// Verstärkung 0.2 … 4
    pub gain: f32,
    /// Beat-Empfindlichkeit 1.1 (empfindlich) … 2 (nur deutliche Schläge)
    pub sensitivity: f32,
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    smoothed: Vec<f32>,
    freq: Vec<u8>,
    band_max: [f32; 4],
    peaks: [f32; BANDS],
    bass_history: VecDeque<f32>,
    last_beat: Option<f64>,
    beat_intervals: VecDeque<f64>,
    beat_pulse: f32,
    beat_count: u64,
    bpm: f32,
}

impl Analyzer {
    pub fn new(sample_rate: f32) -> Self {
        let fft = FftPlanner::new().plan_fft_forward(FFT_SIZE);
        let n = FFT_SIZE as f32;
        let window = (0..FFT_SIZE)
            .map(|i| {
                let x = i as f32 / n;
                0.42 - 0.5 * (std::f32::consts::TAU * x).cos() + 0.08 * (2.0 * std::f32::consts::TAU * x).cos()
            })
            .collect();
        Self {
            sample_rate,
            gain: 1.0,
            sensitivity: 1.35,
            fft,
            window,
            smoothed: vec![0.0; FFT_SIZE / 2],
            freq: vec![0; FFT_SIZE / 2],
            band_max: [0.2; 4],
            peaks: [0.0; BANDS],
            bass_history: VecDeque::new(),
            last_beat: None,
            beat_intervals: VecDeque::new(),
            beat_pulse: 0.0,
            beat_count: 0,
            bpm: 0.0,
        }
    }

    /// Manueller Beat (Tap): für Beat-Effekte ohne Musik
    pub fn tap(&mut self, now_ms: f64) {
        self.register_beat(now_ms);
    }

    fn register_beat(&mut self, now: f64) {
        if let Some(last) = self.last_beat {
            let dt = now - last;
            if dt > 250.0 && dt < 2000.0 {
                self.beat_intervals.push_back(dt);
                while self.beat_intervals.len() > 12 {
                    self.beat_intervals.pop_front();
                }
                let mut sorted: Vec<f64> = self.beat_intervals.iter().copied().collect();
                sorted.sort_by(|a, b| a.total_cmp(b));
                let mut bpm = 60000.0 / sorted[sorted.len() / 2];
                while bpm < 70.0 {
                    bpm *= 2.0;
                }
                while bpm > 190.0 {
                    bpm /= 2.0;
                }
                self.bpm = bpm as f32;
            }
        }
        self.last_beat = Some(now);
        self.beat_pulse = 1.0;
        self.beat_count += 1;
    }

    /// Bild ohne Musik (Beat-Puls klingt ab)
    pub fn silent(&mut self) -> AudioFrame {
        self.beat_pulse *= 0.86;
        AudioFrame { beat: self.beat_pulse, beat_count: self.beat_count, bpm: self.bpm.round(), ..Default::default() }
    }

    /// time: die letzten FFT_SIZE Samples (mono, −1…1)
    pub fn analyze(&mut self, time: &[f32], now_ms: f64) -> AudioFrame {
        assert_eq!(time.len(), FFT_SIZE);
        let gain = self.gain;

        // Spektrum wie AnalyserNode.getByteFrequencyData
        let mut buf: Vec<Complex32> = time.iter().zip(&self.window).map(|(s, w)| Complex32::new(s * w, 0.0)).collect();
        self.fft.process(&mut buf);
        let n = FFT_SIZE as f32;
        for k in 0..FFT_SIZE / 2 {
            let mag = buf[k].norm() / n;
            self.smoothed[k] = 0.55 * self.smoothed[k] + 0.45 * mag;
            let db = 20.0 * self.smoothed[k].max(1e-12).log10();
            self.freq[k] = (255.0 * (db + 90.0) / 75.0).floor().clamp(0.0, 255.0) as u8;
        }
        let bin_hz = self.sample_rate / n;
        let freq = self.freq.clone();
        let avg = |lo: f32, hi: f32| {
            let a = ((lo / bin_hz).floor() as usize).max(1);
            let b = ((hi / bin_hz).ceil() as usize).min(freq.len() - 1);
            if b < a {
                return 0.0;
            }
            let sum: u32 = freq[a..=b].iter().map(|&v| v as u32).sum();
            sum as f32 / (b - a + 1) as f32 / 255.0
        };

        let rms = (time.iter().map(|s| s * s).sum::<f32>() / n).sqrt();
        let raw = [rms * 3.0, avg(20.0, 150.0), avg(150.0, 2000.0), avg(2000.0, 12000.0)];
        // Automatische Pegelanpassung: leise Musik wirkt so lebendig wie laute
        let mut norm = [0.0; 4];
        for i in 0..4 {
            self.band_max[i] = raw[i].max(self.band_max[i] * 0.997).max(0.08);
            norm[i] = (raw[i] / self.band_max[i] * 0.9 * gain).min(1.0);
        }

        // Beat: Bass-Energie deutlich über dem Mittel der letzten Sekunde
        self.bass_history.push_back(raw[1]);
        while self.bass_history.len() > 60 {
            self.bass_history.pop_front();
        }
        let mean = self.bass_history.iter().sum::<f32>() / self.bass_history.len() as f32;
        let since = self.last_beat.map_or(f64::INFINITY, |t| now_ms - t);
        if raw[1] > mean * self.sensitivity && raw[1] > 0.12 && since > 260.0 {
            self.register_beat(now_ms);
        } else {
            self.beat_pulse *= 0.86;
        }

        // 64 logarithmische Bänder 30 Hz … 16 kHz mit Winamp-artig fallenden Spitzen
        let mut spectrum = [0.0; BANDS];
        let (lo, hi) = (30f32.ln(), 16000f32.ln());
        for i in 0..BANDS {
            let f0 = (lo + (hi - lo) * i as f32 / BANDS as f32).exp();
            let f1 = (lo + (hi - lo) * (i + 1) as f32 / BANDS as f32).exp();
            let v = (avg(f0, f1) * gain * 1.15).min(1.0);
            spectrum[i] = v;
            self.peaks[i] = v.max(self.peaks[i] - 0.012);
        }

        // Wellenform ab einem Nulldurchgang, damit das Oszilloskop ruhig steht
        let mut start = 0;
        for i in 1..FFT_SIZE / 2 {
            if time[i - 1] < 0.0 && time[i] >= 0.0 {
                start = i;
                break;
            }
        }
        let step = FFT_SIZE / 2 / WAVE;
        let mut wave = [0.0; WAVE];
        for (i, w) in wave.iter_mut().enumerate() {
            *w = (time[start + i * step] * 2.5 * gain).clamp(-1.0, 1.0);
        }

        AudioFrame {
            level: norm[0],
            bass: norm[1],
            mid: norm[2],
            high: norm[3],
            beat: self.beat_pulse,
            beat_count: self.beat_count,
            bpm: self.bpm.round(),
            spectrum,
            peaks: self.peaks,
            wave,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, rate: f32, amp: f32, offset: usize) -> Vec<f32> {
        (0..FFT_SIZE).map(|i| amp * (std::f32::consts::TAU * freq * (i + offset) as f32 / rate).sin()).collect()
    }

    #[test]
    fn bass_tone_is_bass() {
        let mut a = Analyzer::new(48000.0);
        let mut f = AudioFrame::default();
        for k in 0..10 {
            f = a.analyze(&sine(80.0, 48000.0, 0.5, k * 800), k as f64 * 16.0);
        }
        assert!(f.bass > 0.5, "Bass {}", f.bass);
        assert!(f.bass > f.high * 2.0, "Bass {} Höhen {}", f.bass, f.high);
        let loudest = (0..BANDS).max_by(|&i, &j| f.spectrum[i].total_cmp(&f.spectrum[j])).unwrap();
        assert!(loudest < 20, "lautestes Band {loudest}");
        assert!(f.wave.iter().any(|&w| w > 0.5));
    }

    #[test]
    fn detects_beats_and_tempo() {
        let mut a = Analyzer::new(48000.0);
        let silence = vec![0.0; FFT_SIZE];
        let kick = sine(60.0, 48000.0, 0.9, 0);
        // 120 BPM: alle 500 ms ein Schlag, Analyse alle 16 ms
        let mut t = 0.0;
        for _ in 0..400 {
            let on = (t % 500.0) < 48.0;
            a.analyze(if on { &kick } else { &silence }, t);
            t += 16.0;
        }
        let f = a.analyze(&silence, t);
        assert!(f.beat_count >= 8, "Beats {}", f.beat_count);
        assert!((f.bpm - 120.0).abs() <= 4.0, "BPM {}", f.bpm);
    }

    #[test]
    fn tap_tempo() {
        let mut a = Analyzer::new(44100.0);
        for i in 0..6 {
            a.tap(i as f64 * 600.0);
        }
        let f = a.silent();
        assert_eq!(f.bpm, 100.0);
        assert_eq!(f.beat_count, 6);
    }

    #[test]
    fn texture_layout() {
        let mut f = AudioFrame::default();
        f.spectrum[0] = 1.0;
        f.wave[0] = 1.0;
        let d = f.texture_bytes();
        assert_eq!(d[0], 255);
        assert_eq!(d[256], 255);
        assert_eq!(d[257], 128);
    }
}
