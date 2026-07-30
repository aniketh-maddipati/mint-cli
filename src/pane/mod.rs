use ratatui::layout::{Constraint, Layout, Rect};
use uuid::Uuid;

use crate::agents::pty::PtySession;
use crate::config::{AgentCmd, Config};
use tokio::sync::mpsc::UnboundedSender;

use crate::app::AppEvent;

/// What runs inside a pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaneKind {
    Claude,
    Codex,
    Command {
        label: String,
        cmd: AgentCmd,
    },
}

impl PaneKind {
    pub fn title(&self) -> &str {
        match self {
            PaneKind::Claude => "Claude",
            PaneKind::Codex => "Codex",
            PaneKind::Command { label, .. } => label,
        }
    }

    pub fn is_agent(&self) -> bool {
        matches!(self, PaneKind::Claude | PaneKind::Codex)
    }

    fn agent_cmd(&self, config: &Config) -> Option<AgentCmd> {
        match self {
            PaneKind::Claude => Some(config.claude.clone()),
            PaneKind::Codex => Some(config.codex.clone()),
            PaneKind::Command { cmd, .. } => Some(cmd.clone()),
        }
    }
}

/// A terminal pane backed by an optional live PTY session.
pub struct Pane {
    pub id: String,
    pub kind: PaneKind,
    pub pty: Option<PtySession>,
}

impl Pane {
    pub fn new(kind: PaneKind) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            kind,
            pty: None,
        }
    }

    pub fn title(&self) -> &str {
        self.kind.title()
    }

    pub fn is_running(&self) -> bool {
        self.pty.is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutMode {
    /// One pane fills the workspace.
    Single,
    /// Claude | Codex side-by-side (creates missing agent panes on demand).
    Split,
}

/// Manages workspace panes, focus, layout, and lazy PTY startup.
pub struct PaneManager {
    pub panes: Vec<Pane>,
    pub active: usize,
    pub layout: LayoutMode,
    pub timeline_visible: bool,
}

impl PaneManager {
    pub fn from_config(config: &Config) -> Self {
        let mut panes = vec![Pane::new(PaneKind::Claude)];

        for cmd in &config.commands {
            panes.push(Pane::new(PaneKind::Command {
                label: cmd.label.clone(),
                cmd: cmd.cmd.clone(),
            }));
        }

        Self {
            panes,
            active: 0,
            layout: LayoutMode::Single,
            timeline_visible: false,
        }
    }

    pub fn active(&self) -> &Pane {
        &self.panes[self.active]
    }

    pub fn active_mut(&mut self) -> &mut Pane {
        &mut self.panes[self.active]
    }

    pub fn pane(&self, id: &str) -> Option<&Pane> {
        self.panes.iter().find(|p| p.id == id)
    }

    pub fn pane_mut(&mut self, id: &str) -> Option<&mut Pane> {
        self.panes.iter_mut().find(|p| p.id == id)
    }

    pub fn pane_index(&self, id: &str) -> Option<usize> {
        self.panes.iter().position(|p| p.id == id)
    }

    pub fn focus_next(&mut self) {
        if self.panes.len() <= 1 {
            return;
        }
        self.active = (self.active + 1) % self.panes.len();
    }

    pub fn focus_prev(&mut self) {
        if self.panes.len() <= 1 {
            return;
        }
        self.active = self.active.checked_sub(1).unwrap_or(self.panes.len() - 1);
    }

    pub fn focus_index(&mut self, idx: usize) {
        if idx < self.panes.len() {
            self.active = idx;
        }
    }

    pub fn toggle_split(&mut self) {
        self.layout = match self.layout {
            LayoutMode::Single => LayoutMode::Split,
            LayoutMode::Split => LayoutMode::Single,
        };
        self.ensure_split_panes();
    }

    pub fn toggle_timeline(&mut self) {
        self.timeline_visible = !self.timeline_visible;
    }

    /// Ensure Claude and Codex panes exist when entering split layout.
    fn ensure_split_panes(&mut self) {
        if self.layout != LayoutMode::Split {
            return;
        }
        for kind in [PaneKind::Claude, PaneKind::Codex] {
            if !self.panes.iter().any(|p| p.kind == kind) {
                self.panes.push(Pane::new(kind));
            }
        }
    }

    /// Panes visible in the current layout.
    pub fn visible_indices(&self) -> Vec<usize> {
        match self.layout {
            LayoutMode::Single => vec![self.active],
            LayoutMode::Split => {
                let mut indices = Vec::new();
                for kind in [PaneKind::Claude, PaneKind::Codex] {
                    if let Some(i) = self.panes.iter().position(|p| p.kind == kind) {
                        indices.push(i);
                    }
                }
                if indices.is_empty() {
                    vec![self.active]
                } else {
                    indices
                }
            }
        }
    }

    /// Compute render rects for each visible pane within `area`.
    pub fn layout_rects(&self, area: Rect) -> Vec<(usize, Rect)> {
        let indices = self.visible_indices();
        if indices.is_empty() {
            return Vec::new();
        }
        if indices.len() == 1 {
            return vec![(indices[0], area)];
        }
        let cols = Layout::horizontal([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)]).split(area);
        vec![(indices[0], cols[0]), (indices[1], cols[1])]
    }

    /// Spawn the PTY for `pane_idx` if not already running.
    pub fn ensure_started(
        &mut self,
        pane_idx: usize,
        rows: u16,
        cols: u16,
        config: &Config,
        tx: UnboundedSender<AppEvent>,
    ) -> Result<(), String> {
        if pane_idx >= self.panes.len() {
            return Ok(());
        }
        if self.panes[pane_idx].pty.is_some() {
            return Ok(());
        }
        let kind = self.panes[pane_idx].kind.clone();
        let Some(cmd) = kind.agent_cmd(config) else {
            return Ok(());
        };
        let id = self.panes[pane_idx].id.clone();
        match PtySession::spawn(id.clone(), &cmd, rows, cols, tx) {
            Ok(session) => {
                self.panes[pane_idx].pty = Some(session);
                Ok(())
            }
            Err(err) => Err(format!("Failed to start {}: {err}", kind.title())),
        }
    }

    /// Resize every live PTY to `(rows, cols)`.
    pub fn resize_all(&mut self, rows: u16, cols: u16) {
        for pane in &mut self.panes {
            if let Some(s) = pane.pty.as_mut() {
                s.resize(rows, cols);
            }
        }
    }

    pub fn stop_pane(&mut self, pane_idx: usize) {
        if pane_idx < self.panes.len() {
            self.panes[pane_idx].pty = None;
        }
    }

    pub fn stop_active(&mut self) {
        let active = self.active;
        self.stop_pane(active);
    }
}
