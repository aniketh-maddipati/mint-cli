use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config::Params;

pub type BranchId = String;

pub fn main_branch() -> BranchId {
    "main".to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
}

impl ChatMessage {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
        }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PtyAgentKind {
    Claude,
    Codex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HttpProvider {
    Tinker,
    LmStudio,
}

impl HttpProvider {
    pub fn title(self) -> &'static str {
        match self {
            HttpProvider::Tinker => "Tinker",
            HttpProvider::LmStudio => "LM Studio",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionKind {
    Pty { agent: PtyAgentKind },
    Http { provider: HttpProvider },
}

impl SessionKind {
    pub fn is_pty(&self) -> bool {
        matches!(self, SessionKind::Pty { .. })
    }

    pub fn is_http(&self) -> bool {
        matches!(self, SessionKind::Http { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunStatus {
    Done,
    Error,
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunTag {
    Ok,
    Fail,
    Salvage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunNode {
    pub id: String,
    pub branch: BranchId,
    pub parent: Option<String>,
    pub started_at: String,
    pub duration_ms: u64,
    pub status: RunStatus,
    pub model: String,
    pub request_messages: Vec<ChatMessage>,
    pub response: String,
    pub image_paths: Vec<String>,
    pub tag: Option<RunTag>,
    pub note: String,
    pub error: Option<String>,
}

impl RunNode {
    pub fn new(model: String, request_messages: Vec<ChatMessage>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            branch: main_branch(),
            parent: None,
            started_at: now_rfc3339(),
            duration_ms: 0,
            status: RunStatus::Done,
            model,
            request_messages,
            response: String::new(),
            image_paths: Vec::new(),
            tag: None,
            note: String::new(),
            error: None,
        }
    }

    pub fn short_id(&self) -> &str {
        &self.id[..self.id.len().min(8)]
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub name: String,
    pub kind: SessionKind,
    pub created_at: String,
    pub updated_at: String,
    pub paused: bool,
    pub messages: Vec<ChatMessage>,
    pub draft_prompt: String,
    pub params: Params,
    pub model: String,
    pub active_branch: BranchId,
    pub playhead_run: Option<String>,
}

impl Session {
    pub fn new(name: impl Into<String>, kind: SessionKind, model: String, params: Params) -> Self {
        let now = now_rfc3339();
        Self {
            id: Uuid::new_v4().to_string(),
            name: name.into(),
            kind,
            created_at: now.clone(),
            updated_at: now,
            paused: false,
            messages: Vec::new(),
            draft_prompt: String::new(),
            params,
            model,
            active_branch: main_branch(),
            playhead_run: None,
        }
    }

    pub fn touch(&mut self) {
        self.updated_at = now_rfc3339();
    }

    pub fn is_http(&self) -> bool {
        self.kind.is_http()
    }

    pub fn is_pty(&self) -> bool {
        self.kind.is_pty()
    }
}

pub struct SessionStore;

impl SessionStore {
    pub fn root() -> Result<PathBuf> {
        let dirs = ProjectDirs::from("dev", "mint", "mint-cli")
            .context("could not resolve data directory")?;
        Ok(dirs.data_dir().join("sessions"))
    }

    pub fn session_dir(id: &str) -> Result<PathBuf> {
        Ok(Self::root()?.join(id))
    }

    pub fn runs_dir(session_id: &str) -> Result<PathBuf> {
        Ok(Self::session_dir(session_id)?.join("runs"))
    }

    pub fn load_all() -> Result<Vec<Session>> {
        let root = Self::root()?;
        if !root.exists() {
            return Ok(Vec::new());
        }
        let mut sessions = Vec::new();
        for entry in std::fs::read_dir(&root).context("read sessions dir")? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let path = entry.path().join("session.json");
            if path.exists() {
                if let Ok(session) = Self::load_session_file(&path) {
                    sessions.push(session);
                }
            }
        }
        sessions.sort_by(|a, b| a.updated_at.cmp(&b.updated_at));
        Ok(sessions)
    }

    pub fn load_session(id: &str) -> Result<Session> {
        let path = Self::session_dir(id)?.join("session.json");
        Self::load_session_file(&path)
    }

    fn load_session_file(path: &Path) -> Result<Session> {
        let text = std::fs::read_to_string(path).context("read session.json")?;
        let session: Session = serde_json::from_str(&text).context("parse session.json")?;
        Ok(session)
    }

    pub fn save_session(session: &Session) -> Result<()> {
        let dir = Self::session_dir(&session.id)?;
        std::fs::create_dir_all(&dir).context("create session dir")?;
        std::fs::create_dir_all(dir.join("runs")).ok();
        let path = dir.join("session.json");
        let text = serde_json::to_string_pretty(session).context("serialize session")?;
        std::fs::write(path, text).context("write session.json")?;
        Ok(())
    }

    pub fn save_run(session_id: &str, run: &RunNode) -> Result<()> {
        let dir = Self::runs_dir(session_id)?;
        std::fs::create_dir_all(&dir).context("create runs dir")?;
        let path = dir.join(format!("{}.json", run.id));
        let text = serde_json::to_string_pretty(run).context("serialize run")?;
        std::fs::write(path, text).context("write run.json")?;
        Ok(())
    }
}

pub fn now_rfc3339() -> String {
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}.{:03}Z", dur.as_secs(), dur.subsec_millis())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_json_roundtrip() {
        let session = Session::new(
            "test",
            SessionKind::Http {
                provider: HttpProvider::Tinker,
            },
            "tinker://checkpoint".to_string(),
            Params {
                temperature: 0.7,
                max_tokens: 100.0,
                top_p: 1.0,
            },
        );
        let json = serde_json::to_string(&session).unwrap();
        let parsed: Session = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, "test");
        assert_eq!(parsed.model, "tinker://checkpoint");
    }

    #[test]
    fn run_node_roundtrip() {
        let mut run = RunNode::new("model".to_string(), vec![ChatMessage::user("hi")]);
        run.response = "hello".to_string();
        run.status = RunStatus::Done;
        let json = serde_json::to_string(&run).unwrap();
        let parsed: RunNode = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.response, "hello");
    }
}
