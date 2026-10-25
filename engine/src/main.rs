mod audio;
mod chat;
mod child;
mod text;
mod tts;
mod wake;

use audio::{Recording, Speech, Voice};
use chat::{Chat, Frame};
use serde_json::json;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};
use tts::{NaturalVoice, TtsEvent};
use wake::{Segment, Wake, WakeListener};

enum Event {
    Cmd(String),
    Chat(Frame),
    Heard(Result<String, String>),
    Tts(TtsEvent),
    Segment(Segment),
    Woke(Woke),
}

enum Woke {
    Nothing,
    Alone,
    Request(Result<String, String>),
}

impl From<Segment> for Event {
    fn from(s: Segment) -> Self {
        Event::Segment(s)
    }
}

impl From<TtsEvent> for Event {
    fn from(e: TtsEvent) -> Self {
        Event::Tts(e)
    }
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
    std::env::var_os("RASHIN_VOZ_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|| runtime_dir().join("rashin-voz.sock"))
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
    natural: Option<NaturalVoice>,
    natural_ready: bool,
    lang: String,
    wake: Option<WakeListener>,
    checking: bool,
    prompting: bool,
    armed_until: Option<Instant>,
    quiet_until: Option<Instant>,
    segments: u64,
    tx: Sender<Event>,
}

impl Engine {
    fn set(&mut self, state: State) {
        if self.state == State::Speaking && state != State::Speaking {
            self.quiet_until = Some(Instant::now() + Duration::from_millis(500));
            let follow = Duration::from_secs(if self.prompting { 8 } else { 6 });
            self.prompting = false;
            if self.wake.is_some() && state == State::Idle {
                self.arm(Some(Instant::now() + follow));
            }
        }
        if self.state != state {
            self.state = state;
            emit(json!({"ev": "state", "state": state.name()}));
        }
    }

    fn arm(&mut self, until: Option<Instant>) {
        let was = self.armed_until.is_some();
        self.armed_until = until;
        if was != until.is_some() {
            emit(json!({"ev": "armed", "on": until.is_some()}));
        }
    }

    fn wake_on(&mut self, on: bool) {
        if !on {
            self.wake = None;
            self.arm(None);
            return;
        }
        if self.wake.is_some() {
            return;
        }
        match WakeListener::start(self.tx.clone()) {
            Ok(w) => self.wake = Some(w),
            Err(e) => emit(json!({"ev": "error", "text": e})),
        }
    }

    fn segment(&mut self, samples: Vec<i16>) {
        if self.state != State::Idle || self.checking || self.recording.is_some() {
            return;
        }
        self.segments += 1;
        if std::env::var_os("RASHIN_VOZ_DEBUG").is_some() {
            emit(json!({"ev": "debug", "segment": samples.len()}));
        }
        let path = runtime_dir().join(format!(
            "rashin-voz-{}-seg{}.wav",
            std::process::id(),
            self.segments
        ));
        if wake::write_wav(&path, &samples).is_err() {
            return;
        }
        let tx = self.tx.clone();
        let lang = self.lang.clone();
        if self.armed_until.is_some() {
            self.arm(None);
            self.set(State::Transcribing);
            thread::spawn(move || {
                let _ = tx.send(Event::Heard(audio::transcribe(&path, &lang)));
            });
            return;
        }
        self.checking = true;
        thread::spawn(move || {
            let quick = audio::transcribe_with(&path, &lang, Some("base")).unwrap_or_default();
            if std::env::var_os("RASHIN_VOZ_DEBUG").is_some() {
                emit(json!({"ev": "debug", "text": quick}));
            }
            let woke = match wake::detect(&quick) {
                Wake::None => Woke::Nothing,
                Wake::Alone => Woke::Alone,
                Wake::WithRequest => Woke::Request(
                    audio::transcribe_with(&path, &lang, None).map(|t| wake::strip(&t)),
                ),
            };
            let _ = std::fs::remove_file(&path);
            let _ = tx.send(Event::Woke(woke));
        });
    }

    fn woke(&mut self, woke: Woke) {
        self.checking = false;
        if self.state != State::Idle {
            return;
        }
        match woke {
            Woke::Nothing => {}
            Woke::Request(r) => {
                emit(json!({"ev": "wake"}));
                self.heard(r);
            }
            Woke::Alone => {
                emit(json!({"ev": "wake"}));
                let word = wake::prompt_word(&self.lang);
                self.prompting = true;
                if self.speak
                    && self.natural_ready
                    && let Some(n) = self.natural.as_mut()
                    && n.say(word, &self.lang)
                {
                    return self.set(State::Speaking);
                }
                if self.speak
                    && let Ok(s) = Speech::start(&self.voice, word)
                {
                    self.speech = Some(s);
                    return self.set(State::Speaking);
                }
                self.prompting = false;
                self.arm(Some(Instant::now() + Duration::from_secs(8)));
            }
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
        if self.state == State::Speaking
            && let Some(n) = self.natural.as_mut()
        {
            n.stop();
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
        let lang = self.lang.clone();
        thread::spawn(move || {
            let _ = tx.send(Event::Heard(audio::transcribe(&path, &lang)));
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
            "lang" => {
                self.lang = tts::resolve_lang(arg);
                emit(json!({"ev": "lang", "lang": self.lang}));
            }
            "say" => self.send(arg),
            "wake" => self.wake_on(arg != "off"),
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
                if self.natural_ready
                    && let Some(n) = self.natural.as_mut()
                    && n.say(&spoken, &self.lang)
                {
                    return self.set(State::Speaking);
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

    fn tts(&mut self, event: TtsEvent) {
        match event {
            TtsEvent::Ready => self.natural_ready = true,
            TtsEvent::Done if self.state == State::Speaking && self.speech.is_none() => {
                self.set(State::Idle)
            }
            TtsEvent::Error(e) => emit(json!({"ev": "error", "text": e})),
            TtsEvent::Closed => {
                self.natural = None;
                self.natural_ready = false;
                if self.state == State::Speaking && self.speech.is_none() {
                    self.set(State::Idle);
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
        let now = Instant::now();
        if self.armed_until.is_some_and(|t| now >= t) {
            self.arm(None);
        }
        if self.quiet_until.is_some_and(|t| now >= t) {
            self.quiet_until = None;
        }
        let mute =
            self.state != State::Idle || self.recording.is_some() || self.quiet_until.is_some();
        if let Some(w) = self.wake.as_ref() {
            w.mute(mute);
        }
    }
}

fn serve(tts_script: Option<String>) {
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
        natural: None,
        natural_ready: false,
        lang: tts::resolve_lang("auto"),
        wake: None,
        checking: false,
        prompting: false,
        armed_until: None,
        quiet_until: None,
        segments: 0,
        tx,
    };
    emit(json!({"ev": "state", "state": "idle"}));
    engine.connect();
    if let Some(script) = tts_script {
        let home = std::env::var("HOME").unwrap_or_default();
        let python = std::env::var("RASHIN_VOZ_TTS_PYTHON").unwrap_or_else(|_| {
            format!("{home}/.local/share/claude-tts/chatterbox/venv/bin/python")
        });
        match NaturalVoice::start(&python, &script, engine.tx.clone()) {
            Ok(n) => engine.natural = Some(n),
            Err(e) => emit(json!({"ev": "error", "text": e})),
        }
    }
    emit(json!({"ev": "lang", "lang": engine.lang}));
    loop {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(Event::Cmd(c)) if c == "quit" => break,
            Ok(Event::Cmd(c)) => engine.command(&c),
            Ok(Event::Chat(f)) => engine.frame(f),
            Ok(Event::Heard(r)) => engine.heard(r),
            Ok(Event::Tts(e)) => engine.tts(e),
            Ok(Event::Segment(Segment(s))) => engine.segment(s),
            Ok(Event::Woke(w)) => engine.woke(w),
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
        Some("serve") => serve(
            args.iter()
                .position(|a| a == "--tts")
                .and_then(|i| args.get(i + 1).cloned())
                .or_else(|| std::env::var("RASHIN_VOZ_TTS").ok()),
        ),
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
