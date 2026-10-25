mod audio;
mod chat;
mod text;

use audio::{Recording, Speech, Voice};
use chat::{Chat, Frame};
use serde_json::json;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

enum Event {
    Cmd(String),
    Chat(Frame),
    Heard(Result<String, String>),
}

impl From<Frame> for Event {
    fn from(f: Frame) -> Self {
        Event::Chat(f)
    }
}

#[derive(Clone, Copy, PartialEq)]
enum State {
    Idle,
    Listening,
    Transcribing,
    Thinking,
    Speaking,
}

impl State {
    fn name(self) -> &'static str {
        match self {
            State::Idle => "idle",
            State::Listening => "listening",
            State::Transcribing => "transcribing",
            State::Thinking => "thinking",
            State::Speaking => "speaking",
        }
    }
}

fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}

fn socket_path() -> PathBuf {
    runtime_dir().join("rashin-voz.sock")
}

fn emit(value: serde_json::Value) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{value}");
    let _ = out.flush();
}

fn voice_from_env() -> Voice {
    let home = std::env::var("HOME").unwrap_or_default();
    let piper = std::env::var("RASHIN_VOZ_PIPER").unwrap_or_else(|_| {
        let local = format!("{home}/.local/share/claude-tts/venv/bin/piper");
        if std::path::Path::new(&local).exists() {
            local
        } else {
            "piper".into()
        }
    });
    let model = std::env::var("RASHIN_VOZ_VOICE")
        .unwrap_or_else(|_| format!("{home}/.local/share/claude-tts/voices/es_MX-ald-medium.onnx"));
    Voice { piper, model }
}

fn socket_inode() -> Option<u64> {
    std::fs::metadata(socket_path()).ok().map(|m| m.ino())
}

fn listen_socket(tx: Sender<Event>) -> Option<u64> {
    let path = socket_path();
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            emit(json!({"ev": "error", "text": format!("socket de control: {e}")}));
            return None;
        }
    };
    thread::spawn(move || {
        for stream in listener.incoming().map_while(Result::ok) {
            let mut line = String::new();
            if BufReader::new(stream).read_line(&mut line).is_ok()
                && tx.send(Event::Cmd(line.trim().to_string())).is_err()
            {
                return;
            }
        }
    });
    socket_inode()
}

struct Engine {
    state: State,
    chat: Option<Chat>,
    recording: Option<Recording>,
    speech: Option<Speech>,
    reply: String,
    awaiting: bool,
    pending: Option<String>,
    chat_ready: bool,
    speak: bool,
    voice: Voice,
    tx: Sender<Event>,
}

impl Engine {
    fn set(&mut self, state: State) {
        if self.state != state {
            self.state = state;
            emit(json!({"ev": "state", "state": state.name()}));
        }
    }

    fn error(&mut self, text: impl Into<String>) {
        emit(json!({"ev": "error", "text": text.into()}));
        self.set(State::Idle);
    }

    fn silence(&mut self) {
        if let Some(s) = self.speech.take() {
            s.stop();
        }
        if self.state == State::Speaking {
            self.set(State::Idle);
        }
    }

    fn listen(&mut self) {
        if self.recording.is_some() || matches!(self.state, State::Transcribing | State::Thinking) {
            return;
        }
        self.silence();
        let path = runtime_dir().join(format!("rashin-voz-{}.wav", std::process::id()));
        match Recording::start(&path) {
            Ok(r) => {
                self.recording = Some(r);
                self.set(State::Listening);
            }
            Err(e) => self.error(e),
        }
    }

    fn stop(&mut self) {
        let Some(rec) = self.recording.take() else {
            return;
        };
        let path = rec.stop();
        self.set(State::Transcribing);
        let tx = self.tx.clone();
        thread::spawn(move || {
            let _ = tx.send(Event::Heard(audio::transcribe(&path)));
        });
    }

    fn send(&mut self, text: &str) {
        let text = text.trim();
        if text.is_empty() || self.awaiting {
            return;
        }
        self.silence();
        self.reply.clear();
        self.awaiting = true;
        self.set(State::Thinking);
        self.pending = Some(text.to_string());
        self.connect();
        self.flush();
    }

    fn connect(&mut self) {
        if self.chat.is_some() {
            return;
        }
        self.chat_ready = false;
        match Chat::start(self.tx.clone()) {
            Ok(c) => self.chat = Some(c),
            Err(e) => {
                self.awaiting = false;
                self.pending = None;
                self.error(e);
            }
        }
    }

    fn flush(&mut self) {
        if !self.chat_ready {
            return;
        }
        let (Some(chat), Some(text)) = (self.chat.as_mut(), self.pending.take()) else {
            return;
        };
        if let Err(e) = chat.send(&text) {
            self.chat = None;
            self.awaiting = false;
            self.error(e);
        }
    }

    fn command(&mut self, cmd: &str) {
        let (verb, arg) = cmd.split_once(' ').unwrap_or((cmd, ""));
        match verb {
            "toggle" if self.recording.is_some() => self.stop(),
            "toggle" | "listen" => self.listen(),
            "stop" => self.stop(),
            "cancel" => {
                if let Some(r) = self.recording.take() {
                    r.cancel();
                    self.set(State::Idle);
                }
            }
            "silence" => self.silence(),
            "say" => self.send(arg),
            "speak" => {
                self.speak = arg != "off";
                if !self.speak {
                    self.silence();
                }
            }
            "" => {}
            other => emit(json!({"ev": "error", "text": format!("orden desconocida: {other}")})),
        }
    }

    fn heard(&mut self, result: Result<String, String>) {
        match result {
            Ok(text) if !text.is_empty() => {
                emit(json!({"ev": "heard", "text": text}));
                self.send(&text);
            }
            Ok(_) => self.error("no te escuché, prueba otra vez"),
            Err(e) => self.error(e),
        }
    }

    fn frame(&mut self, frame: Frame) {
        match frame {
            Frame::Ready => {
                self.chat_ready = true;
                self.flush();
            }
            Frame::Text(t) if self.awaiting => self.reply.push_str(&t),
            Frame::Activity(a) if self.awaiting => emit(json!({"ev": "activity", "text": a})),
            Frame::TurnEnd if self.awaiting => {
                self.awaiting = false;
                let reply = std::mem::take(&mut self.reply);
                emit(json!({"ev": "reply", "text": reply}));
                let spoken = text::speakable(&reply);
                if !self.speak || spoken.is_empty() {
                    return self.set(State::Idle);
                }
                match Speech::start(&self.voice, &spoken) {
                    Ok(s) => {
                        self.speech = Some(s);
                        self.set(State::Speaking);
                    }
                    Err(e) => self.error(e),
                }
            }
            Frame::Error(e) => {
                self.chat = None;
                self.pending = None;
                self.awaiting = false;
                self.error(e);
            }
            Frame::Closed => {
                self.chat = None;
                if self.awaiting {
                    self.awaiting = false;
                    self.error("el chat de Rashin se cerró");
                }
            }
            _ => {}
        }
    }

    fn tick(&mut self) {
        if self.speech.as_mut().is_some_and(Speech::finished) {
            self.speech = None;
            self.set(State::Idle);
        }
    }
}

fn serve() {
    let (tx, rx) = mpsc::channel();
    let socket_tx = tx.clone();
    let mut socket = listen_socket(socket_tx.clone());
    let mut ticks: u32 = 0;
    let stdin_tx = tx.clone();
    thread::spawn(move || {
        for line in std::io::stdin().lock().lines().map_while(Result::ok) {
            if stdin_tx.send(Event::Cmd(line.trim().to_string())).is_err() {
                return;
            }
        }
        let _ = stdin_tx.send(Event::Cmd("quit".into()));
    });

    let mut engine = Engine {
        state: State::Idle,
        chat: None,
        recording: None,
        speech: None,
        reply: String::new(),
        awaiting: false,
        pending: None,
        chat_ready: false,
        speak: true,
        voice: voice_from_env(),
        tx,
    };
    emit(json!({"ev": "state", "state": "idle"}));
    engine.connect();
    loop {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(Event::Cmd(c)) if c == "quit" => break,
            Ok(Event::Cmd(c)) => engine.command(&c),
            Ok(Event::Chat(f)) => engine.frame(f),
            Ok(Event::Heard(r)) => engine.heard(r),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        engine.tick();
        ticks = ticks.wrapping_add(1);
        if ticks.is_multiple_of(10) && socket_inode() != socket {
            socket = listen_socket(socket_tx.clone());
        }
    }
    engine.silence();
    if let Some(r) = engine.recording.take() {
        r.cancel();
    }
    if socket.is_some() && socket_inode() == socket {
        let _ = std::fs::remove_file(socket_path());
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("serve") => serve(),
        Some(_) => {
            let line = args.join(" ");
            match UnixStream::connect(socket_path()) {
                Ok(mut s) => {
                    let _ = writeln!(s, "{line}");
                }
                Err(_) => {
                    eprintln!("rashin-voz no está en marcha (¿está instalado el plugin?)");
                    std::process::exit(1);
                }
            }
        }
        None => {
            eprintln!(
                "uso: rashin-voz serve | rashin-voz toggle|listen|stop|cancel|silence|say <texto>"
            );
            std::process::exit(2);
        }
    }
}
