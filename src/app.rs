use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Rect;
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinHandle;

use crate::agents::lmstudio;
use crate::agents::pty::PtySession;
use crate::config::{Config, Params};
use crate::ui::scrubber::Scrubber;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentKind {
    Claude,
    Codex,
    LmStudio,
}

impl AgentKind {
    pub const ALL: [AgentKind; 3] = [AgentKind::Claude, AgentKind::Codex, AgentKind::LmStudio];

    pub fn title(self) -> &'static str {
        match self {
            AgentKind::Claude => "Claude",
            AgentKind::Codex => "Codex",
            AgentKind::LmStudio => "LM Studio",
        }
    }

    pub fn is_pty(self) -> bool {
        matches!(self, AgentKind::Claude | AgentKind::Codex)
    }

    fn index(self) -> usize {
        match self {
            AgentKind::Claude => 0,
            AgentKind::Codex => 1,
            AgentKind::LmStudio => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Output,
    Controls,
    Scrubbers,
}

#[derive(Debug, Clone, Copy)]
pub enum Action {
    Start,
    Stop,
    Save,
}

impl Action {
    pub const ALL: [Action; 3] = [Action::Start, Action::Stop, Action::Save];

    pub fn label(self) -> &'static str {
        match self {
            Action::Start => "Start",
            Action::Stop => "Stop",
            Action::Save => "Save",
        }
    }
}

/// Events funneled into the single-threaded update loop from every source.
pub enum AppEvent {
    Input(Event),
    PtyOutput(AgentKind, Vec<u8>),
    PtyExited(AgentKind),
    LmChunk(String),
    LmDone,
    LmError(String),
    Tick,
}

/// Rects captured during render so pointer events can be hit-tested.
#[derive(Default)]
pub struct LayoutRects {
    pub tabs: Vec<Rect>,
    pub buttons: Vec<Rect>,
    pub scrubber_bars: Vec<Rect>,
    pub output: Rect,
}

pub struct App {
    pub config: Config,
    pub active: AgentKind,
    pub focus: Focus,

    pub claude: Option<PtySession>,
    pub codex: Option<PtySession>,

    pub lm_output: String,
    pub lm_streaming: bool,
    pub lm_prompt: String,
    lm_task: Option<JoinHandle<()>>,

    pub scrubbers: Vec<Scrubber>,
    pub scrubber_sel: usize,
    pub button_sel: usize,

    pub status: String,
    pub should_quit: bool,

    pub layout: LayoutRects,
    /// Inner size (rows, cols) of the output pane at last render.
    output_size: (u16, u16),

    tx: UnboundedSender<AppEvent>,
}

impl App {
    pub fn new(config: Config, tx: UnboundedSender<AppEvent>) -> Self {
        let p = &config.params;
        let scrubbers = vec![
            Scrubber::new("Temperature", p.temperature, 0.0, 2.0, 0.05, 2),
            Scrubber::new("Max Tokens", p.max_tokens, 1.0, 8192.0, 64.0, 0),
            Scrubber::new("Top P", p.top_p, 0.0, 1.0, 0.05, 2),
        ];
        Self {
            config,
            active: AgentKind::Claude,
            focus: Focus::Controls,
            claude: None,
            codex: None,
            lm_output: String::new(),
            lm_streaming: false,
            lm_prompt: String::new(),
            lm_task: None,
            scrubbers,
            scrubber_sel: 0,
            button_sel: 0,
            status: "Ready. F2 output - F3 controls - F4 scrubbers - Ctrl+Q quit".to_string(),
            should_quit: false,
            layout: LayoutRects::default(),
            output_size: (24, 80),
            tx,
        }
    }

    pub fn params(&self) -> Params {
        Params {
            temperature: self.scrubbers[0].value,
            max_tokens: self.scrubbers[1].value,
            top_p: self.scrubbers[2].value,
        }
    }

    pub fn pty(&self, kind: AgentKind) -> Option<&PtySession> {
        match kind {
            AgentKind::Claude => self.claude.as_ref(),
            AgentKind::Codex => self.codex.as_ref(),
            AgentKind::LmStudio => None,
        }
    }

    fn pty_mut(&mut self, kind: AgentKind) -> Option<&mut PtySession> {
        match kind {
            AgentKind::Claude => self.claude.as_mut(),
            AgentKind::Codex => self.codex.as_mut(),
            AgentKind::LmStudio => None,
        }
    }

    /// Record the output pane size so spawns and resizes match the render area.
    pub fn set_output_size(&mut self, rows: u16, cols: u16) {
        self.output_size = (rows.max(1), cols.max(1));
        if let Some(s) = self.pty_mut(self.active) {
            s.resize(rows, cols);
        }
    }

    pub fn handle_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::Input(ev) => self.handle_input(ev),
            AppEvent::PtyOutput(kind, bytes) => {
                if let Some(s) = self.pty_mut(kind) {
                    s.feed(&bytes);
                }
            }
            AppEvent::PtyExited(kind) => {
                self.status = format!("{} exited", kind.title());
            }
            AppEvent::LmChunk(text) => self.lm_output.push_str(&text),
            AppEvent::LmDone => {
                self.lm_streaming = false;
                self.status = "LM Studio: done".to_string();
            }
            AppEvent::LmError(err) => {
                self.lm_streaming = false;
                self.status = format!("LM Studio error: {err}");
            }
            AppEvent::Tick => {}
        }
    }

    fn handle_input(&mut self, event: Event) {
        match event {
            Event::Key(key) => self.handle_key(key),
            Event::Mouse(m) => self.handle_mouse(m),
            _ => {}
        }
    }

    fn handle_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        // Global bindings that win over everything, including PTY forwarding.
        match key.code {
            KeyCode::Char('q') if ctrl => {
                self.should_quit = true;
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
            _ => {}
        }

        match self.focus {
            Focus::Output => self.handle_output_key(key),
            Focus::Controls => self.handle_controls_key(key),
            Focus::Scrubbers => self.handle_scrubbers_key(key),
        }
    }

    fn handle_output_key(&mut self, key: KeyEvent) {
        if self.active.is_pty() {
            let kind = self.active;
            if let Some(bytes) = key_to_bytes(&key) {
                if let Some(s) = self.pty_mut(kind) {
                    s.write_input(&bytes);
                }
            }
            return;
        }

        // LM Studio prompt editing.
        match key.code {
            KeyCode::Char(c) => self.lm_prompt.push(c),
            KeyCode::Backspace => {
                self.lm_prompt.pop();
            }
            KeyCode::Enter => self.start_lm(),
            KeyCode::Esc => self.focus = Focus::Controls,
            _ => {}
        }
    }

    fn handle_controls_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('1') => self.select_agent(AgentKind::Claude),
            KeyCode::Char('2') => self.select_agent(AgentKind::Codex),
            KeyCode::Char('3') => self.select_agent(AgentKind::LmStudio),
            KeyCode::Left => {
                self.button_sel = self.button_sel.saturating_sub(1);
            }
            KeyCode::Right => {
                self.button_sel = (self.button_sel + 1).min(Action::ALL.len() - 1);
            }
            KeyCode::Enter | KeyCode::Char(' ') => self.run_action(Action::ALL[self.button_sel]),
            KeyCode::Tab => self.focus = Focus::Scrubbers,
            KeyCode::BackTab => self.focus = Focus::Output,
            _ => {}
        }
    }

    fn handle_scrubbers_key(&mut self, key: KeyEvent) {
        let big = key.modifiers.contains(KeyModifiers::SHIFT);
        let mult = if big { 5.0 } else { 1.0 };
        match key.code {
            KeyCode::Char('1') => self.select_agent(AgentKind::Claude),
            KeyCode::Char('2') => self.select_agent(AgentKind::Codex),
            KeyCode::Char('3') => self.select_agent(AgentKind::LmStudio),
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
    }

    fn handle_mouse(&mut self, m: MouseEvent) {
        let point = (m.column, m.row);
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                for (i, r) in self.layout.tabs.clone().iter().enumerate() {
                    if contains(*r, point) {
                        self.select_agent(AgentKind::ALL[i]);
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

    fn select_agent(&mut self, kind: AgentKind) {
        self.active = kind;
        self.status = format!("Selected {}", kind.title());
        let (rows, cols) = self.output_size;
        if let Some(s) = self.pty_mut(kind) {
            s.resize(rows, cols);
        }
    }

    fn run_action(&mut self, action: Action) {
        match action {
            Action::Start => self.start_active(),
            Action::Stop => self.stop_active(),
            Action::Save => self.save_config(),
        }
    }

    fn start_active(&mut self) {
        match self.active {
            AgentKind::Claude | AgentKind::Codex => self.start_pty(self.active),
            AgentKind::LmStudio => self.start_lm(),
        }
    }

    fn start_pty(&mut self, kind: AgentKind) {
        let (rows, cols) = self.output_size;
        let cmd = match kind {
            AgentKind::Claude => self.config.claude.clone(),
            AgentKind::Codex => self.config.codex.clone(),
            AgentKind::LmStudio => return,
        };
        match PtySession::spawn(kind, &cmd, rows, cols, self.tx.clone()) {
            Ok(session) => {
                match kind {
                    AgentKind::Claude => self.claude = Some(session),
                    AgentKind::Codex => self.codex = Some(session),
                    AgentKind::LmStudio => {}
                }
                self.status = format!("Started {} ({})", kind.title(), cmd.command);
            }
            Err(err) => self.status = format!("Failed to start {}: {err}", kind.title()),
        }
    }

    fn start_lm(&mut self) {
        if self.lm_prompt.trim().is_empty() {
            self.status = "Type a prompt (focus output with F2), then Enter".to_string();
            return;
        }
        if let Some(task) = self.lm_task.take() {
            task.abort();
        }
        self.lm_output.clear();
        self.lm_streaming = true;
        self.status = "LM Studio: streaming...".to_string();

        let cfg = self.config.lmstudio.clone();
        let params = self.params();
        let prompt = self.lm_prompt.clone();
        let tx = self.tx.clone();
        self.lm_task = Some(tokio::spawn(async move {
            lmstudio::stream_chat(cfg, params, prompt, tx).await;
        }));
    }

    fn stop_active(&mut self) {
        match self.active {
            AgentKind::Claude => {
                self.claude = None;
            }
            AgentKind::Codex => {
                self.codex = None;
            }
            AgentKind::LmStudio => {
                if let Some(task) = self.lm_task.take() {
                    task.abort();
                }
                self.lm_streaming = false;
            }
        }
        self.status = format!("Stopped {}", self.active.title());
    }

    fn save_config(&mut self) {
        self.config.params = self.params();
        match self.config.save() {
            Ok(()) => self.status = "Config saved".to_string(),
            Err(err) => self.status = format!("Save failed: {err}"),
        }
    }

    pub fn active_tab_index(&self) -> usize {
        self.active.index()
    }
}

fn contains(r: Rect, p: (u16, u16)) -> bool {
    p.0 >= r.x && p.0 < r.x + r.width && p.1 >= r.y && p.1 < r.y + r.height
}

/// Translate a key event into the byte sequence a terminal would send.
fn key_to_bytes(key: &KeyEvent) -> Option<Vec<u8>> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Char(c) => {
            if ctrl {
                let upper = c.to_ascii_uppercase();
                if upper.is_ascii_alphabetic() {
                    return Some(vec![(upper as u8) - 0x40]);
                }
                match c {
                    ' ' => return Some(vec![0]),
                    _ => {}
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
