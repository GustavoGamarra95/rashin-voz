use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};

pub struct Recording {
    child: Child,
    pub path: PathBuf,
}

impl Recording {
    pub fn start(path: &Path) -> Result<Self, String> {
        let child = crate::child::command("pw-record")
            .args(["--rate", "16000", "--channels", "1", "--format", "s16"])
            .arg(path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("no se pudo abrir el micrófono (pw-record): {e}"))?;
        Ok(Self {
            child,
            path: path.to_path_buf(),
        })
    }

    pub fn stop(mut self) -> PathBuf {
        unsafe { libc::kill(self.child.id() as i32, libc::SIGINT) };
        let _ = self.child.wait();
        self.path.clone()
    }

    pub fn cancel(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.path);
    }
}

pub fn transcribe(path: &Path, lang: &str) -> Result<String, String> {
    let mut cmd = crate::child::command("voxtype");
    if !lang.is_empty() && lang != "auto" {
        cmd.args(["--language", lang]);
    }
    let out = cmd
        .arg("transcribe")
        .arg(path)
        .stderr(Stdio::null())
        .output()
        .map_err(|e| format!("no se pudo ejecutar voxtype: {e}"))?;
    let _ = std::fs::remove_file(path);
    if !out.status.success() {
        return Err("voxtype no pudo transcribir el audio".into());
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    Ok(stdout
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .to_string())
}

#[derive(Clone)]
pub struct Voice {
    pub piper: String,
    pub model: String,
}

impl Voice {
    fn rate(&self) -> u64 {
        std::fs::read_to_string(format!("{}.json", self.model))
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .and_then(|v| v["audio"]["sample_rate"].as_u64())
            .unwrap_or(22050)
    }
}

pub struct Speech {
    piper: Child,
    player: Child,
}

impl Speech {
    pub fn start(voice: &Voice, text: &str) -> Result<Self, String> {
        let mut piper = crate::child::command(&voice.piper)
            .args(["-m", &voice.model, "--output-raw"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("no se pudo ejecutar piper ({}): {e}", voice.piper))?;
        let pcm = piper.stdout.take().ok_or("piper sin salida")?;
        let player = crate::child::command("pw-play")
            .args([
                "--raw",
                "--rate",
                &voice.rate().to_string(),
                "--channels",
                "1",
                "--format",
                "s16",
                "-",
            ])
            .stdin(pcm)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("no se pudo reproducir (pw-play): {e}"))?;
        if let Some(mut input) = piper.stdin.take() {
            let _ = input.write_all(text.as_bytes());
        }
        Ok(Self { piper, player })
    }

    pub fn finished(&mut self) -> bool {
        matches!(self.player.try_wait(), Ok(Some(_)))
    }

    pub fn stop(mut self) {
        let _ = self.piper.kill();
        let _ = self.player.kill();
        let _ = self.piper.wait();
        let _ = self.player.wait();
    }
}
