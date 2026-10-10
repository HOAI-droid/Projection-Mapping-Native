// Audio-Quellen für die Audio-Reaktivität:
//   Eingang – Mikrofon oder Audio-Interface, auch virtuelle Geräte wie BlackHole/Loopback
//             (damit lässt sich der Ton von Spotify, Apple Music, YouTube … abgreifen)
//   Player  – eigene Musikdateien abspielen (hörbar) und gleichzeitig analysieren (über ffmpeg)
//
// Die Audiogeräte laufen in eigenen Threads (CoreAudio-Streams dürfen den Thread nicht wechseln);
// die Samples landen in einem Ringpuffer, aus dem ~60× pro Sekunde analysiert wird.

pub mod analysis;

use std::collections::VecDeque;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

pub use analysis::AudioFrame;
use analysis::{Analyzer, FFT_SIZE};

const RING: usize = 8192;
/// Höchstens so viele neue Samples für Milkdrop zwischenspeichern
const PCM_MAX: usize = 8192;

#[derive(Default)]
struct Ring {
    buf: VecDeque<f32>,
    /// neue Samples seit der letzten Abholung (Futter für projectM)
    pcm: Vec<f32>,
    rate: f32,
}

impl Ring {
    fn push(&mut self, s: f32) {
        if self.buf.len() >= RING {
            self.buf.pop_front();
        }
        self.buf.push_back(s);
        if self.pcm.len() < PCM_MAX {
            self.pcm.push(s);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceKind {
    None,
    Input,
    Player,
}

/// Hält einen Audio-Stream in einem eigenen Thread am Leben
struct StreamThread {
    stop: Arc<AtomicBool>,
}

impl Drop for StreamThread {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn keep_stream<F>(build: F) -> Result<(StreamThread, f32), String>
where
    F: FnOnce() -> Result<(cpal::Stream, f32), String> + Send + 'static,
{
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = std::sync::mpsc::channel();
    let s = stop.clone();
    std::thread::Builder::new()
        .name("audio".into())
        .spawn(move || match build() {
            Ok((stream, rate)) => {
                if let Err(e) = stream.play() {
                    let _ = tx.send(Err(e.to_string()));
                    return;
                }
                let _ = tx.send(Ok(rate));
                while !s.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(50));
                }
                drop(stream);
            }
            Err(e) => {
                let _ = tx.send(Err(e));
            }
        })
        .map_err(|e| e.to_string())?;
    let rate = rx.recv_timeout(Duration::from_secs(10)).map_err(|_| "Audiogerät antwortet nicht".to_string())??;
    Ok((StreamThread { stop }, rate))
}

fn find_input(name: &str) -> Result<cpal::Device, String> {
    let host = cpal::default_host();
    if name.is_empty() {
        return host.default_input_device().ok_or_else(|| "Kein Audio-Eingang gefunden".to_string());
    }
    host.input_devices()
        .map_err(|e| e.to_string())?
        .find(|d| d.name().is_ok_and(|n| n == name))
        .ok_or_else(|| format!("Audio-Eingang „{name}“ nicht gefunden"))
}

fn build_input(name: String, ring: Arc<Mutex<Ring>>) -> Result<(cpal::Stream, f32), String> {
    let device = find_input(&name)?;
    let supported = device.default_input_config().map_err(|e| e.to_string())?;
    let config: cpal::StreamConfig = supported.config();
    let channels = config.channels as usize;
    let rate = config.sample_rate.0 as f32;
    let err = |e| log::warn!("Audio-Eingang: {e}");
    macro_rules! stream {
        ($t:ty, $conv:expr) => {{
            let ring = ring.clone();
            device.build_input_stream(
                &config,
                move |data: &[$t], _: &cpal::InputCallbackInfo| {
                    let mut r = ring.lock().unwrap();
                    for frame in data.chunks(channels) {
                        let s: f32 = frame.iter().map(|v| $conv(*v)).sum::<f32>() / channels as f32;
                        r.push(s);
                    }
                },
                err,
                None,
            )
        }};
    }
    let stream = match supported.sample_format() {
        cpal::SampleFormat::F32 => stream!(f32, |v: f32| v),
        cpal::SampleFormat::I16 => stream!(i16, |v: i16| v as f32 / 32768.0),
        cpal::SampleFormat::I32 => stream!(i32, |v: i32| v as f32 / 2147483648.0),
        cpal::SampleFormat::U16 => stream!(u16, |v: u16| (v as f32 - 32768.0) / 32768.0),
        f => return Err(format!("Sample-Format {f:?} wird nicht unterstützt")),
    }
    .map_err(|e| e.to_string())?;
    Ok((stream, rate))
}

// --- Player

struct PlayerShared {
    playing: AtomicBool,
    /// abgespielte Stereo-Frames seit Titelbeginn (inkl. Sprungziel)
    frames: AtomicU64,
    /// Dekoder ist am Dateiende und der Puffer leer
    finished: AtomicBool,
    eof: AtomicBool,
    /// steigt bei jedem Titelwechsel/Sprung; alte Dekoder beenden sich dann
    generation: AtomicU64,
    buf: Mutex<VecDeque<f32>>,
}

pub struct Player {
    pub tracks: Vec<PathBuf>,
    pub index: usize,
    pub duration: f64,
    shared: Arc<PlayerShared>,
    output: Option<StreamThread>,
    rate: f32,
}

impl Player {
    fn new() -> Self {
        Self {
            tracks: vec![],
            index: 0,
            duration: 0.0,
            shared: Arc::new(PlayerShared {
                playing: AtomicBool::new(false),
                frames: AtomicU64::new(0),
                finished: AtomicBool::new(false),
                eof: AtomicBool::new(false),
                generation: AtomicU64::new(0),
                buf: Mutex::new(VecDeque::new()),
            }),
            output: None,
            rate: 48000.0,
        }
    }

    pub fn playing(&self) -> bool {
        self.shared.playing.load(Ordering::Relaxed)
    }

    pub fn position(&self) -> f64 {
        self.shared.frames.load(Ordering::Relaxed) as f64 / self.rate as f64
    }

    pub fn track_name(&self, i: usize) -> String {
        self.tracks.get(i).and_then(|p| p.file_stem()).map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
    }

    fn ensure_output(&mut self, ring: &Arc<Mutex<Ring>>) -> Result<(), String> {
        if self.output.is_some() {
            return Ok(());
        }
        let shared = self.shared.clone();
        let ring_cb = ring.clone();
        let (thread, rate) = keep_stream(move || {
            let ring = ring_cb;
            let host = cpal::default_host();
            let device = host.default_output_device().ok_or("Kein Audio-Ausgang gefunden")?;
            let supported = device.default_output_config().map_err(|e| e.to_string())?;
            let config: cpal::StreamConfig = supported.config();
            let channels = config.channels as usize;
            let rate = config.sample_rate.0 as f32;
            if supported.sample_format() != cpal::SampleFormat::F32 {
                return Err(format!("Audio-Ausgang mit {:?} wird nicht unterstützt", supported.sample_format()));
            }
            let stream = device
                .build_output_stream(
                    &config,
                    move |out: &mut [f32], _: &cpal::OutputCallbackInfo| {
                        let playing = shared.playing.load(Ordering::Relaxed);
                        let mut buf = shared.buf.lock().unwrap();
                        let mut r = ring.lock().unwrap();
                        let mut frames = 0u64;
                        for frame in out.chunks_mut(channels) {
                            let (l, rr) = if playing && buf.len() >= 2 {
                                frames += 1;
                                (buf.pop_front().unwrap(), buf.pop_front().unwrap())
                            } else {
                                (0.0, 0.0)
                            };
                            for (c, o) in frame.iter_mut().enumerate() {
                                *o = if c % 2 == 0 { l } else { rr };
                            }
                            if playing {
                                r.push((l + rr) * 0.5);
                            }
                        }
                        shared.frames.fetch_add(frames, Ordering::Relaxed);
                        if playing && buf.is_empty() && shared.eof.load(Ordering::Relaxed) {
                            shared.finished.store(true, Ordering::Relaxed);
                        }
                    },
                    |e| log::warn!("Audio-Ausgang: {e}"),
                    None,
                )
                .map_err(|e| e.to_string())?;
            Ok((stream, rate))
        })?;
        self.output = Some(thread);
        self.rate = rate;
        ring.lock().unwrap().rate = rate;
        Ok(())
    }

    /// Titel ab Position starten (Dekoder im Hintergrund)
    fn start_decoder(&mut self, at: f64) -> Result<(), String> {
        let Some(path) = self.tracks.get(self.index).cloned() else { return Ok(()) };
        let ff = crate::media::ffmpeg().ok_or("ffmpeg fehlt – im Terminal: brew install ffmpeg")?;
        let generation = self.shared.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.shared.buf.lock().unwrap().clear();
        self.shared.eof.store(false, Ordering::SeqCst);
        self.shared.finished.store(false, Ordering::SeqCst);
        self.shared.frames.store((at * self.rate as f64) as u64, Ordering::SeqCst);
        self.duration = crate::media::ffprobe()
            .and_then(|p| {
                Command::new(p)
                    .args(["-v", "error", "-show_entries", "format=duration", "-of", "default=nw=1:nk=1"])
                    .arg(&path)
                    .output()
                    .ok()
            })
            .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())
            .unwrap_or(0.0);
        let shared = self.shared.clone();
        let rate = self.rate as u32;
        std::thread::Builder::new()
            .name("decoder".into())
            .spawn(move || {
                let child = Command::new(ff)
                    .args(["-v", "error", "-nostdin", "-ss", &format!("{at:.3}"), "-i"])
                    .arg(&path)
                    .args(["-vn", "-f", "f32le", "-ac", "2", "-ar", &rate.to_string(), "pipe:1"])
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .spawn();
                let Ok(mut child) = child else {
                    shared.eof.store(true, Ordering::SeqCst);
                    return;
                };
                let mut out = child.stdout.take().unwrap();
                let mut bytes = vec![0u8; 16384];
                let max = rate as usize * 2 * 2; // 2 Sekunden Stereo
                loop {
                    if shared.generation.load(Ordering::SeqCst) != generation {
                        break;
                    }
                    if shared.buf.lock().unwrap().len() > max {
                        std::thread::sleep(Duration::from_millis(20));
                        continue;
                    }
                    let n = match out.read(&mut bytes) {
                        Ok(0) | Err(_) => {
                            if shared.generation.load(Ordering::SeqCst) == generation {
                                shared.eof.store(true, Ordering::SeqCst);
                            }
                            break;
                        }
                        Ok(n) => n - n % 4,
                    };
                    let mut buf = shared.buf.lock().unwrap();
                    for c in bytes[..n].chunks_exact(4) {
                        buf.push_back(f32::from_le_bytes([c[0], c[1], c[2], c[3]]));
                    }
                }
                let _ = child.kill();
                let _ = child.wait();
            })
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

pub struct AudioEngine {
    pub source: SourceKind,
    pub input_name: String,
    pub inputs: Vec<String>,
    pub error: Option<String>,
    pub player: Player,
    pub analyzer: Analyzer,
    pub frame: AudioFrame,
    ring: Arc<Mutex<Ring>>,
    input: Option<StreamThread>,
    last: Option<Instant>,
    start: Instant,
    /// neue Samples für Milkdrop (seit der letzten Abholung)
    pcm: Vec<f32>,
}

impl Default for AudioEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioEngine {
    pub fn new() -> Self {
        Self {
            source: SourceKind::None,
            input_name: String::new(),
            inputs: vec![],
            error: None,
            player: Player::new(),
            analyzer: Analyzer::new(48000.0),
            frame: AudioFrame::default(),
            ring: Arc::new(Mutex::new(Ring { rate: 48000.0, ..Default::default() })),
            input: None,
            last: None,
            start: Instant::now(),
            pcm: vec![],
        }
    }

    pub fn refresh_inputs(&mut self) {
        let host = cpal::default_host();
        self.inputs = host.input_devices().map(|it| it.filter_map(|d| d.name().ok()).collect()).unwrap_or_default();
    }

    pub fn stop(&mut self) {
        self.input = None;
        self.player.shared.playing.store(false, Ordering::Relaxed);
        self.player.output = None;
        self.source = SourceKind::None;
        self.ring.lock().unwrap().buf.clear();
    }

    pub fn use_input(&mut self, name: &str) {
        self.stop();
        let ring = self.ring.clone();
        let n = name.to_string();
        match keep_stream(move || build_input(n, ring)) {
            Ok((thread, rate)) => {
                self.input = Some(thread);
                self.ring.lock().unwrap().rate = rate;
                self.analyzer.sample_rate = rate;
                self.input_name = name.to_string();
                self.source = SourceKind::Input;
                self.error = None;
            }
            Err(e) => self.error = Some(format!("Audio-Eingang nicht verfügbar: {e}")),
        }
    }

    // --- Player

    pub fn add_tracks(&mut self, paths: Vec<PathBuf>) {
        let before = self.player.tracks.len();
        self.player.tracks.extend(paths);
        if self.source != SourceKind::Player && self.player.tracks.len() > before {
            self.play_track(before);
        }
    }

    pub fn play_track(&mut self, index: usize) {
        if index >= self.player.tracks.len() {
            return;
        }
        if self.source != SourceKind::Player {
            self.stop();
        }
        if let Err(e) = self.player.ensure_output(&self.ring) {
            self.error = Some(e);
            return;
        }
        self.analyzer.sample_rate = self.player.rate;
        self.player.index = index;
        match self.player.start_decoder(0.0) {
            Ok(()) => {
                self.player.shared.playing.store(true, Ordering::Relaxed);
                self.source = SourceKind::Player;
                self.error = None;
            }
            Err(e) => self.error = Some(e),
        }
    }

    pub fn toggle_play(&mut self) {
        if self.source != SourceKind::Player {
            self.play_track(self.player.index);
            return;
        }
        let p = &self.player.shared.playing;
        p.store(!p.load(Ordering::Relaxed), Ordering::Relaxed);
    }

    pub fn next(&mut self) {
        let n = self.player.tracks.len();
        if n > 0 {
            self.play_track((self.player.index + 1) % n);
        }
    }

    pub fn prev(&mut self) {
        let n = self.player.tracks.len();
        if n > 0 {
            self.play_track((self.player.index + n - 1) % n);
        }
    }

    pub fn seek(&mut self, t: f64) {
        if self.source == SourceKind::Player {
            if let Err(e) = self.player.start_decoder(t.max(0.0)) {
                self.error = Some(e);
            }
        }
    }

    pub fn remove_track(&mut self, i: usize) {
        if i < self.player.tracks.len() {
            let playing_this = self.source == SourceKind::Player && i == self.player.index;
            self.player.tracks.remove(i);
            if playing_this {
                self.stop();
            } else if i < self.player.index {
                self.player.index -= 1;
            }
        }
    }

    pub fn tap(&mut self) {
        self.analyzer.tap(self.start.elapsed().as_secs_f64() * 1000.0);
    }

    /// Einmal pro gezeichnetem Bild: analysiert höchstens ~60× pro Sekunde
    pub fn update(&mut self) {
        // Titel zu Ende → nächster
        if self.source == SourceKind::Player && self.player.shared.finished.swap(false, Ordering::SeqCst) {
            self.next();
        }
        let now = Instant::now();
        if self.last.is_some_and(|t| now.duration_since(t) < Duration::from_millis(15)) {
            return;
        }
        self.last = Some(now);
        let now_ms = self.start.elapsed().as_secs_f64() * 1000.0;
        let mut r = self.ring.lock().unwrap();
        self.pcm.append(&mut r.pcm);
        if self.pcm.len() > PCM_MAX {
            let cut = self.pcm.len() - PCM_MAX;
            self.pcm.drain(..cut);
        }
        let active = self.source != SourceKind::None && r.buf.len() >= FFT_SIZE && !(self.source == SourceKind::Player && !self.player.playing());
        if active {
            let n = r.buf.len();
            let time: Vec<f32> = r.buf.range(n - FFT_SIZE..).copied().collect();
            drop(r);
            self.frame = self.analyzer.analyze(&time, now_ms);
        } else {
            drop(r);
            self.frame = self.analyzer.silent();
        }
    }

    /// Neue Samples seit dem letzten Aufruf (für Milkdrop)
    pub fn take_pcm(&mut self) -> Vec<f32> {
        std::mem::take(&mut self.pcm)
    }
}
