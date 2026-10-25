use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::thread;

const RATE: usize = 16000;
const FRAME: usize = 480;
const START_FRAMES: usize = 5;
const END_FRAMES: usize = 25;
const PREROLL_FRAMES: usize = 10;
const MIN_SAMPLES: usize = RATE / 2;
const MAX_SAMPLES: usize = RATE * 15;

pub struct Segment(pub Vec<i16>);

pub struct WakeListener {
    child: Child,
    pub muted: Arc<AtomicBool>,
}

impl WakeListener {
    pub fn start<T: From<Segment> + Send + 'static>(tx: Sender<T>) -> Result<Self, String> {
        let mut child = crate::child::command("pw-record")
            .args([
                "--raw",
                "--rate",
                "16000",
                "--channels",
                "1",
                "--format",
                "s16",
                "-",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("no se pudo escuchar el micrófono: {e}"))?;
        let mut stdout = child.stdout.take().ok_or("micrófono sin salida")?;
        let muted = Arc::new(AtomicBool::new(false));
        let flag = muted.clone();
        thread::spawn(move || {
            let mut vad = Vad::default();
            let mut buf = vec![0u8; FRAME * 2];
            while stdout.read_exact(&mut buf).is_ok() {
                let frame: Vec<i16> = buf
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|b| i16::from_le_bytes([b[0], b[1]]))
                    .collect();
                if flag.load(Ordering::Relaxed) {
                    vad.reset();
                    continue;
                }
                if let Some(seg) = vad.push(&frame)
                    && tx.send(Segment(seg).into()).is_err()
                {
                    return;
                }
            }
        });
        Ok(Self { child, muted })
    }

    pub fn mute(&self, on: bool) {
        self.muted.store(on, Ordering::Relaxed);
    }
}

impl Drop for WakeListener {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Default)]
struct Vad {
    floor: f32,
    loud: usize,
    quiet: usize,
    speaking: bool,
    recent: Vec<Vec<i16>>,
    speech: Vec<i16>,
}

impl Vad {
    fn reset(&mut self) {
        self.loud = 0;
        self.quiet = 0;
        self.speaking = false;
        self.recent.clear();
        self.speech.clear();
    }

    fn push(&mut self, frame: &[i16]) -> Option<Vec<i16>> {
        let rms =
            (frame.iter().map(|&s| (s as f32).powi(2)).sum::<f32>() / frame.len() as f32).sqrt();
        if self.floor == 0.0 {
            self.floor = rms.max(50.0);
        }
        let threshold = (self.floor * 3.0).max(300.0);
        let loud = rms > threshold;
        if !self.speaking {
            self.floor = self.floor * 0.95 + rms.min(threshold) * 0.05;
            self.recent.push(frame.to_vec());
            if self.recent.len() > PREROLL_FRAMES {
                self.recent.remove(0);
            }
            self.loud = if loud { self.loud + 1 } else { 0 };
            if self.loud >= START_FRAMES {
                self.speaking = true;
                self.quiet = 0;
                self.speech = self.recent.concat();
                self.recent.clear();
            }
            return None;
        }
        self.speech.extend_from_slice(frame);
        self.quiet = if loud { 0 } else { self.quiet + 1 };
        if self.quiet >= END_FRAMES || self.speech.len() >= MAX_SAMPLES {
            let seg = std::mem::take(&mut self.speech);
            self.reset();
            return (seg.len() >= MIN_SAMPLES).then_some(seg);
        }
        None
    }
}

pub fn write_wav(path: &Path, samples: &[i16]) -> std::io::Result<()> {
    let data = (samples.len() * 2) as u32;
    let mut f = std::fs::File::create(path)?;
    f.write_all(b"RIFF")?;
    f.write_all(&(36 + data).to_le_bytes())?;
    f.write_all(b"WAVEfmt ")?;
    f.write_all(&16u32.to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?;
    f.write_all(&(RATE as u32).to_le_bytes())?;
    f.write_all(&(RATE as u32 * 2).to_le_bytes())?;
    f.write_all(&2u16.to_le_bytes())?;
    f.write_all(&16u16.to_le_bytes())?;
    f.write_all(b"data")?;
    f.write_all(&data.to_le_bytes())?;
    let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    f.write_all(&bytes)
}

fn normalize(text: &str) -> Vec<String> {
    let folded: String = text
        .to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ä' | 'ã' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'ö' | 'õ' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            c if c.is_alphanumeric() => c,
            _ => ' ',
        })
        .collect();
    folded.split_whitespace().map(str::to_string).collect()
}

const GREETINGS: [&str; 13] = [
    "hola", "hey", "oye", "ok", "okay", "hello", "hi", "ola", "salut", "ciao", "hallo", "ei", "eh",
];

fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let cur = row[j + 1];
            row[j + 1] = (prev + usize::from(ca != cb)).min(row[j] + 1).min(cur + 1);
            prev = cur;
        }
    }
    row[b.len()]
}

fn is_name(word: &str, greeted: bool) -> bool {
    let limit = if greeted { 2 } else { 1 };
    word.chars().count() >= 4
        && ["rashin", "rachin", "rasin"]
            .iter()
            .any(|n| distance(word, n) <= limit)
}

pub enum Wake {
    None,
    Alone,
    WithRequest,
}

pub fn detect(text: &str) -> Wake {
    let words = normalize(text);
    let Some(pos) = words
        .iter()
        .take(3)
        .enumerate()
        .position(|(i, w)| is_name(w, i > 0 && GREETINGS.contains(&words[i - 1].as_str())))
    else {
        return Wake::None;
    };
    if !words[..pos].iter().all(|w| GREETINGS.contains(&w.as_str())) {
        return Wake::None;
    }
    if words.len() - pos > 2 {
        Wake::WithRequest
    } else {
        Wake::Alone
    }
}

pub fn strip(text: &str) -> String {
    let mut seen = 0;
    let mut greeted = false;
    for (i, word) in text.split_whitespace().enumerate() {
        let w = normalize(word);
        let is_wake = w
            .iter()
            .all(|x| GREETINGS.contains(&x.as_str()) || is_name(x, greeted));
        greeted = w.iter().any(|x| GREETINGS.contains(&x.as_str()));
        if !is_wake || i > 3 {
            break;
        }
        seen = i + 1;
    }
    let rest: Vec<&str> = text.split_whitespace().skip(seen).collect();
    rest.join(" ")
        .trim_start_matches([',', '.', '!', '¡', ' '])
        .to_string()
}

pub fn prompt_word(lang: &str) -> &'static str {
    match lang {
        "en" => "Yes?",
        "pt" => "Sim?",
        "fr" => "Oui ?",
        "it" => "Sì?",
        "de" => "Ja?",
        _ => "¿Sí?",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_wake_phrases() {
        assert!(matches!(detect("¡Hola, Rashin!"), Wake::Alone));
        assert!(matches!(
            detect("Hola Racing, ¿qué hora es?"),
            Wake::WithRequest
        ));
        assert!(matches!(
            detect("Rashin, abre el navegador"),
            Wake::WithRequest
        ));
        assert!(matches!(detect("Ayer hablé con Rashin"), Wake::None));
        assert!(matches!(detect("hola a todos"), Wake::None));
        assert!(matches!(detect("Hola Basin, que eres?"), Wake::WithRequest));
        assert!(matches!(detect("hola basta ya"), Wake::None));
        assert!(matches!(detect("Raquel ven"), Wake::None));
    }

    #[test]
    fn strips_the_wake_words() {
        assert_eq!(strip("Hola Rashin, ¿qué hora es?"), "¿qué hora es?");
        assert_eq!(strip("Rashin abre el navegador"), "abre el navegador");
        assert_eq!(strip("Hola Basin, que eres?"), "que eres?");
    }

    #[test]
    fn vad_cuts_one_utterance() {
        let mut vad = Vad::default();
        let quiet = vec![10i16; FRAME];
        let loud: Vec<i16> = (0..FRAME)
            .map(|i| if i % 2 == 0 { 4000 } else { -4000 })
            .collect();
        for _ in 0..30 {
            assert!(vad.push(&quiet).is_none());
        }
        for _ in 0..40 {
            assert!(vad.push(&loud).is_none());
        }
        let mut got = None;
        for _ in 0..40 {
            if let Some(s) = vad.push(&quiet) {
                got = Some(s);
            }
        }
        assert!(got.is_some_and(|s| s.len() >= 40 * FRAME));
    }
}
