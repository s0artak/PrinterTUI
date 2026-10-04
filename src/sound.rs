//! The printer's sounds. They are synthesized (no sound files ship with the app), written once
//! per volume as WAV files and played by the system: afplay on macOS, PipeWire, PulseAudio or
//! ALSA on Linux, the sound API on Windows. PRINTERTUI_SOUND_LOG=<file> logs them instead of
//! playing them (to lay the sound under a screen recording).

use std::f32::consts::TAU;
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, Debug)]
pub enum Sound {
    /// The app starts: three notes going up.
    Boot,
    /// A setting changed.
    Blip,
    /// A page goes through: motor whirr and the sheet coming out.
    Print,
    /// The scanner's lamp sweeping.
    Scan,
    /// Something finished well.
    Done,
    /// Something went wrong: a grumble.
    Jam,
}

#[cfg(test)]
pub const ALL: [Sound; 6] = [Sound::Boot, Sound::Blip, Sound::Print, Sound::Scan, Sound::Done, Sound::Jam];

const RATE: u32 = 22050;

/// Gentle: heard over a quiet room, not over a conversation (see the default_volume_is_gentle test).
pub const DEFAULT_VOLUME: u8 = 50;

static VOLUME: AtomicU8 = AtomicU8::new(DEFAULT_VOLUME);

/// 0 (muted) to 100.
pub fn set_volume(v: u8) {
    VOLUME.store(v.min(100), Ordering::Relaxed);
}

pub fn volume() -> u8 {
    VOLUME.load(Ordering::Relaxed)
}

fn name(s: Sound) -> &'static str {
    match s {
        Sound::Boot => "boot",
        Sound::Blip => "blip",
        Sound::Print => "print",
        Sound::Scan => "scan",
        Sound::Done => "done",
        Sound::Jam => "jam",
    }
}

/// Plays a sound in the background at the current volume; silently does nothing without a player.
pub fn play(s: Sound) {
    let vol = volume();
    if let Ok(log) = std::env::var("PRINTERTUI_SOUND_LOG") {
        use std::io::Write;
        let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
        let wav = file(s, vol.max(1)).map_or(String::new(), |p| p.to_string_lossy().into_owned());
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(log) {
            let _ = writeln!(f, "{ms} {} {vol} {wav}", name(s));
        }
        return;
    }
    if vol == 0 {
        return;
    }
    std::thread::spawn(move || {
        if let Some(path) = file(s, vol) {
            player(&path);
        }
    });
}

/// The sound as a WAV file in the temp folder, written the first time it is needed.
pub fn file(s: Sound, vol: u8) -> Option<std::path::PathBuf> {
    let dir = printertui::temp_root().join("sounds");
    // the version changes with the sounds, so an update does not play the old ones
    let path = dir.join(format!("{}-{vol}-v2.wav", name(s)));
    if !path.exists() {
        std::fs::create_dir_all(&dir).ok()?;
        let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
        std::fs::write(&tmp, wav(&samples(s), vol)).ok()?;
        std::fs::rename(&tmp, &path).ok()?;
    }
    Some(path)
}

#[cfg(target_os = "macos")]
fn player(path: &std::path::Path) {
    let _ = std::process::Command::new("afplay").arg(path).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status();
}

#[cfg(all(unix, not(target_os = "macos")))]
fn player(path: &std::path::Path) {
    use std::process::{Command, Stdio};
    for (cmd, args) in [("pw-play", &[][..]), ("paplay", &[][..]), ("aplay", &["-q"][..])] {
        // a player that is installed but has no sound server to talk to fails: try the next one
        if Command::new(cmd).args(args).arg(path).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success()) {
            return;
        }
    }
}

#[cfg(windows)]
fn player(path: &std::path::Path) {
    use windows::Win32::Media::Audio::{PlaySoundW, SND_FILENAME, SND_NODEFAULT, SND_SYNC};
    let wide: Vec<u16> = path.to_string_lossy().encode_utf16().chain([0]).collect();
    // this thread is the sound's own, so it can wait for it
    unsafe {
        let _ = PlaySoundW(windows::core::PCWSTR(wide.as_ptr()), None, SND_FILENAME | SND_SYNC | SND_NODEFAULT);
    }
}

/// 16-bit mono WAV; the volume is applied on a curve, so the steps sound even.
fn wav(samples: &[f32], vol: u8) -> Vec<u8> {
    let gain = (vol as f32 / 100.0).powi(2);
    let data: Vec<u8> = samples.iter().flat_map(|s| (((s * gain).clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes()).collect();
    let mut out = Vec::with_capacity(44 + data.len());
    out.extend(b"RIFF");
    out.extend((36 + data.len() as u32).to_le_bytes());
    out.extend(b"WAVEfmt ");
    out.extend(16u32.to_le_bytes());
    out.extend(1u16.to_le_bytes()); // PCM
    out.extend(1u16.to_le_bytes()); // mono
    out.extend(RATE.to_le_bytes());
    out.extend((RATE * 2).to_le_bytes());
    out.extend(2u16.to_le_bytes());
    out.extend(16u16.to_le_bytes());
    out.extend(b"data");
    out.extend((data.len() as u32).to_le_bytes());
    out.extend(data);
    out
}

/// Repeatable noise in -1..1.
struct Noise(u32);

impl Noise {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.0 >> 8) as f32 / (1 << 23) as f32 - 1.0
    }
}

/// A note: sine with a little third harmonic, quick attack, exponential decay.
fn note(out: &mut Vec<f32>, freq: f32, secs: f32, amp: f32) {
    let n = (secs * RATE as f32) as usize;
    for i in 0..n {
        let t = i as f32 / RATE as f32;
        let env = (t * 400.0).min(1.0) * (-t * 9.0).exp();
        out.push(amp * env * ((TAU * freq * t).sin() + 0.3 * (TAU * 3.0 * freq * t).sin()) / 1.3);
    }
}

fn silence(out: &mut Vec<f32>, secs: f32) {
    out.extend(std::iter::repeat_n(0.0, (secs * RATE as f32) as usize));
}

pub fn samples(s: Sound) -> Vec<f32> {
    let mut out = Vec::new();
    let mut noise = Noise(7);
    let secs = |n: usize| n as f32 / RATE as f32;
    match s {
        Sound::Boot => {
            for f in [523.25, 659.25, 783.99] {
                note(&mut out, f, 0.12, 0.56);
            }
        }
        Sound::Blip => note(&mut out, 1318.5, 0.05, 0.35),
        Sound::Done => {
            note(&mut out, 880.0, 0.12, 0.7);
            note(&mut out, 1318.5, 0.3, 0.7);
        }
        Sound::Print => {
            // the motor: a buzzing sawtooth, pulsing like a print head going back and forth
            let n = (1.1 * RATE as f32) as usize;
            for i in 0..n {
                let t = secs(i);
                let fade = (t * 20.0).min(1.0) * ((1.1 - t) * 20.0).min(1.0);
                let saw = 2.0 * ((t * 70.0 + 0.5 * (t * 3.0).sin()).fract()) - 1.0;
                let pulse = 0.55 + 0.45 * (TAU * 7.0 * t).sin();
                out.push(0.55 * fade * pulse * (0.7 * saw + 0.3 * noise.next()));
            }
            // the sheet sliding out: a short hiss
            let n = (0.18 * RATE as f32) as usize;
            let mut last = 0.0;
            for i in 0..n {
                let x = noise.next();
                let env = (1.0 - i as f32 / n as f32).powi(2);
                out.push(0.12 * env * (x - last));
                last = x;
            }
        }
        Sound::Scan => {
            // a low hum rising and falling as the lamp crosses the glass
            let n = (1.6 * RATE as f32) as usize;
            let mut phase = 0.0;
            for i in 0..n {
                let t = secs(i);
                let f = 110.0 + 40.0 * (std::f32::consts::PI * t / 1.6).sin();
                phase += TAU * f / RATE as f32;
                let fade = (t * 10.0).min(1.0) * ((1.6 - t) * 10.0).min(1.0);
                out.push(0.38 * fade * (phase.sin() + 0.5 * (2.0 * phase).sin() + 0.05 * noise.next()) / 1.5);
            }
        }
        Sound::Jam => {
            // three grumbles, then a sad slide down
            for _ in 0..3 {
                let n = (0.16 * RATE as f32) as usize;
                for i in 0..n {
                    let t = secs(i);
                    let square = if (t * 95.0).fract() < 0.5 { 1.0 } else { -1.0 };
                    let env = (t * 200.0).min(1.0) * (1.0 - t / 0.16);
                    out.push(0.42 * env * (0.8 * square + 0.2 * noise.next()));
                }
                silence(&mut out, 0.05);
            }
            let n = (0.35 * RATE as f32) as usize;
            let mut phase = 0.0;
            for i in 0..n {
                let t = secs(i);
                phase += TAU * (300.0 - 500.0 * t) / RATE as f32;
                out.push(0.5 * (1.0 - t / 0.35) * phase.sin());
            }
        }
    }
    out
}

/// Every sound at the default volume is easy to hear but never loud: its average level (RMS)
/// between -30 and -22 dBFS, like the system's own notification sounds, its peaks below -12 dBFS.
#[test]
fn default_volume_is_gentle() {
    let gain = (DEFAULT_VOLUME as f32 / 100.0).powi(2);
    let levels: Vec<(Sound, f32, f32)> = ALL
        .iter()
        .map(|&s| {
            let loud: Vec<f32> = samples(s).iter().map(|v| v * gain).filter(|v| v.abs() > 1e-3).collect();
            let rms = 20.0 * (loud.iter().map(|v| v * v).sum::<f32>() / loud.len() as f32).sqrt().log10();
            let peak = 20.0 * loud.iter().fold(0f32, |m, v| m.max(v.abs())).log10();
            println!("{s:?}: rms {rms:.1} dBFS, peak {peak:.1} dBFS");
            (s, rms, peak)
        })
        .collect();
    for (s, rms, peak) in levels {
        assert!((-30.0..=-22.0).contains(&rms) && peak < -12.0, "{s:?}: rms {rms:.1}, peak {peak:.1}");
    }
}

#[test]
fn sounds_are_short_quiet_waves() {
    for s in ALL {
        let x = samples(s);
        assert!(!x.is_empty() && x.len() < 3 * RATE as usize, "{s:?}");
        assert!(x.iter().all(|v| v.abs() <= 1.0), "{s:?}");
        // muted is silence, full volume keeps the shape
        let w = wav(&x, 0);
        assert_eq!(&w[..4], b"RIFF");
        assert!(w[44..].iter().all(|b| *b == 0));
        assert_eq!(wav(&x, 100).len(), 44 + 2 * x.len());
    }
}
