// Videowiedergabe über ffmpeg: ein Hintergrund-Thread pro Video dekodiert in rohe RGBA-Bilder
// (auf dem Mac mit VideoToolbox-Hardwaredekodierung) und taktet sie nach Bildrate und Tempo.
// Mehrere Flächen mit demselben Video teilen sich einen Player.

use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use super::ffmpeg;

#[derive(Clone, Copy, PartialEq)]
pub struct Playback {
    pub playing: bool,
    pub looping: bool,
    pub speed: f64,
}

struct Shared {
    state: Mutex<VState>,
    cv: Condvar,
}

struct VState {
    playback: Playback,
    frame: Option<Arc<Vec<u8>>>,
    seq: u64,
    stop: bool,
    ended: bool,
    error: Option<String>,
}

pub struct VideoPlayer {
    shared: Arc<Shared>,
    pub width: u32,
    pub height: u32,
    last_seq: u64,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// Ausgabegröße: höchstens 1920 Pixel breit (bzw. 1920 hoch bei Hochformat), gerade Werte
pub fn output_size(w: u32, h: u32) -> (u32, u32) {
    let scale = (1920.0 / w.max(h).max(1) as f64).min(1.0);
    let even = |v: f64| ((v / 2.0).round() as u32 * 2).max(2);
    (even(w as f64 * scale), even(h as f64 * scale))
}

impl VideoPlayer {
    pub fn new(path: PathBuf, src_w: u32, src_h: u32, fps: f64, playback: Playback) -> Self {
        let (width, height) = output_size(src_w, src_h);
        let shared = Arc::new(Shared {
            state: Mutex::new(VState { playback, frame: None, seq: 0, stop: false, ended: false, error: None }),
            cv: Condvar::new(),
        });
        let s = shared.clone();
        let fps = if fps > 0.0 { fps } else { 30.0 };
        let thread = std::thread::Builder::new()
            .name("video".into())
            .spawn(move || run(s, path, width, height, fps))
            .ok();
        Self { shared, width, height, last_seq: 0, thread }
    }

    pub fn set_playback(&self, p: Playback) {
        let mut st = self.shared.state.lock().unwrap();
        if st.playback != p {
            // Wieder abspielen nach dem Ende → von vorn
            if p.playing && !st.playback.playing && st.ended {
                st.ended = false;
            }
            st.playback = p;
            self.shared.cv.notify_all();
        }
    }

    /// Neues Bild seit dem letzten Aufruf?
    pub fn take_frame(&mut self) -> Option<Arc<Vec<u8>>> {
        let st = self.shared.state.lock().unwrap();
        if st.seq != self.last_seq {
            self.last_seq = st.seq;
            st.frame.clone()
        } else {
            None
        }
    }

    pub fn error(&self) -> Option<String> {
        self.shared.state.lock().unwrap().error.clone()
    }
}

impl Drop for VideoPlayer {
    fn drop(&mut self) {
        {
            let mut st = self.shared.state.lock().unwrap();
            st.stop = true;
            self.shared.cv.notify_all();
        }
        // Thread nicht abwarten: er beendet ffmpeg selbst, sobald er stop sieht
        drop(self.thread.take());
    }
}

fn spawn_ffmpeg(path: &PathBuf, w: u32, h: u32) -> std::io::Result<Child> {
    let ff = ffmpeg().ok_or_else(|| std::io::Error::other("ffmpeg fehlt – im Terminal: brew install ffmpeg"))?;
    let mut cmd = Command::new(ff);
    cmd.args(["-hide_banner", "-loglevel", "error", "-nostdin"]);
    if cfg!(target_os = "macos") {
        cmd.args(["-hwaccel", "videotoolbox"]);
    }
    cmd.arg("-i").arg(path);
    cmd.args(["-an", "-sn", "-vf", &format!("scale={w}:{h}:flags=bilinear"), "-f", "rawvideo", "-pix_fmt", "rgba", "pipe:1"]);
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    cmd.spawn()
}

fn run(shared: Arc<Shared>, path: PathBuf, w: u32, h: u32, fps: f64) {
    let frame_bytes = (w * h * 4) as usize;
    'outer: loop {
        let mut child = match spawn_ffmpeg(&path, w, h) {
            Ok(c) => c,
            Err(e) => {
                shared.state.lock().unwrap().error = Some(e.to_string());
                return;
            }
        };
        let mut out = child.stdout.take().unwrap();
        // Fehlermeldungen von ffmpeg mitlesen (sonst kann die volle Pipe ffmpeg blockieren)
        let err_text = Arc::new(Mutex::new(String::new()));
        let err_thread = child.stderr.take().map(|mut e| {
            let t = err_text.clone();
            std::thread::spawn(move || {
                let mut s = String::new();
                let _ = e.read_to_string(&mut s);
                *t.lock().unwrap() = s;
            })
        });
        // Uhr: Bild n wird bei clock_start + n/fps/speed gezeigt
        let mut clock_start = Instant::now();
        let mut n: u64 = 0;
        let mut speed = shared.state.lock().unwrap().playback.speed;
        loop {
            // Pause / Stopp / Tempowechsel
            {
                let mut st = shared.state.lock().unwrap();
                if st.stop {
                    let _ = child.kill();
                    let _ = child.wait();
                    return;
                }
                if !st.playback.playing {
                    let paused_at = Instant::now();
                    while !st.playback.playing && !st.stop {
                        st = shared.cv.wait(st).unwrap();
                    }
                    clock_start += paused_at.elapsed();
                    if st.stop {
                        continue;
                    }
                }
                if (st.playback.speed - speed).abs() > 1e-6 {
                    // Uhr so umstellen, dass das aktuelle Bild bleibt
                    let pos = n as f64 / fps / speed;
                    speed = st.playback.speed.max(0.01);
                    clock_start = Instant::now() - Duration::from_secs_f64(n as f64 / fps / speed);
                    let _ = pos;
                }
            }
            let mut buf = vec![0u8; frame_bytes];
            if out.read_exact(&mut buf).is_err() {
                // Ende der Datei (oder Fehler)
                let _ = child.kill();
                let status = child.wait();
                if n == 0 {
                    if let Some(t) = err_thread {
                        let _ = t.join();
                    }
                    let msg = err_text.lock().unwrap().clone();
                    let mut st = shared.state.lock().unwrap();
                    st.error = Some(if msg.trim().is_empty() { format!("Video nicht lesbar ({status:?})") } else { msg.trim().to_string() });
                    return;
                }
                let mut st = shared.state.lock().unwrap();
                if st.playback.looping {
                    continue 'outer;
                }
                st.ended = true;
                st.playback.playing = false;
                // Warten, bis wieder abgespielt wird (dann von vorn) oder Schluss ist
                while !st.stop && !st.playback.playing {
                    st = shared.cv.wait(st).unwrap();
                }
                if st.stop {
                    return;
                }
                continue 'outer;
            }
            let due = clock_start + Duration::from_secs_f64(n as f64 / fps / speed);
            let now = Instant::now();
            if due > now {
                std::thread::sleep(due - now);
            } else if now - due > Duration::from_millis(250) {
                // Weit hinterher (z. B. nach Pause des Rechners): Uhr nachziehen statt zu rasen
                clock_start = now - Duration::from_secs_f64(n as f64 / fps / speed);
            }
            n += 1;
            let mut st = shared.state.lock().unwrap();
            st.frame = Some(Arc::new(buf));
            st.seq += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(output_size(3840, 2160), (1920, 1080));
        assert_eq!(output_size(1280, 720), (1280, 720));
        assert_eq!(output_size(1081, 1921), (1080, 1920));
    }

    /// Erzeugt ein kurzes Testvideo und spielt es ab (nur wenn ffmpeg vorhanden ist)
    #[test]
    fn plays_generated_video() {
        let Some(ff) = ffmpeg() else { return };
        let dir = std::env::temp_dir().join(format!("pm-video-{}", crate::model::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("t.mp4");
        let ok = Command::new(ff)
            .args(["-v", "error", "-f", "lavfi", "-i", "testsrc=size=320x240:rate=30:duration=1", "-pix_fmt", "yuv420p"])
            .arg(&file)
            .status()
            .unwrap()
            .success();
        if !ok {
            return;
        }
        let mut p = VideoPlayer::new(file, 320, 240, 30.0, Playback { playing: true, looping: true, speed: 4.0 });
        let start = Instant::now();
        let mut frames = 0;
        while start.elapsed() < Duration::from_millis(3000) && frames < 20 {
            if let Some(f) = p.take_frame() {
                assert_eq!(f.len(), 320 * 240 * 4);
                frames += 1;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(p.error().is_none(), "{:?}", p.error());
        assert!(frames > 10, "nur {frames} Bilder");
        std::fs::remove_dir_all(dir).ok();
    }
}
