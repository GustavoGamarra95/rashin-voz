use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::mpsc::Sender;
use std::thread;

pub enum TtsEvent {
    Ready,
    Done,
    Error(String),
    Closed,
}

pub struct NaturalVoice {
    child: Child,
    stdin: ChildStdin,
}

impl NaturalVoice {
    pub fn start<T: From<TtsEvent> + Send + 'static>(
        python: &str,
        script: &str,
        tx: Sender<T>,
    ) -> Result<Self, String> {
        let mut child = crate::child::command(python)
            .arg(script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("no se pudo iniciar la voz natural: {e}"))?;
        let stdin = child.stdin.take().ok_or("voz sin entrada")?;
        let stdout = child.stdout.take().ok_or("voz sin salida")?;
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let Ok(v) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                let event = match v["ev"].as_str().unwrap_or("") {
                    "ready" => TtsEvent::Ready,
                    "done" => TtsEvent::Done,
                    "error" => {
                        TtsEvent::Error(v["text"].as_str().unwrap_or("error de voz").to_string())
                    }
                    _ => continue,
                };
                if tx.send(event.into()).is_err() {
                    return;
                }
            }
            let _ = tx.send(TtsEvent::Closed.into());
        });
        Ok(Self { child, stdin })
    }

    fn send(&mut self, value: Value) -> bool {
        writeln!(self.stdin, "{value}")
            .and_then(|_| self.stdin.flush())
            .is_ok()
    }

    pub fn say(&mut self, text: &str, lang: &str) -> bool {
        self.send(json!({"cmd": "say", "text": text, "lang": lang}))
    }

    pub fn stop(&mut self) {
        self.send(json!({"cmd": "stop"}));
    }
}

impl Drop for NaturalVoice {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn resolve_lang(setting: &str) -> String {
    let setting = setting.trim().to_lowercase();
    if !setting.is_empty() && setting != "auto" {
        return setting;
    }
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .find(|v| !v.is_empty() && v != "C" && v != "POSIX")
        .map(|v| {
            v.chars()
                .take_while(|c| c.is_ascii_alphabetic())
                .collect::<String>()
                .to_lowercase()
        })
        .filter(|code| code.len() == 2)
        .unwrap_or_else(|| "es".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_language_wins() {
        assert_eq!(resolve_lang("EN"), "en");
    }

    #[test]
    fn auto_reads_os_locale() {
        let code = resolve_lang("auto");
        assert_eq!(code.len(), 2);
    }
}
