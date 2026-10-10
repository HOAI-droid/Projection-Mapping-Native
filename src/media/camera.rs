// Live-Kameras (MacBook-Webcam, iPhone über Continuity, USB-/HDMI-Grabber) über ffmpeg.
// macOS: AVFoundation (ffmpeg -f avfoundation), Linux: Video4Linux. Ein Stream pro Gerät, auch wenn
// mehrere Flächen dieselbe Kamera zeigen. Geräte werden über ihren Namen angesprochen
// ('' = Standardkamera).

use std::io::{BufRead, Read};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

use super::ffmpeg;

/// Liste der Kameras (Namen)
pub fn list_cameras() -> Vec<String> {
    if cfg!(target_os = "macos") {
        let Some(ff) = ffmpeg() else { return vec![] };
        let Ok(out) = Command::new(ff)
            .args(["-hide_banner", "-f", "avfoundation", "-list_devices", "true", "-i", ""])
            .stdin(Stdio::null())
            .output()
        else {
            return vec![];
        };
        parse_avfoundation_devices(&String::from_utf8_lossy(&out.stderr))
    } else {
        let mut v: Vec<String> = std::fs::read_dir("/dev")
            .map(|d| d.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| n.starts_with("video")).collect())
            .unwrap_or_default();
        v.sort();
        v.into_iter().map(|n| format!("/dev/{n}")).collect()
    }
}

/// Videogeräte aus der Ausgabe von `ffmpeg -f avfoundation -list_devices true -i ""`
pub fn parse_avfoundation_devices(text: &str) -> Vec<String> {
    let mut out = vec![];
    let mut in_video = false;
    for line in text.lines() {
        if line.contains("AVFoundation video devices") {
            in_video = true;
            continue;
        }
        if line.contains("AVFoundation audio devices") {
            break;
        }
        if !in_video {
            continue;
        }
        // „[AVFoundation indev @ 0x…] [0] FaceTime HD Camera“
        if let Some(pos) = line.rfind("] [") {
            let rest = &line[pos + 3..];
            if let Some(end) = rest.find("] ") {
                out.push(rest[end + 2..].trim().to_string());
            }
        }
    }
    out
}

/// Unterstützte Modi aus der Fehlermeldung von avfoundation („  1280x720@[1.000000 30.000000]fps“)
pub fn parse_modes(text: &str) -> Vec<(u32, u32, f64, f64)> {
    let mut modes = vec![];
    for line in text.lines() {
        let Some(at) = line.find("@[") else { continue };
        let Some(open) = line[at..].find('[') else { continue };
        let Some(close) = line[at..].find(']') else { continue };
        let size = line[..at].trim().rsplit(' ').next().unwrap_or("");
        let mut wh = size.split('x');
        let (Some(w), Some(h)) = (wh.next().and_then(|v| v.parse().ok()), wh.next().and_then(|v| v.parse().ok())) else { continue };
        let rates: Vec<f64> = line[at + open + 1..at + close].split_whitespace().filter_map(|v| v.parse().ok()).collect();
        if let (Some(&lo), Some(&hi)) = (rates.first(), rates.last()) {
            modes.push((w, h, lo, hi));
        }
    }
    modes
}

/// Bester Modus: 1280×720 bevorzugt, sonst die größte Auflösung bis 1920 Breite; bis 30 fps
pub fn choose_mode(modes: &[(u32, u32, f64, f64)]) -> Option<(u32, u32, f64)> {
    let score = |&(w, h, _, hi): &(u32, u32, f64, f64)| {
        let pref = if (w, h) == (1280, 720) { 10_000_000 } else { 0 };
        let fits = if w <= 1920 { 1_000_000 } else { 0 };
        pref + fits + (w * h / 10) as i64 + hi.min(30.0) as i64
    };
    modes.iter().max_by_key(|m| score(m)).map(|&(w, h, lo, hi)| (w, h, hi.min(30.0).max(lo)))
}

struct Shared {
    frame: Option<Arc<Vec<u8>>>,
    seq: u64,
    width: u32,
    height: u32,
    error: Option<String>,
    stop: bool,
}

pub struct CameraStream {
    shared: Arc<Mutex<Shared>>,
    child: Arc<Mutex<Option<Child>>>,
    last_seq: u64,
}

impl CameraStream {
    pub fn open(device: &str) -> Self {
        let shared = Arc::new(Mutex::new(Shared { frame: None, seq: 0, width: 0, height: 0, error: None, stop: false }));
        let child = Arc::new(Mutex::new(None));
        let (s, c, dev) = (shared.clone(), child.clone(), device.to_string());
        std::thread::Builder::new()
            .name("camera".into())
            .spawn(move || {
                if let Err(e) = run(&s, &c, &dev) {
                    s.lock().unwrap().error = Some(e);
                }
            })
            .ok();
        Self { shared, child, last_seq: 0 }
    }

    /// Neues Bild (Breite, Höhe, RGBA) seit dem letzten Aufruf?
    pub fn take_frame(&mut self) -> Option<(u32, u32, Arc<Vec<u8>>)> {
        let st = self.shared.lock().unwrap();
        if st.seq != self.last_seq {
            self.last_seq = st.seq;
            st.frame.clone().map(|f| (st.width, st.height, f))
        } else {
            None
        }
    }

    pub fn error(&self) -> Option<String> {
        self.shared.lock().unwrap().error.clone()
    }
}

impl Drop for CameraStream {
    fn drop(&mut self) {
        self.shared.lock().unwrap().stop = true;
        if let Some(mut c) = self.child.lock().unwrap().take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

fn input_args(device: &str, mode: Option<(u32, u32, f64)>) -> Result<Vec<String>, String> {
    if cfg!(target_os = "macos") {
        let devices = list_cameras();
        let index = if device.is_empty() { 0 } else { devices.iter().position(|d| d == device).ok_or(format!("Kamera „{device}“ nicht gefunden"))? };
        if devices.is_empty() {
            return Err("Keine Kamera gefunden".into());
        }
        let mut a = vec!["-f".into(), "avfoundation".into()];
        if let Some((w, h, fps)) = mode {
            a.extend(["-framerate".into(), format!("{fps:.3}"), "-video_size".into(), format!("{w}x{h}")]);
        } else {
            // absichtlich unpassend: ffmpeg nennt dann die unterstützten Modi
            a.extend(["-framerate".into(), "12345".into()]);
        }
        a.extend(["-i".into(), format!("{index}:none")]);
        Ok(a)
    } else {
        let dev = if device.is_empty() { "/dev/video0".to_string() } else { device.to_string() };
        Ok(vec!["-f".into(), "v4l2".into(), "-i".into(), dev])
    }
}

fn run(shared: &Arc<Mutex<Shared>>, child_slot: &Arc<Mutex<Option<Child>>>, device: &str) -> Result<(), String> {
    let ff = ffmpeg().ok_or("ffmpeg fehlt – im Terminal: brew install ffmpeg")?;
    // macOS: erst die unterstützten Modi erfragen, dann mit dem besten öffnen
    let mode = if cfg!(target_os = "macos") {
        let out = Command::new(&ff)
            .args(["-hide_banner"])
            .args(input_args(device, None)?)
            .args(["-frames:v", "1", "-f", "null", "-"])
            .stdin(Stdio::null())
            .output()
            .map_err(|e| e.to_string())?;
        let text = String::from_utf8_lossy(&out.stderr);
        let modes = parse_modes(&text);
        if modes.is_empty() && !out.status.success() {
            return Err(format!("Kamera nicht verfügbar (Zugriff erlaubt? Systemeinstellungen → Datenschutz → Kamera)\n{}", text.trim()));
        }
        choose_mode(&modes).or(Some((1280, 720, 30.0)))
    } else {
        None
    };
    // Ausgabe auf höchstens 1280 Breite skalieren
    let (w, h) = match mode {
        Some((w, h, _)) if w > 1280 => (1280, ((h as f64 * 1280.0 / w as f64) / 2.0).round() as u32 * 2),
        Some((w, h, _)) => (w, h),
        None => (1280, 720),
    };
    let mut cmd = Command::new(&ff);
    cmd.args(["-hide_banner", "-loglevel", "error", "-nostdin"]).args(input_args(device, mode)?);
    cmd.args(["-an", "-vf", &format!("scale={w}:{h}"), "-f", "rawvideo", "-pix_fmt", "rgba", "pipe:1"]);
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    let mut out = child.stdout.take().unwrap();
    let err = child.stderr.take();
    let err_text = Arc::new(Mutex::new(String::new()));
    if let Some(e) = err {
        let t = err_text.clone();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(e).lines().map_while(Result::ok) {
                let mut s = t.lock().unwrap();
                s.push_str(&line);
                s.push('\n');
                if s.len() > 4000 {
                    let cut = s.len() - 2000;
                    s.drain(..cut);
                }
            }
        });
    }
    {
        let mut slot = child_slot.lock().unwrap();
        if shared.lock().unwrap().stop {
            let _ = child.kill();
            return Ok(());
        }
        *slot = Some(child);
    }
    shared.lock().unwrap().width = w;
    shared.lock().unwrap().height = h;
    let size = (w * h * 4) as usize;
    loop {
        let mut buf = vec![0u8; size];
        if out.read_exact(&mut buf).is_err() {
            if shared.lock().unwrap().stop {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
            let msg = err_text.lock().unwrap().clone();
            return Err(if msg.trim().is_empty() { "Kamera-Stream beendet".into() } else { msg.trim().to_string() });
        }
        let mut st = shared.lock().unwrap();
        if st.stop {
            return Ok(());
        }
        st.frame = Some(Arc::new(buf));
        st.seq += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_devices() {
        let text = "[AVFoundation indev @ 0x1] AVFoundation video devices:\n\
            [AVFoundation indev @ 0x1] [0] FaceTime HD Camera\n\
            [AVFoundation indev @ 0x1] [1] iPhone [Kamera]\n\
            [AVFoundation indev @ 0x1] [2] Capture screen 0\n\
            [AVFoundation indev @ 0x1] AVFoundation audio devices:\n\
            [AVFoundation indev @ 0x1] [0] MacBook Air Microphone\n";
        assert_eq!(parse_avfoundation_devices(text), vec!["FaceTime HD Camera", "iPhone [Kamera]", "Capture screen 0"]);
    }

    #[test]
    fn parses_modes_and_chooses() {
        let text = "[avfoundation @ 0x1] Selected framerate (1.000000) is not supported by the device.\n\
            [avfoundation @ 0x1] Supported modes:\n\
            [avfoundation @ 0x1]   640x480@[1.000000 30.000000]fps\n\
            [avfoundation @ 0x1]   1280x720@[1.000000 30.000000]fps\n\
            [avfoundation @ 0x1]   1920x1080@[1.000000 30.000000]fps\n";
        let modes = parse_modes(text);
        assert_eq!(modes.len(), 3);
        assert_eq!(choose_mode(&modes), Some((1280, 720, 30.0)));
        assert_eq!(choose_mode(&[(1920, 1080, 1.0, 60.0), (3840, 2160, 1.0, 30.0)]), Some((1920, 1080, 30.0)));
    }
}
