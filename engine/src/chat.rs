use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::Sender;
use std::thread;

pub enum Frame {
    Ready,
    Text(String),
    Activity(String),
    TurnEnd,
    Error(String),
    Closed,
}

pub struct Chat {
    child: Child,
    stdin: ChildStdin,
}

impl Chat {
    pub fn start<T: From<Frame> + Send + 'static>(tx: Sender<T>) -> Result<Self, String> {
        let mut child = Command::new("ryoku-rashin")
            .args(["chat", "--follow"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("no se pudo abrir el chat de Rashin: {e}"))?;
        let stdin = child.stdin.take().ok_or("chat sin entrada")?;
        let stdout = child.stdout.take().ok_or("chat sin salida")?;
        thread::spawn(move || {
            let mut replaying = false;
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let Ok(f) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                let text = || f["text"].as_str().unwrap_or("").to_string();
                let frame = match f["type"].as_str().unwrap_or("") {
                    "replay_start" => {
                        replaying = true;
                        None
                    }
                    "replay_end" => {
                        replaying = false;
                        Some(Frame::Ready)
                    }
                    _ if replaying => None,
                    "agent_text" => Some(Frame::Text(text())),
                    "tool" => Some(Frame::Activity(
                        f["title"]
                            .as_str()
                            .unwrap_or("usando una herramienta")
                            .to_string(),
                    )),
                    "agent_thought" => Some(Frame::Activity("pensando".into())),
                    "turn_end" => Some(Frame::TurnEnd),
                    "state" if f["state"] == "dead" => Some(Frame::Error(
                        f["error"]
                            .as_str()
                            .unwrap_or("el chat se cerró")
                            .to_string(),
                    )),
                    _ => None,
                };
                if let Some(frame) = frame
                    && tx.send(frame.into()).is_err()
                {
                    return;
                }
            }
            let _ = tx.send(Frame::Closed.into());
        });
        Ok(Self { child, stdin })
    }

    pub fn send(&mut self, text: &str) -> Result<(), String> {
        writeln!(self.stdin, "{}", json!({ "type": "user", "text": text }))
            .and_then(|_| self.stdin.flush())
            .map_err(|e| format!("no se pudo enviar al chat: {e}"))
    }
}

impl Drop for Chat {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
