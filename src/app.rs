use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Rect;
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinHandle;

use crate::agents::openai_compat;
use crate::agents::pty::PtySession;
use crate::config::{Config, Params};
use crate::session::{
    ChatMessage, HttpProvider, PtyAgentKind, RunNode, RunStatus, Session, SessionKind,
    SessionStore,
};
use crate::ui::scrubber::Scrubber;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Sessions,
    Output,
    Controls,
    Scrubbers,
}

#[derive(Debug, Clone, Copy)]
pub enum Action {
    Start,
    Stop,
    Clear,
    Save,
}

impl Action {
    pub const ALL: [Action; 4] = [Action::Start, Action::Stop, Action::Clear, Action::Save];

    pub fn label(self) -> &'static str {
        match self {
            Action::Start => "Start",
            Action::Stop => "Stop",
            Action::Clear => "Clear",
            Action::Save => "Save",
        }
    }
}

/// Events funneled into the single-threaded update loop from every source.
pub enum AppEvent {
    Input(Event),
    PtyOutput(String, Vec<u8>),
    PtyExited(String),
    HttpChunk(String, String),
    RunFinished(String, RunNode),
    HttpError(String, String),
    Tick,
}

/// Rects captured during render so pointer events can be hit-tested.
#[derive(Default)]
pub struct LayoutRects {
    pub sessions: Vec<Rect>,
    pub buttons: Vec<Rect>,
    pub scrubber_bars: Vec<Rect>,
    pub output: Rect,
}

pub struct SessionRuntime {
    pub session: Session,
    pub pty: Option<PtySession>,
    pub streaming: String,
    pub http_running: bool,
    http_task: Option<JoinHandle<()>>,
    pub last_run: Option<RunNode>,
}

impl SessionRuntime {
    fn id(&self) -> &str {
        &self.session.id
    }

    fn is_http(&self) -> bool {
        self.session.is_http()
    }

    fn is_pty(&self) -> bool {
        self.session.is_pty()
    }
}

pub struct App {
    pub config: Config,
    pub sessions: Vec<SessionRuntime>,
    pub active_idx: usize,
    pub focus: Focus,
    pub new_kind_idx: usize,

    pub scrubbers: Vec<Scrubber>,
    pub scrubber_sel: usize,
    pub button_sel: usize,
    pub session_sel: usize,

    pub status: String,
    pub should_quit: bool,

    pub layout: LayoutRects,
    output_size: (u16, u16),

    tx: UnboundedSender<AppEvent>,
}

impl App {
    pub fn new(config: Config, tx: UnboundedSender<AppEvent>) -> Self {
        let mut sessions = SessionStore::load_all().unwrap_or_default();
        if sessions.is_empty() {
            sessions.push(Session::new(
                "Tinker eval",
                SessionKind::Http {
                    provider: HttpProvider::Tinker,
                },
                config.http.tinker.model.clone(),
                config.params.clone(),
            ));
            sessions.push(Session::new(
                "Codex",
                SessionKind::Pty {
                    agent: PtyAgentKind::Codex,
                },
                String::new(),
                config.params.clone(),
            ));
            for s in &sessions {
                let _ = SessionStore::save_session(s);
            }
        }

        let runtimes: Vec<SessionRuntime> = sessions
            .into_iter()
            .map(|session| SessionRuntime {
                session,
                pty: None,
                streaming: String::new(),
                http_running: false,
                http_task: None,
                last_run: None,
            })
            .collect();

        let active_idx = runtimes.len().saturating_sub(1);
        let params = runtimes
            .get(active_idx)
            .map(|r| r.session.params.clone())
            .unwrap_or_else(|| config.params.clone());

        let scrubbers = scrubbers_from_params(&params);

        Self {
            config,
            sessions: runtimes,
            active_idx,
            focus: Focus::Controls,
            new_kind_idx: 0,
            scrubbers,
            scrubber_sel: 0,
            button_sel: 0,
            session_sel: active_idx,
            status: "Ready. F1 sessions · F2 output · F3 controls · F4 scrubbers · Ctrl+Q quit"
                .to_string(),
            should_quit: false,
            layout: LayoutRects::default(),
            output_size: (24, 80),
            tx,
        }
    }

    pub fn active(&self) -> &SessionRuntime {
        &self.sessions[self.active_idx]
    }

    fn active_mut(&mut self) -> &mut SessionRuntime {
        &mut self.sessions[self.active_idx]
    }

    fn session_index(&self, id: &str) -> Option<usize> {
        self.sessions.iter().position(|r| r.session.id == id)
    }

    fn session_mut(&mut self, id: &str) -> Option<&mut SessionRuntime> {
        let idx = self.session_index(id)?;
        Some(&mut self.sessions[idx])
    }

    pub fn params(&self) -> Params {
        Params {
            temperature: self.scrubbers[0].value,
            max_tokens: self.scrubbers[1].value,
            top_p: self.scrubbers[2].value,
        }
    }

    fn sync_scrubbers_from_active(&mut self) {
        let params = self.active().session.params.clone();
        self.scrubbers = scrubbers_from_params(&params);
    }

    fn sync_scrubbers_to_active(&mut self) {
        self.active_mut().session.params = self.params();
        self.active_mut().session.touch();
    }

    pub fn set_output_size(&mut self, rows: u16, cols: u16) {
        self.output_size = (rows.max(1), cols.max(1));
        for rt in &mut self.sessions {
            if let Some(s) = rt.pty.as_mut() {
                s.resize(rows, cols);
            }
        }
    }

    pub fn handle_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::Input(ev) => self.handle_input(ev),
            AppEvent::PtyOutput(id, bytes) => {
                if let Some(rt) = self.session_mut(&id) {
                    if let Some(s) = rt.pty.as_mut() {
                        s.feed(&bytes);
                    }
                }
            }
            AppEvent::PtyExited(id) => {
                if let Some(rt) = self.session_mut(&id) {
                    rt.pty = None;
                    self.status = format!("{} exited", rt.session.name);
                }
            }
            AppEvent::HttpChunk(id, text) => {
                if let Some(rt) = self.session_mut(&id) {
                    rt.streaming.push_str(&text);
                }
            }
            AppEvent::RunFinished(id, run) => {
                if let Some(rt) = self.session_mut(&id) {
                    rt.http_running = false;
                    rt.http_task = None;
                    if run.status == RunStatus::Done && !run.response.is_empty() {
                        rt.session
                            .messages
                            .push(ChatMessage::assistant(run.response.clone()));
                    }
                    rt.session.playhead_run = Some(run.id.clone());
                    rt.session.touch();
                    rt.last_run = Some(run.clone());
                    rt.streaming.clear();
                    let _ = SessionStore::save_session(&rt.session);
                    let _ = SessionStore::save_run(&id, &run);
                    self.status = format!(
                        "Run {} · {}ms · {:?}",
                        run.short_id(),
                        run.duration_ms,
                        run.status
                    );
                }
            }
            AppEvent::HttpError(id, err) => {
                if let Some(rt) = self.session_mut(&id) {
                    rt.http_running = false;
                    rt.http_task = None;
                    rt.streaming.clear();
                    self.status = format!("{} error: {err}", rt.session.name);
                }
            }
            AppEvent::Tick => {}
        }
    }

    fn handle_input(&mut self, event: Event) {
        match event {
            Event::Key(key) => self.handle_key(key),
            Event::Mouse(m) => self.handle_mouse(m),
            Event::Resize(_, _) => {}
            _ => {}
        }
    }

    fn handle_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        match key.code {
            KeyCode::Char('q') if ctrl => {
                self.should_quit = true;
                return;
            }
            KeyCode::F(1) => {
                self.focus = Focus::Sessions;
                self.session_sel = self.active_idx;
                return;
            }
            KeyCode::F(2) => {
                self.focus = Focus::Output;
                return;
            }
            KeyCode::F(3) => {
                self.focus = Focus::Controls;
                return;
            }
            KeyCode::F(4) => {
                self.focus = Focus::Scrubbers;
                return;
            }
            KeyCode::F(10) => {
                self.should_quit = true;
                return;
            }
            KeyCode::Char('[') => {
                self.select_session(self.active_idx.saturating_sub(1));
                return;
            }
            KeyCode::Char(']') => {
                let next = (self.active_idx + 1).min(self.sessions.len().saturating_sub(1));
                self.select_session(next);
                return;
            }
            KeyCode::Char('n') if !ctrl => {
                self.new_session();
                return;
            }
            _ => {}
        }

        match self.focus {
            Focus::Sessions => self.handle_sessions_key(key),
            Focus::Output => self.handle_output_key(key),
            Focus::Controls => self.handle_controls_key(key),
            Focus::Scrubbers => self.handle_scrubbers_key(key),
        }
    }

    fn handle_sessions_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up => {
                self.session_sel = self.session_sel.saturating_sub(1);
            }
            KeyCode::Down => {
                self.session_sel = (self.session_sel + 1).min(self.sessions.len().saturating_sub(1));
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                self.select_session(self.session_sel);
            }
            KeyCode::Tab => self.focus = Focus::Output,
            KeyCode::BackTab => self.focus = Focus::Controls,
            _ => {}
        }
    }

    fn handle_output_key(&mut self, key: KeyEvent) {
        let is_pty = self.active().is_pty();
        if is_pty {
            if let Some(bytes) = key_to_bytes(&key) {
                if let Some(s) = self.active_mut().pty.as_mut() {
                    s.write_input(&bytes);
                }
            }
            return;
        }

        match key.code {
            KeyCode::Char(c) => self.active_mut().session.draft_prompt.push(c),
            KeyCode::Backspace => {
                self.active_mut().session.draft_prompt.pop();
            }
            KeyCode::Enter => self.send_http(),
            KeyCode::Esc => self.focus = Focus::Controls,
            _ => {}
        }
    }

    fn handle_controls_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Left => {
                self.button_sel = self.button_sel.saturating_sub(1);
            }
            KeyCode::Right => {
                self.button_sel = (self.button_sel + 1).min(Action::ALL.len() - 1);
            }
            KeyCode::Enter | KeyCode::Char(' ') => self.run_action(Action::ALL[self.button_sel]),
            KeyCode::Tab => self.focus = Focus::Scrubbers,
            KeyCode::BackTab => self.focus = Focus::Sessions,
            _ => {}
        }
    }

    fn handle_scrubbers_key(&mut self, key: KeyEvent) {
        let big = key.modifiers.contains(KeyModifiers::SHIFT);
        let mult = if big { 5.0 } else { 1.0 };
        match key.code {
            KeyCode::Up => {
                self.scrubber_sel = self.scrubber_sel.saturating_sub(1);
            }
            KeyCode::Down => {
                self.scrubber_sel = (self.scrubber_sel + 1).min(self.scrubbers.len() - 1);
            }
            KeyCode::Left => self.scrubbers[self.scrubber_sel].dec(mult),
            KeyCode::Right => self.scrubbers[self.scrubber_sel].inc(mult),
            KeyCode::Tab => self.focus = Focus::Output,
            KeyCode::BackTab => self.focus = Focus::Controls,
            _ => {}
        }
        self.sync_scrubbers_to_active();
    }

    fn handle_mouse(&mut self, m: MouseEvent) {
        let point = (m.column, m.row);
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                for (i, r) in self.layout.sessions.clone().iter().enumerate() {
                    if contains(*r, point) {
                        self.focus = Focus::Sessions;
                        self.session_sel = i;
                        self.select_session(i);
                        return;
                    }
                }
                for (i, r) in self.layout.buttons.clone().iter().enumerate() {
                    if contains(*r, point) {
                        self.focus = Focus::Controls;
                        self.button_sel = i;
                        self.run_action(Action::ALL[i]);
                        return;
                    }
                }
                for (i, r) in self.layout.scrubber_bars.clone().iter().enumerate() {
                    if contains(*r, point) {
                        self.focus = Focus::Scrubbers;
                        self.scrubber_sel = i;
                        self.scrub_at(i, *r, m.column);
                        self.sync_scrubbers_to_active();
                        return;
                    }
                }
                if contains(self.layout.output, point) {
                    self.focus = Focus::Output;
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                for (i, r) in self.layout.scrubber_bars.clone().iter().enumerate() {
                    if point.1 >= r.y && point.1 < r.y + r.height {
                        self.scrubber_sel = i;
                        self.scrub_at(i, *r, m.column);
                        self.sync_scrubbers_to_active();
                        return;
                    }
                }
            }
            _ => {}
        }
    }

    fn scrub_at(&mut self, index: usize, bar: Rect, col: u16) {
        if bar.width == 0 {
            return;
        }
        let x = col.saturating_sub(bar.x).min(bar.width.saturating_sub(1));
        let ratio = x as f64 / (bar.width.saturating_sub(1).max(1)) as f64;
        self.scrubbers[index].set_ratio(ratio);
    }

    fn select_session(&mut self, idx: usize) {
        if idx >= self.sessions.len() {
            return;
        }
        self.sync_scrubbers_to_active();
        self.active_idx = idx;
        self.session_sel = idx;
        self.sync_scrubbers_from_active();
        let name = self.active().session.name.clone();
        self.status = format!("Session: {name}");
        let (rows, cols) = self.output_size;
        if let Some(s) = self.active_mut().pty.as_mut() {
            s.resize(rows, cols);
        }
    }

    fn new_session(&mut self) {
        self.sync_scrubbers_to_active();

        const KINDS: [(&str, SessionKind); 4] = [
            (
                "Tinker",
                SessionKind::Http {
                    provider: HttpProvider::Tinker,
                },
            ),
            (
                "LM Studio",
                SessionKind::Http {
                    provider: HttpProvider::LmStudio,
                },
            ),
            (
                "Codex",
                SessionKind::Pty {
                    agent: PtyAgentKind::Codex,
                },
            ),
            (
                "Claude",
                SessionKind::Pty {
                    agent: PtyAgentKind::Claude,
                },
            ),
        ];

        let idx = self.new_kind_idx % KINDS.len();
        self.new_kind_idx = (self.new_kind_idx + 1) % KINDS.len();
        let (label, kind) = &KINDS[idx];

        let model = match kind {
            SessionKind::Http {
                provider: HttpProvider::Tinker,
            } => self.config.http.tinker.model.clone(),
            SessionKind::Http {
                provider: HttpProvider::LmStudio,
            } => self.config.http.lmstudio.model.clone(),
            SessionKind::Pty { .. } => String::new(),
        };

        let name = format!("{label} {}", self.sessions.len() + 1);
        let session = Session::new(name, kind.clone(), model, self.params());
        let _ = SessionStore::save_session(&session);

        self.sessions.push(SessionRuntime {
            session,
            pty: None,
            streaming: String::new(),
            http_running: false,
            http_task: None,
            last_run: None,
        });
        self.select_session(self.sessions.len() - 1);
        self.status = format!("New session: {}", self.active().session.name);
    }

    fn run_action(&mut self, action: Action) {
        match action {
            Action::Start => self.start_active(),
            Action::Stop => self.stop_active(),
            Action::Clear => self.clear_active(),
            Action::Save => self.save_all(),
        }
    }

    fn start_active(&mut self) {
        match &self.active().session.kind {
            SessionKind::Pty { agent } => self.start_pty(*agent),
            SessionKind::Http { .. } => self.send_http(),
        }
    }

    fn start_pty(&mut self, agent: PtyAgentKind) {
        let id = self.active().session.id.clone();
        self.active_mut().pty = None;

        let (rows, cols) = self.output_size;
        let cmd = match agent {
            PtyAgentKind::Claude => self.config.claude.clone(),
            PtyAgentKind::Codex => self.config.codex.clone(),
        };
        match PtySession::spawn(id.clone(), &cmd, rows, cols, self.tx.clone()) {
            Ok(session) => {
                self.active_mut().pty = Some(session);
                self.focus = Focus::Output;
                self.status = format!("Started {} ({})", self.active().session.name, cmd.command);
            }
            Err(err) => {
                self.status = format!("Failed to start {}: {err}", self.active().session.name);
            }
        }
    }

    fn send_http(&mut self) {
        let draft = self.active().session.draft_prompt.trim().to_string();
        if draft.is_empty() {
            self.status = "Type a prompt (F2 output), then Enter or Start".to_string();
            return;
        }

        if let Some(task) = self.active_mut().http_task.take() {
            task.abort();
        }

        self.active_mut().session.messages.push(ChatMessage::user(&draft));
        self.active_mut().session.draft_prompt.clear();
        self.active_mut().session.touch();
        self.active_mut().streaming.clear();
        self.active_mut().http_running = true;

        let session_id = self.active().session.id.clone();
        let messages = self.active().session.messages.clone();
        let params = self.params();

        let provider = match &self.active().session.kind {
            SessionKind::Http { provider } => *provider,
            _ => return,
        };
        let cfg = self.config.http_for(provider);
        let model = cfg.model.clone();
        self.active_mut().session.model = model;

        let tx = self.tx.clone();
        self.active_mut().http_task = Some(tokio::spawn(async move {
            openai_compat::stream_chat(session_id, cfg, params, messages, tx).await;
        }));

        self.focus = Focus::Output;
        self.status = format!("{}: streaming...", self.active().session.name);
    }

    fn stop_active(&mut self) {
        match &self.active().session.kind {
            SessionKind::Pty { .. } => {
                self.active_mut().pty = None;
            }
            SessionKind::Http { .. } => {
                if let Some(task) = self.active_mut().http_task.take() {
                    task.abort();
                }
                self.active_mut().http_running = false;
                self.active_mut().streaming.clear();
            }
        }
        self.status = format!("Stopped {}", self.active().session.name);
    }

    fn clear_active(&mut self) {
        match &self.active().session.kind {
            SessionKind::Http { .. } => {
                self.active_mut().session.messages.clear();
                self.active_mut().session.draft_prompt.clear();
                self.active_mut().streaming.clear();
                self.active_mut().last_run = None;
                self.active_mut().session.touch();
                self.status = "Conversation cleared".to_string();
            }
            SessionKind::Pty { .. } => {
                self.status = "Clear N/A for PTY sessions".to_string();
            }
        }
    }

    pub fn save_all(&mut self) {
        self.sync_scrubbers_to_active();
        self.config.params = self.params();
        match self.config.save() {
            Ok(()) => {}
            Err(err) => {
                self.status = format!("Config save failed: {err}");
                return;
            }
        }
        for rt in &self.sessions {
            if let Err(err) = SessionStore::save_session(&rt.session) {
                self.status = format!("Session save failed: {err}");
                return;
            }
        }
        self.status = "Saved config + sessions".to_string();
    }
}

fn scrubbers_from_params(p: &Params) -> Vec<Scrubber> {
    vec![
        Scrubber::new("Temperature", p.temperature, 0.0, 2.0, 0.05, 2),
        Scrubber::new("Max Tokens", p.max_tokens, 1.0, 8192.0, 64.0, 0),
        Scrubber::new("Top P", p.top_p, 0.0, 1.0, 0.05, 2),
    ]
}

fn contains(r: Rect, p: (u16, u16)) -> bool {
    p.0 >= r.x && p.0 < r.x + r.width && p.1 >= r.y && p.1 < r.y + r.height
}

fn key_to_bytes(key: &KeyEvent) -> Option<Vec<u8>> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Char(c) => {
            if ctrl {
                let upper = c.to_ascii_uppercase();
                if upper.is_ascii_alphabetic() {
                    return Some(vec![(upper as u8) - 0x40]);
                }
                if c == ' ' {
                    return Some(vec![0]);
                }
            }
            let mut buf = [0u8; 4];
            Some(c.encode_utf8(&mut buf).as_bytes().to_vec())
        }
        KeyCode::Enter => Some(vec![b'\r']),
        KeyCode::Backspace => Some(vec![0x7f]),
        KeyCode::Tab => Some(vec![b'\t']),
        KeyCode::BackTab => Some(b"\x1b[Z".to_vec()),
        KeyCode::Esc => Some(vec![0x1b]),
        KeyCode::Left => Some(b"\x1b[D".to_vec()),
        KeyCode::Right => Some(b"\x1b[C".to_vec()),
        KeyCode::Up => Some(b"\x1b[A".to_vec()),
        KeyCode::Down => Some(b"\x1b[B".to_vec()),
        KeyCode::Home => Some(b"\x1b[H".to_vec()),
        KeyCode::End => Some(b"\x1b[F".to_vec()),
        KeyCode::PageUp => Some(b"\x1b[5~".to_vec()),
        KeyCode::PageDown => Some(b"\x1b[6~".to_vec()),
        KeyCode::Delete => Some(b"\x1b[3~".to_vec()),
        KeyCode::Insert => Some(b"\x1b[2~".to_vec()),
        _ => None,
    }
}
