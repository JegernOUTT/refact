use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub struct Item {
    pub kind: ItemKind,
    pub id: String,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    Chat,
    Task,
}
impl Item {
    pub fn key(&self) -> String {
        format!(
            "{}:{}",
            match self.kind {
                ItemKind::Chat => "chat",
                ItemKind::Task => "task",
            },
            self.id
        )
    }
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaxonomyKind {
    Topics,
    Tags,
}
impl TaxonomyKind {
    pub fn key(self) -> &'static str {
        match self {
            Self::Topics => "topics",
            Self::Tags => "tags",
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Taxon {
    pub id: String,
    pub name: String,
    pub archived: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Assignment {
    pub item: Item,
    pub topic_id: Option<String>,
    pub tag_ids: Vec<String>,
    pub topic_locked: bool,
    pub tags_locked: bool,
}
impl Assignment {
    pub fn empty(item: Item) -> Self {
        Self {
            item,
            topic_id: None,
            tag_ids: vec![],
            topic_locked: false,
            tags_locked: false,
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Op {
    CreateTopic {
        name: String,
    },
    RenameTopic {
        id: String,
        name: String,
    },
    MergeTopic {
        from_id: String,
        into_id: String,
    },
    ArchiveTopic {
        id: String,
    },
    CreateTag {
        name: String,
    },
    RenameTag {
        id: String,
        name: String,
    },
    MergeTag {
        from_id: String,
        into_id: String,
    },
    ArchiveTag {
        id: String,
    },
    AssignItem {
        item: Item,
        topic_id: Option<String>,
        tag_ids: Vec<String>,
        topic_locked: Option<bool>,
        tags_locked: Option<bool>,
    },
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ApplyRequest {
    pub expected_revision: u64,
    pub request_id: String,
    pub operations: Vec<Op>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct OpResult {
    pub op_index: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ApplyResponse {
    pub revision: u64,
    pub results: Vec<OpResult>,
}
#[derive(Clone, Debug, Serialize)]
pub struct TaxonomyResponse {
    pub revision: u64,
    pub items: Vec<Taxon>,
    pub next_cursor: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ItemsRequest {
    pub items: Vec<Item>,
}
#[derive(Clone, Debug, Serialize)]
pub struct ItemsResponse {
    #[serde(skip)]
    pub source_records: Vec<refact_privacy::FileRecord>,
    pub revision: u64,
    pub assignments: Vec<Assignment>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Authority {
    User,
    Model,
}
#[derive(Debug, Clone, Serialize)]
pub struct Error {
    pub code: &'static str,
    pub message: String,
}
impl Error {
    pub fn invalid(message: impl Into<String>) -> Self {
        Self {
            code: "invalid_argument",
            message: message.into(),
        }
    }
    pub fn conflict(message: impl Into<String>) -> Self {
        Self {
            code: "conflict",
            message: message.into(),
        }
    }
    pub fn denied() -> Self {
        Self {
            code: "scope_denied",
            message: "organization source unavailable or not visible".into(),
        }
    }
    pub fn storage(_: impl std::fmt::Display) -> Self {
        Self {
            code: "unavailable",
            message: "organization storage unavailable".into(),
        }
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for Error {}
impl From<rusqlite::Error> for Error {
    fn from(value: rusqlite::Error) -> Self {
        Self::storage(value)
    }
}
impl From<serde_json::Error> for Error {
    fn from(value: serde_json::Error) -> Self {
        Self::storage(value)
    }
}
