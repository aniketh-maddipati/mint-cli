use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::project::ProjectId;
use crate::project::ProjectRegistry;

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

/// A project-scoped stage in the debug timeline (replaces chat sessions).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stage {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub updated_at: String,
    pub active_branch: BranchId,
    pub playhead_run: Option<String>,
}

impl Stage {
    pub fn new(name: impl Into<String>) -> Self {
        let now = now_rfc3339();
        Self {
            id: Uuid::new_v4().to_string(),
            name: name.into(),
            created_at: now.clone(),
            updated_at: now,
            active_branch: main_branch(),
            playhead_run: None,
        }
    }

    pub fn touch(&mut self) {
        self.updated_at = now_rfc3339();
    }
}

pub struct ProjectStore;

impl ProjectStore {
    pub fn root(project_id: &ProjectId) -> Result<PathBuf> {
        ProjectRegistry::data_dir(project_id)
    }

    pub fn stages_dir(project_id: &ProjectId) -> Result<PathBuf> {
        Ok(Self::root(project_id)?.join("stages"))
    }

    pub fn stage_dir(project_id: &ProjectId, stage_id: &str) -> Result<PathBuf> {
        Ok(Self::stages_dir(project_id)?.join(stage_id))
    }

    pub fn runs_dir(project_id: &ProjectId, stage_id: &str) -> Result<PathBuf> {
        Ok(Self::stage_dir(project_id, stage_id)?.join("runs"))
    }

    pub fn load_stages(project_id: &ProjectId) -> Result<Vec<Stage>> {
        let root = Self::stages_dir(project_id)?;
        if !root.exists() {
            return Ok(Vec::new());
        }
        let mut stages = Vec::new();
        for entry in std::fs::read_dir(&root).context("read stages dir")? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let path = entry.path().join("stage.json");
            if path.exists() {
                if let Ok(stage) = Self::load_stage_file(&path) {
                    stages.push(stage);
                }
            }
        }
        stages.sort_by(|a, b| a.updated_at.cmp(&b.updated_at));
        Ok(stages)
    }

    pub fn load_or_init(project_id: &ProjectId) -> Result<Vec<Stage>> {
        let mut stages = Self::load_stages(project_id)?;
        if stages.is_empty() {
            let stage = Stage::new("default");
            Self::save_stage(project_id, &stage)?;
            stages.push(stage);
        }
        Ok(stages)
    }

    fn load_stage_file(path: &Path) -> Result<Stage> {
        let text = std::fs::read_to_string(path).context("read stage.json")?;
        let stage: Stage = serde_json::from_str(&text).context("parse stage.json")?;
        Ok(stage)
    }

    pub fn save_stage(project_id: &ProjectId, stage: &Stage) -> Result<()> {
        let dir = Self::stage_dir(project_id, &stage.id)?;
        std::fs::create_dir_all(&dir).context("create stage dir")?;
        std::fs::create_dir_all(dir.join("runs")).ok();
        let path = dir.join("stage.json");
        let text = serde_json::to_string_pretty(stage).context("serialize stage")?;
        std::fs::write(path, text).context("write stage.json")?;
        Ok(())
    }

    pub fn save_run(project_id: &ProjectId, stage_id: &str, run: &RunNode) -> Result<()> {
        let dir = Self::runs_dir(project_id, stage_id)?;
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
    fn stage_json_roundtrip() {
        let stage = Stage::new("eval");
        let json = serde_json::to_string(&stage).unwrap();
        let parsed: Stage = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, "eval");
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
