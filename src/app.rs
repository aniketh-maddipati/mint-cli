use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use tokio::sync::mpsc::UnboundedSender;

use crate::config::Config;
use crate::pane::{LayoutMode, PaneManager};
use crate::project::{ProjectId, ProjectRegistry};
use crate::session::{ProjectStore, Stage};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Pty,
    Timeline,
}

/// Events funneled into the single-threaded update loop from every source.
pub enum AppEvent {
    Input(Event),
    PtyOutput(String, Vec<u8>),
    PtyExited(String),
    Tick,
}

/// Per-pane render geometry captured during draw.
#[derive(Default, Clone)]
pub struct PaneLayout {
    pub pane_idx: usize,
    pub area: Rect,
    pub inner: Rect,
}

#[derive(Default)]
pub struct LayoutRects {
    pub panes: Vec<PaneLayout>,
    pub timeline: Rect,
}

pub struct App {
    pub config: Config,
    pub project_id: ProjectId,
    pub stages: Vec<Stage>,
    pub active_stage: usize,
    pub panes: PaneManager,
    pub focus: Focus,
    pub status: String,
    pub should_quit: bool,
    pub layout: LayoutRects,
    pane_sizes: Vec<(u16, u16)>,
    tx: UnboundedSender<AppEvent>,
}

impl App {
    pub fn new(config: Config, tx: UnboundedSender<AppEvent>) -> Self {
        let (project_id, _cwd) = ProjectRegistry::detect_current().unwrap_or_else(|_| {
            (
                ProjectRegistry::detect(std::path::Path::new(".")),
                std::path::PathBuf::from("."),
            )
        });

        let stages = ProjectStore::load_or_init(&project_id).unwrap_or_else(|_| {
            vec![Stage::new("default")]
        });
        let active_stage = stages.len().saturating_sub(1);
        let panes = PaneManager::from_config(&config);

        Self {
            config,
            project_id,
            stages,
            active_stage,
            panes,
            focus: Focus::Pty,
            status: String::new(),
            should_quit: false,
            layout: LayoutRects::default(),
            pane_sizes: Vec::new(),
            tx,
        }
    }

    pub fn active_stage(&self) -> &Stage {
        &self.stages[self.active_stage]
    }

    pub fn set_pane_layouts(&mut self, layouts: Vec<PaneLayout>) {
        self.pane_sizes = layouts
            .iter()
            .map(|l| (l.inner.height.max(1), l.inner.width.max(1)))
            .collect();
        self.layout.panes = layouts;
    }

    pub fn ensure_visible_panes_started(&mut self) {
        let indices: Vec<usize> = self.panes.visible_indices();
        let sizes: Vec<(u16, u16)> = self.pane_sizes.clone();
        let tx = self.tx.clone();
        let config = self.config.clone();

        for (i, &pane_idx) in indices.iter().enumerate() {
            let (rows, cols) = sizes.get(i).copied().unwrap_or((24, 80));
            if let Err(err) = self
                .panes
                .ensure_started(pane_idx, rows, cols, &config, tx.clone())
            {
                self.status = err;
            }
        }
    }

    pub fn resize_live_ptys(&mut self) {
        for layout in &self.layout.panes {
            let rows = layout.inner.height.max(1);
            let cols = layout.inner.width.max(1);
            if let Some(pane) = self.panes.panes.get_mut(layout.pane_idx) {
                if let Some(s) = pane.pty.as_mut() {
                    s.resize(rows, cols);
                }
            }
        }
    }

    pub fn handle_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::Input(ev) => self.handle_input(ev),
            AppEvent::PtyOutput(id, bytes) => {
                if let Some(pane) = self.panes.pane_mut(&id) {
                    if let Some(s) = pane.pty.as_mut() {
                        s.feed(&bytes);
                    }
                }
            }
            AppEvent::PtyExited(id) => {
                if let Some(pane) = self.panes.pane_mut(&id) {
                    let title = pane.title().to_string();
                    pane.pty = None;
                    self.status = format!("{title} exited");
                }
            }
            AppEvent::Tick => {}
        }
    }

    fn handle_input(&mut self, event: Event) {
        match event {
            Event::Key(key) => self.handle_key(key),
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
            KeyCode::F(3) => {
                self.panes.toggle_timeline();
                self.focus = if self.panes.timeline_visible {
                    Focus::Timeline
                } else {
                    Focus::Pty
                };
                return;
            }
            KeyCode::F(10) => {
                self.should_quit = true;
                return;
            }
            KeyCode::Char('\\') if ctrl => {
                self.panes.toggle_split();
                return;
            }
            KeyCode::Char(']') if !ctrl => {
                self.panes.focus_next();
                return;
            }
            KeyCode::Char('[') if !ctrl => {
                self.panes.focus_prev();
                return;
            }
            KeyCode::Char('r') if ctrl => {
                let active = self.panes.active;
                self.panes.stop_pane(active);
                return;
            }
            _ => {}
        }

        if self.focus == Focus::Timeline {
            self.handle_timeline_key(key);
            return;
        }

        self.handle_pty_key(key);
    }

    fn handle_timeline_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::F(3) => {
                self.panes.timeline_visible = false;
                self.focus = Focus::Pty;
            }
            KeyCode::Up => {
                self.active_stage = self.active_stage.saturating_sub(1);
            }
            KeyCode::Down => {
                self.active_stage = (self.active_stage + 1).min(self.stages.len().saturating_sub(1));
            }
            _ => {}
        }
    }

    fn handle_pty_key(&mut self, key: KeyEvent) {
        let active_idx = if self.panes.layout == LayoutMode::Split {
            // In split mode, route keys to the focused agent pane.
            self.panes.active
        } else {
            self.panes.active
        };

        if let Some(bytes) = key_to_bytes(&key) {
            if let Some(pane) = self.panes.panes.get_mut(active_idx) {
                if let Some(s) = pane.pty.as_mut() {
                    s.write_input(&bytes);
                }
            }
        }
    }

    pub fn project_label(&self) -> &str {
        self.project_id.as_str()
    }

    pub fn layout_mode_label(&self) -> &'static str {
        match self.panes.layout {
            LayoutMode::Single => "single",
            LayoutMode::Split => "split",
        }
    }
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
