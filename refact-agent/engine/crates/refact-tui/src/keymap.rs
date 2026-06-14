use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyContext {
    Main,
    ProjectPicker,
    ModalPicker,
    Approval,
    Overlay,
    OverlaySearch,
    VimNormal,
    VimInsert,
}

impl KeyContext {
    pub fn label(self) -> &'static str {
        match self {
            Self::Main => "main",
            Self::ProjectPicker => "projects",
            Self::ModalPicker => "pickers",
            Self::Approval => "approvals",
            Self::Overlay => "overlay",
            Self::OverlaySearch => "overlay search",
            Self::VimNormal => "vim normal",
            Self::VimInsert => "vim insert",
        }
    }

    fn order(self) -> usize {
        match self {
            Self::Main => 0,
            Self::ProjectPicker => 1,
            Self::ModalPicker => 2,
            Self::Approval => 3,
            Self::Overlay => 4,
            Self::OverlaySearch => 5,
            Self::VimNormal => 6,
            Self::VimInsert => 7,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyAction {
    ShowHelp,
    ToggleEvents,
    Quit,
    NewChat,
    OpenProjects,
    OpenModels,
    OpenModes,
    OpenTranscriptOverlay,
    OpenExternalEditor,
    ToggleReasoning,
    HistorySearch,
    KillToLineEnd,
    KillToLineStart,
    Yank,
    Undo,
    Redo,
    CtrlC,
    Cancel,
    CycleToolSelection,
    ToggleSelectedTool,
    OpenSlashCommands,
    OpenFileMention,
    InsertNewline,
    Accept,
    Backspace,
    Delete,
    MoveLeft,
    MoveRight,
    MoveHome,
    MoveEnd,
    MoveUp,
    MoveDown,
    ScrollPageUp,
    ScrollPageDown,
    ToggleVimMode,
    ApprovalApproveOnce,
    ApprovalApproveForChat,
    ApprovalDeny,
    ApprovalToggleDetails,
    OverlaySearch,
    OverlayToggleCopyMode,
    OverlayYank,
    OverlayNextMatch,
    OverlayPreviousMatch,
    VimEnterInsert,
    VimAppend,
    VimOpenBelow,
    VimDeleteLine,
    VimMoveLeft,
    VimMoveDown,
    VimMoveUp,
    VimMoveRight,
    VimWordForward,
    VimWordBackward,
    VimLineStart,
    VimLineEnd,
    VimNormalMode,
}

impl KeyAction {
    pub fn name(self) -> &'static str {
        match self {
            Self::ShowHelp => "help",
            Self::ToggleEvents => "toggle-events",
            Self::Quit => "quit",
            Self::NewChat => "new-chat",
            Self::OpenProjects => "projects",
            Self::OpenModels => "models",
            Self::OpenModes => "modes",
            Self::OpenTranscriptOverlay => "transcript-overlay",
            Self::OpenExternalEditor => "external-editor",
            Self::ToggleReasoning => "toggle-reasoning",
            Self::HistorySearch => "history-search",
            Self::KillToLineEnd => "kill-to-line-end",
            Self::KillToLineStart => "kill-to-line-start",
            Self::Yank => "yank",
            Self::Undo => "undo",
            Self::Redo => "redo",
            Self::CtrlC => "ctrl-c",
            Self::Cancel => "cancel",
            Self::CycleToolSelection => "cycle-tool-selection",
            Self::ToggleSelectedTool => "toggle-selected-tool",
            Self::OpenSlashCommands => "slash-commands",
            Self::OpenFileMention => "file-mention",
            Self::InsertNewline => "newline",
            Self::Accept => "send",
            Self::Backspace => "backspace",
            Self::Delete => "delete",
            Self::MoveLeft => "move-left",
            Self::MoveRight => "move-right",
            Self::MoveHome => "move-home",
            Self::MoveEnd => "move-end",
            Self::MoveUp => "move-up",
            Self::MoveDown => "move-down",
            Self::ScrollPageUp => "page-up",
            Self::ScrollPageDown => "page-down",
            Self::ToggleVimMode => "toggle-vim",
            Self::ApprovalApproveOnce => "approval-approve-once",
            Self::ApprovalApproveForChat => "approval-approve-for-chat",
            Self::ApprovalDeny => "approval-deny",
            Self::ApprovalToggleDetails => "approval-toggle-details",
            Self::OverlaySearch => "overlay-search",
            Self::OverlayToggleCopyMode => "overlay-copy-mode",
            Self::OverlayYank => "overlay-yank",
            Self::OverlayNextMatch => "overlay-next-match",
            Self::OverlayPreviousMatch => "overlay-previous-match",
            Self::VimEnterInsert => "vim-insert",
            Self::VimAppend => "vim-append",
            Self::VimOpenBelow => "vim-open-below",
            Self::VimDeleteLine => "vim-delete-line",
            Self::VimMoveLeft => "vim-left",
            Self::VimMoveDown => "vim-down",
            Self::VimMoveUp => "vim-up",
            Self::VimMoveRight => "vim-right",
            Self::VimWordForward => "vim-word-forward",
            Self::VimWordBackward => "vim-word-backward",
            Self::VimLineStart => "vim-line-start",
            Self::VimLineEnd => "vim-line-end",
            Self::VimNormalMode => "vim-normal",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::ShowHelp => "show generated keymap help",
            Self::ToggleEvents => "toggle daemon events and workers pane",
            Self::Quit => "quit the TUI",
            Self::NewChat => "start a new chat",
            Self::OpenProjects => "open project picker",
            Self::OpenModels => "open model picker",
            Self::OpenModes => "open mode picker",
            Self::OpenTranscriptOverlay => "open transcript overlay",
            Self::OpenExternalEditor => "edit composer in external editor",
            Self::ToggleReasoning => "fold or unfold reasoning blocks",
            Self::HistorySearch => "reverse search composer history",
            Self::KillToLineEnd => "cut composer text to line end",
            Self::KillToLineStart => "cut composer text to line start",
            Self::Yank => "yank composer kill buffer",
            Self::Undo => "undo composer edit",
            Self::Redo => "redo composer edit",
            Self::CtrlC => "abort active turn or arm exit",
            Self::Cancel => "cancel, close, or abort active work",
            Self::CycleToolSelection => "select next tool card",
            Self::ToggleSelectedTool => "expand selected tool card",
            Self::OpenSlashCommands => "open slash command picker",
            Self::OpenFileMention => "open file mention picker",
            Self::InsertNewline => "insert composer newline",
            Self::Accept => "accept selection or send composer",
            Self::Backspace => "delete left or remove queued item",
            Self::Delete => "delete right or remove queued item",
            Self::MoveLeft => "move composer cursor left",
            Self::MoveRight => "move composer cursor right",
            Self::MoveHome => "move to line start",
            Self::MoveEnd => "move to line end",
            Self::MoveUp => "move up or history previous",
            Self::MoveDown => "move down or history next",
            Self::ScrollPageUp => "scroll transcript up",
            Self::ScrollPageDown => "scroll transcript down",
            Self::ToggleVimMode => "toggle composer vim mode",
            Self::ApprovalApproveOnce => "approve current tool once",
            Self::ApprovalApproveForChat => "approve matching tools for chat",
            Self::ApprovalDeny => "deny current tool request",
            Self::ApprovalToggleDetails => "toggle approval details",
            Self::OverlaySearch => "start overlay search",
            Self::OverlayToggleCopyMode => "toggle raw copy mode",
            Self::OverlayYank => "copy visible raw overlay text to terminal clipboard",
            Self::OverlayNextMatch => "jump to next search match",
            Self::OverlayPreviousMatch => "jump to previous search match",
            Self::VimEnterInsert => "enter vim insert mode",
            Self::VimAppend => "append after cursor and insert",
            Self::VimOpenBelow => "open a new line below and insert",
            Self::VimDeleteLine => "delete current line with dd",
            Self::VimMoveLeft => "vim left motion",
            Self::VimMoveDown => "vim down motion",
            Self::VimMoveUp => "vim up motion",
            Self::VimMoveRight => "vim right motion",
            Self::VimWordForward => "vim next word motion",
            Self::VimWordBackward => "vim previous word motion",
            Self::VimLineStart => "vim line start motion",
            Self::VimLineEnd => "vim line end motion",
            Self::VimNormalMode => "return to vim normal mode",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        let normalized = normalize_action_name(name);
        ALL_ACTIONS
            .iter()
            .copied()
            .find(|action| action.name() == normalized)
    }
}

const ALL_ACTIONS: &[KeyAction] = &[
    KeyAction::ShowHelp,
    KeyAction::ToggleEvents,
    KeyAction::Quit,
    KeyAction::NewChat,
    KeyAction::OpenProjects,
    KeyAction::OpenModels,
    KeyAction::OpenModes,
    KeyAction::OpenTranscriptOverlay,
    KeyAction::OpenExternalEditor,
    KeyAction::ToggleReasoning,
    KeyAction::HistorySearch,
    KeyAction::KillToLineEnd,
    KeyAction::KillToLineStart,
    KeyAction::Yank,
    KeyAction::Undo,
    KeyAction::Redo,
    KeyAction::CtrlC,
    KeyAction::Cancel,
    KeyAction::CycleToolSelection,
    KeyAction::ToggleSelectedTool,
    KeyAction::OpenSlashCommands,
    KeyAction::OpenFileMention,
    KeyAction::InsertNewline,
    KeyAction::Accept,
    KeyAction::Backspace,
    KeyAction::Delete,
    KeyAction::MoveLeft,
    KeyAction::MoveRight,
    KeyAction::MoveHome,
    KeyAction::MoveEnd,
    KeyAction::MoveUp,
    KeyAction::MoveDown,
    KeyAction::ScrollPageUp,
    KeyAction::ScrollPageDown,
    KeyAction::ToggleVimMode,
    KeyAction::ApprovalApproveOnce,
    KeyAction::ApprovalApproveForChat,
    KeyAction::ApprovalDeny,
    KeyAction::ApprovalToggleDetails,
    KeyAction::OverlaySearch,
    KeyAction::OverlayToggleCopyMode,
    KeyAction::OverlayYank,
    KeyAction::OverlayNextMatch,
    KeyAction::OverlayPreviousMatch,
    KeyAction::VimEnterInsert,
    KeyAction::VimAppend,
    KeyAction::VimOpenBelow,
    KeyAction::VimDeleteLine,
    KeyAction::VimMoveLeft,
    KeyAction::VimMoveDown,
    KeyAction::VimMoveUp,
    KeyAction::VimMoveRight,
    KeyAction::VimWordForward,
    KeyAction::VimWordBackward,
    KeyAction::VimLineStart,
    KeyAction::VimLineEnd,
    KeyAction::VimNormalMode,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyBinding {
    code: KeyCode,
    modifiers: KeyModifiers,
}

impl KeyBinding {
    pub fn parse(raw: &str) -> Result<Self, String> {
        let mut modifiers = KeyModifiers::empty();
        let raw = raw.trim();
        if raw.is_empty() {
            return Err("empty key binding".to_string());
        }
        let normalized = raw
            .trim()
            .to_ascii_lowercase()
            .replace('+', "-")
            .replace('_', "-")
            .replace(' ', "");
        let mut key_parts = Vec::new();
        for part in normalized.split('-').filter(|part| !part.is_empty()) {
            match part {
                "ctrl" | "control" => modifiers.insert(KeyModifiers::CONTROL),
                "alt" | "option" => modifiers.insert(KeyModifiers::ALT),
                "shift" => modifiers.insert(KeyModifiers::SHIFT),
                other => key_parts.push(other),
            }
        }
        let key_name = key_parts.join("-");
        let mut code = parse_key_code(&key_name)?;
        if let KeyCode::Char(ch) = code {
            if ch.is_ascii_uppercase() {
                modifiers.insert(KeyModifiers::SHIFT);
                code = KeyCode::Char(ch.to_ascii_lowercase());
            }
        }
        Ok(Self { code, modifiers })
    }

    pub fn from_event(key: KeyEvent) -> Option<Self> {
        if key.kind != KeyEventKind::Press {
            return None;
        }
        let mut modifiers =
            key.modifiers & (KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT);
        let mut code = key.code;
        if let KeyCode::Char(ch) = code {
            if modifiers.contains(KeyModifiers::CONTROL) {
                code = KeyCode::Char(ch.to_ascii_lowercase());
                if !key.modifiers.contains(KeyModifiers::SHIFT) {
                    modifiers.remove(KeyModifiers::SHIFT);
                }
            } else if ch.is_ascii_alphabetic() {
                if ch.is_ascii_uppercase() {
                    modifiers.insert(KeyModifiers::SHIFT);
                    code = KeyCode::Char(ch.to_ascii_lowercase());
                }
            } else {
                modifiers.remove(KeyModifiers::SHIFT);
            }
        }
        Some(Self { code, modifiers })
    }

    pub fn normalized(&self) -> String {
        let mut parts = Vec::new();
        if self.modifiers.contains(KeyModifiers::CONTROL) {
            parts.push("ctrl".to_string());
        }
        if self.modifiers.contains(KeyModifiers::ALT) {
            parts.push("alt".to_string());
        }
        if self.modifiers.contains(KeyModifiers::SHIFT) {
            parts.push("shift".to_string());
        }
        parts.push(code_name(self.code));
        parts.join("-")
    }

    pub fn display(&self) -> String {
        let mut parts = Vec::new();
        if self.modifiers.contains(KeyModifiers::CONTROL) {
            parts.push("Ctrl".to_string());
        }
        if self.modifiers.contains(KeyModifiers::ALT) {
            parts.push("Alt".to_string());
        }
        if self.modifiers.contains(KeyModifiers::SHIFT) {
            parts.push("Shift".to_string());
        }
        let mut label = code_label(self.code);
        if !self.modifiers.is_empty() {
            if let KeyCode::Char(ch) = self.code {
                if ch.is_ascii_alphabetic() {
                    label = ch.to_ascii_uppercase().to_string();
                }
            }
        }
        parts.push(label);
        parts.join("-")
    }
}

impl fmt::Display for KeyBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.display())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeymapEntry {
    pub context: KeyContext,
    pub action: KeyAction,
    pub bindings: Vec<KeyBinding>,
    pub description: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelpRow {
    pub context: KeyContext,
    pub action: KeyAction,
    pub bindings: String,
    pub description: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyDispatch {
    pub action: Option<KeyAction>,
    pub text: Option<char>,
}

impl KeyDispatch {
    pub fn action(action: KeyAction) -> Self {
        Self {
            action: Some(action),
            text: None,
        }
    }

    pub fn text(text: char) -> Self {
        Self {
            action: None,
            text: Some(text),
        }
    }

    pub fn unhandled() -> Self {
        Self {
            action: None,
            text: None,
        }
    }

    pub fn is_unhandled(&self) -> bool {
        self.action.is_none() && self.text.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeymapRegistry {
    entries: Vec<KeymapEntry>,
    lookup: HashMap<(KeyContext, String), KeyAction>,
    warnings: Vec<String>,
    vim_mode: bool,
}

impl Default for KeymapRegistry {
    fn default() -> Self {
        Self::from_entries(default_entries(), false)
    }
}

impl KeymapRegistry {
    pub fn from_toml_str(content: &str) -> Result<Self, String> {
        let config: KeymapFileConfig =
            toml::from_str(content).map_err(|error| error.to_string())?;
        Self::from_config(config)
    }

    pub fn from_config_file_content(content: Option<&str>) -> Result<Self, String> {
        match content {
            Some(content) if !content.trim().is_empty() => Self::from_toml_str(content),
            _ => Ok(Self::default()),
        }
    }

    pub fn default_config_path() -> Option<PathBuf> {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
            .or_else(|| {
                std::env::var_os("USERPROFILE").map(|home| PathBuf::from(home).join(".config"))
            })
            .map(|config| config.join("refact").join("tui.toml"))
    }

    fn from_config(config: KeymapFileConfig) -> Result<Self, String> {
        let mut entries = default_entries();
        if let Some(bindings) = config.bindings {
            for (name, value) in bindings {
                let Some(action) = KeyAction::from_name(&name) else {
                    return Err(format!("unknown TUI key action `{name}`"));
                };
                let parsed = value
                    .values()
                    .iter()
                    .map(|raw| KeyBinding::parse(raw))
                    .collect::<Result<Vec<_>, _>>()?;
                let mut matched = false;
                for entry in entries.iter_mut().filter(|entry| entry.action == action) {
                    entry.bindings = parsed.clone();
                    matched = true;
                }
                if !matched {
                    return Err(format!("TUI key action `{name}` has no registry entry"));
                }
            }
        }
        let mut registry = Self::from_entries(entries, config.vim_mode.unwrap_or(false));
        if let Some(vim_mode) = config.vim {
            registry.vim_mode = vim_mode;
        }
        Ok(registry)
    }

    pub fn dispatch(&self, context: KeyContext, key: KeyEvent) -> KeyDispatch {
        let Some(binding) = KeyBinding::from_event(key) else {
            return KeyDispatch::unhandled();
        };
        let normalized = binding.normalized();
        if let Some(action) = self.lookup.get(&(context, normalized)).copied() {
            return KeyDispatch::action(action);
        }
        key_text(key)
            .map(KeyDispatch::text)
            .unwrap_or_else(KeyDispatch::unhandled)
    }

    pub fn action_for(&self, context: KeyContext, key: KeyEvent) -> Option<KeyAction> {
        self.dispatch(context, key).action
    }

    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    pub fn vim_mode_enabled(&self) -> bool {
        self.vim_mode
    }

    pub fn binding_label(&self, context: KeyContext, action: KeyAction) -> Option<String> {
        self.entries
            .iter()
            .find(|entry| entry.context == context && entry.action == action)
            .and_then(|entry| {
                (!entry.bindings.is_empty()).then(|| {
                    entry
                        .bindings
                        .iter()
                        .map(KeyBinding::display)
                        .collect::<Vec<_>>()
                        .join("/")
                })
            })
    }

    pub fn help_rows(&self) -> Vec<HelpRow> {
        let mut rows = self
            .entries
            .iter()
            .filter(|entry| !entry.bindings.is_empty())
            .map(|entry| HelpRow {
                context: entry.context,
                action: entry.action,
                bindings: entry
                    .bindings
                    .iter()
                    .map(KeyBinding::display)
                    .collect::<Vec<_>>()
                    .join(", "),
                description: entry.description,
            })
            .collect::<Vec<_>>();
        rows.sort_by_key(|row| (row.context.order(), row.action.name()));
        rows
    }

    fn from_entries(entries: Vec<KeymapEntry>, vim_mode: bool) -> Self {
        let mut lookup = HashMap::new();
        let mut warnings = Vec::new();
        let mut seen_action_bindings: HashMap<(KeyContext, KeyAction, String), ()> = HashMap::new();
        for entry in &entries {
            for binding in &entry.bindings {
                let key = (entry.context, binding.normalized());
                let action_key = (entry.context, entry.action, binding.normalized());
                if seen_action_bindings.insert(action_key, ()).is_some() {
                    warnings.push(format!(
                        "duplicate binding {} for {} in {}",
                        binding.display(),
                        entry.action.name(),
                        entry.context.label()
                    ));
                }
                if let Some(previous) = lookup.insert(key.clone(), entry.action) {
                    warnings.push(format!(
                        "binding {} in {} is assigned to both {} and {}; keeping {}",
                        binding.display(),
                        entry.context.label(),
                        previous.name(),
                        entry.action.name(),
                        previous.name()
                    ));
                    lookup.insert(key, previous);
                }
            }
        }
        Self {
            entries,
            lookup,
            warnings,
            vim_mode,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VimMode {
    Normal,
    Insert,
}

impl VimMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Insert => "insert",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VimEffect {
    None,
    MoveLeft,
    MoveDown,
    MoveUp,
    MoveRight,
    WordForward,
    WordBackward,
    LineStart,
    LineEnd,
    DeleteLine,
    Append,
    OpenBelow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VimOutcome {
    pub consumed: bool,
    pub effect: VimEffect,
}

impl VimOutcome {
    fn consumed(effect: VimEffect) -> Self {
        Self {
            consumed: true,
            effect,
        }
    }

    fn unhandled() -> Self {
        Self {
            consumed: false,
            effect: VimEffect::None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VimState {
    enabled: bool,
    mode: VimMode,
    pending_delete: bool,
}

impl VimState {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            mode: VimMode::Insert,
            pending_delete: false,
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn mode(&self) -> VimMode {
        self.mode
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        self.pending_delete = false;
        self.mode = if enabled {
            VimMode::Normal
        } else {
            VimMode::Insert
        };
    }

    pub fn toggle(&mut self) -> bool {
        let enabled = !self.enabled;
        self.set_enabled(enabled);
        enabled
    }

    pub fn context(&self) -> KeyContext {
        match self.mode {
            VimMode::Normal => KeyContext::VimNormal,
            VimMode::Insert => KeyContext::VimInsert,
        }
    }

    pub fn handle_dispatch(&mut self, dispatch: KeyDispatch) -> VimOutcome {
        if !self.enabled {
            return VimOutcome::unhandled();
        }
        match self.mode {
            VimMode::Insert => self.handle_insert_dispatch(dispatch),
            VimMode::Normal => self.handle_normal_dispatch(dispatch),
        }
    }

    fn handle_insert_dispatch(&mut self, dispatch: KeyDispatch) -> VimOutcome {
        match dispatch.action {
            Some(KeyAction::VimNormalMode) => {
                self.mode = VimMode::Normal;
                self.pending_delete = false;
                VimOutcome::consumed(VimEffect::None)
            }
            _ => VimOutcome::unhandled(),
        }
    }

    fn handle_normal_dispatch(&mut self, dispatch: KeyDispatch) -> VimOutcome {
        let outcome = match dispatch.action {
            Some(KeyAction::VimEnterInsert) => {
                self.mode = VimMode::Insert;
                VimOutcome::consumed(VimEffect::None)
            }
            Some(KeyAction::VimAppend) => {
                self.mode = VimMode::Insert;
                VimOutcome::consumed(VimEffect::Append)
            }
            Some(KeyAction::VimOpenBelow) => {
                self.mode = VimMode::Insert;
                VimOutcome::consumed(VimEffect::OpenBelow)
            }
            Some(KeyAction::VimMoveLeft) => VimOutcome::consumed(VimEffect::MoveLeft),
            Some(KeyAction::VimMoveDown) => VimOutcome::consumed(VimEffect::MoveDown),
            Some(KeyAction::VimMoveUp) => VimOutcome::consumed(VimEffect::MoveUp),
            Some(KeyAction::VimMoveRight) => VimOutcome::consumed(VimEffect::MoveRight),
            Some(KeyAction::VimWordForward) => VimOutcome::consumed(VimEffect::WordForward),
            Some(KeyAction::VimWordBackward) => VimOutcome::consumed(VimEffect::WordBackward),
            Some(KeyAction::VimLineStart) => VimOutcome::consumed(VimEffect::LineStart),
            Some(KeyAction::VimLineEnd) => VimOutcome::consumed(VimEffect::LineEnd),
            Some(KeyAction::VimDeleteLine) => {
                if self.pending_delete {
                    self.pending_delete = false;
                    VimOutcome::consumed(VimEffect::DeleteLine)
                } else {
                    self.pending_delete = true;
                    return VimOutcome::consumed(VimEffect::None);
                }
            }
            _ if dispatch.text.is_some() => VimOutcome::consumed(VimEffect::None),
            _ => VimOutcome::unhandled(),
        };
        if !matches!(dispatch.action, Some(KeyAction::VimDeleteLine)) {
            self.pending_delete = false;
        }
        outcome
    }
}

#[derive(Debug, Deserialize)]
struct KeymapFileConfig {
    #[serde(default)]
    vim_mode: Option<bool>,
    #[serde(default)]
    vim: Option<bool>,
    #[serde(default)]
    bindings: Option<HashMap<String, BindingConfig>>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum BindingConfig {
    One(String),
    Many(Vec<String>),
}

impl BindingConfig {
    fn values(&self) -> Vec<String> {
        match self {
            Self::One(value) => vec![value.clone()],
            Self::Many(values) => values.clone(),
        }
    }
}

fn default_entries() -> Vec<KeymapEntry> {
    vec![
        entry(KeyContext::Main, KeyAction::ShowHelp, &["?"]),
        entry(KeyContext::Main, KeyAction::ToggleEvents, &["f2"]),
        entry(KeyContext::Main, KeyAction::Quit, &["ctrl-q"]),
        entry(KeyContext::Main, KeyAction::NewChat, &["ctrl-n"]),
        entry(KeyContext::Main, KeyAction::OpenProjects, &["ctrl-p"]),
        entry(KeyContext::Main, KeyAction::OpenModels, &["ctrl-m"]),
        entry(KeyContext::Main, KeyAction::OpenModes, &["ctrl-o"]),
        entry(
            KeyContext::Main,
            KeyAction::OpenTranscriptOverlay,
            &["ctrl-t"],
        ),
        entry(KeyContext::Main, KeyAction::OpenExternalEditor, &["ctrl-g"]),
        entry(KeyContext::Main, KeyAction::ToggleReasoning, &["alt-r"]),
        entry(KeyContext::Main, KeyAction::HistorySearch, &["ctrl-r"]),
        entry(KeyContext::Main, KeyAction::KillToLineEnd, &["ctrl-k"]),
        entry(KeyContext::Main, KeyAction::KillToLineStart, &["ctrl-u"]),
        entry(KeyContext::Main, KeyAction::Yank, &["ctrl-y"]),
        entry(KeyContext::Main, KeyAction::Undo, &["ctrl-z"]),
        entry(KeyContext::Main, KeyAction::Redo, &["ctrl-shift-z"]),
        entry(KeyContext::Main, KeyAction::CtrlC, &["ctrl-c"]),
        entry(KeyContext::Main, KeyAction::Cancel, &["esc"]),
        entry(KeyContext::Main, KeyAction::CycleToolSelection, &["tab"]),
        entry(KeyContext::Main, KeyAction::ToggleSelectedTool, &["space"]),
        entry(KeyContext::Main, KeyAction::OpenSlashCommands, &["/"]),
        entry(KeyContext::Main, KeyAction::OpenFileMention, &["@"]),
        entry(
            KeyContext::Main,
            KeyAction::InsertNewline,
            &["ctrl-j", "shift-enter", "alt-enter"],
        ),
        entry(KeyContext::Main, KeyAction::Accept, &["enter"]),
        entry(KeyContext::Main, KeyAction::Backspace, &["backspace"]),
        entry(KeyContext::Main, KeyAction::Delete, &["delete"]),
        entry(KeyContext::Main, KeyAction::MoveLeft, &["left"]),
        entry(KeyContext::Main, KeyAction::MoveRight, &["right"]),
        entry(KeyContext::Main, KeyAction::MoveHome, &["home"]),
        entry(KeyContext::Main, KeyAction::MoveEnd, &["end"]),
        entry(KeyContext::Main, KeyAction::MoveUp, &["up"]),
        entry(KeyContext::Main, KeyAction::MoveDown, &["down"]),
        entry(KeyContext::Main, KeyAction::ScrollPageUp, &["pageup"]),
        entry(KeyContext::Main, KeyAction::ScrollPageDown, &["pagedown"]),
        entry(KeyContext::Main, KeyAction::ToggleVimMode, &["ctrl-v"]),
        entry(KeyContext::ProjectPicker, KeyAction::Cancel, &["esc"]),
        entry(KeyContext::ProjectPicker, KeyAction::Accept, &["enter"]),
        entry(KeyContext::ProjectPicker, KeyAction::MoveUp, &["up"]),
        entry(KeyContext::ProjectPicker, KeyAction::MoveDown, &["down"]),
        entry(
            KeyContext::ProjectPicker,
            KeyAction::Backspace,
            &["backspace"],
        ),
        entry(KeyContext::ModalPicker, KeyAction::Cancel, &["esc"]),
        entry(
            KeyContext::ModalPicker,
            KeyAction::Accept,
            &["enter", "tab"],
        ),
        entry(KeyContext::ModalPicker, KeyAction::MoveUp, &["up"]),
        entry(KeyContext::ModalPicker, KeyAction::MoveDown, &["down"]),
        entry(
            KeyContext::ModalPicker,
            KeyAction::ToggleSelectedTool,
            &["space"],
        ),
        entry(
            KeyContext::ModalPicker,
            KeyAction::Backspace,
            &["backspace"],
        ),
        entry(KeyContext::Approval, KeyAction::ApprovalApproveOnce, &["y"]),
        entry(
            KeyContext::Approval,
            KeyAction::ApprovalApproveForChat,
            &["a"],
        ),
        entry(KeyContext::Approval, KeyAction::ApprovalDeny, &["n"]),
        entry(
            KeyContext::Approval,
            KeyAction::ApprovalToggleDetails,
            &["v"],
        ),
        entry(KeyContext::Approval, KeyAction::Cancel, &["esc"]),
        entry(KeyContext::Approval, KeyAction::MoveUp, &["up"]),
        entry(KeyContext::Approval, KeyAction::MoveDown, &["down"]),
        entry(KeyContext::Approval, KeyAction::ScrollPageUp, &["pageup"]),
        entry(
            KeyContext::Approval,
            KeyAction::ScrollPageDown,
            &["pagedown"],
        ),
        entry(KeyContext::Overlay, KeyAction::Cancel, &["esc", "q"]),
        entry(KeyContext::Overlay, KeyAction::OverlaySearch, &["/"]),
        entry(
            KeyContext::Overlay,
            KeyAction::OverlayToggleCopyMode,
            &["c"],
        ),
        entry(KeyContext::Overlay, KeyAction::OverlayYank, &["y"]),
        entry(KeyContext::Overlay, KeyAction::OverlayNextMatch, &["n"]),
        entry(
            KeyContext::Overlay,
            KeyAction::OverlayPreviousMatch,
            &["shift-n"],
        ),
        entry(KeyContext::Overlay, KeyAction::MoveDown, &["down"]),
        entry(KeyContext::Overlay, KeyAction::MoveUp, &["up"]),
        entry(
            KeyContext::Overlay,
            KeyAction::ScrollPageDown,
            &["pagedown"],
        ),
        entry(KeyContext::Overlay, KeyAction::ScrollPageUp, &["pageup"]),
        entry(KeyContext::Overlay, KeyAction::MoveHome, &["home"]),
        entry(KeyContext::Overlay, KeyAction::MoveEnd, &["end"]),
        entry(KeyContext::OverlaySearch, KeyAction::Cancel, &["esc"]),
        entry(KeyContext::OverlaySearch, KeyAction::Accept, &["enter"]),
        entry(
            KeyContext::OverlaySearch,
            KeyAction::Backspace,
            &["backspace"],
        ),
        entry(KeyContext::VimNormal, KeyAction::VimEnterInsert, &["i"]),
        entry(KeyContext::VimNormal, KeyAction::VimAppend, &["a"]),
        entry(KeyContext::VimNormal, KeyAction::VimOpenBelow, &["o"]),
        entry(KeyContext::VimNormal, KeyAction::VimDeleteLine, &["d"]),
        entry(KeyContext::VimNormal, KeyAction::VimMoveLeft, &["h"]),
        entry(KeyContext::VimNormal, KeyAction::VimMoveDown, &["j"]),
        entry(KeyContext::VimNormal, KeyAction::VimMoveUp, &["k"]),
        entry(KeyContext::VimNormal, KeyAction::VimMoveRight, &["l"]),
        entry(KeyContext::VimNormal, KeyAction::VimWordForward, &["w"]),
        entry(KeyContext::VimNormal, KeyAction::VimWordBackward, &["b"]),
        entry(KeyContext::VimNormal, KeyAction::VimLineStart, &["0"]),
        entry(KeyContext::VimNormal, KeyAction::VimLineEnd, &["$"]),
        entry(KeyContext::VimInsert, KeyAction::VimNormalMode, &["esc"]),
    ]
}

fn entry(context: KeyContext, action: KeyAction, keys: &[&str]) -> KeymapEntry {
    KeymapEntry {
        context,
        action,
        bindings: keys
            .iter()
            .map(|key| KeyBinding::parse(key).expect("default key binding must parse"))
            .collect(),
        description: action.description(),
    }
}

fn parse_key_code(key: &str) -> Result<KeyCode, String> {
    let code = match key {
        "enter" | "return" => KeyCode::Enter,
        "esc" | "escape" => KeyCode::Esc,
        "tab" => KeyCode::Tab,
        "backspace" | "bs" => KeyCode::Backspace,
        "delete" | "del" => KeyCode::Delete,
        "insert" | "ins" => KeyCode::Insert,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" | "page-up" | "pgup" => KeyCode::PageUp,
        "pagedown" | "page-down" | "pgdown" | "pgdn" => KeyCode::PageDown,
        "space" => KeyCode::Char(' '),
        value if value.starts_with('f') && value.len() > 1 => {
            let number = value[1..]
                .parse::<u8>()
                .map_err(|_| format!("invalid function key `{key}`"))?;
            KeyCode::F(number)
        }
        value if value.chars().count() == 1 => {
            let ch = value.chars().next().expect("count checked");
            KeyCode::Char(ch)
        }
        _ => return Err(format!("unknown key binding `{key}`")),
    };
    Ok(code)
}

fn code_name(code: KeyCode) -> String {
    match code {
        KeyCode::Enter => "enter".to_string(),
        KeyCode::Esc => "esc".to_string(),
        KeyCode::Tab => "tab".to_string(),
        KeyCode::Backspace => "backspace".to_string(),
        KeyCode::Delete => "delete".to_string(),
        KeyCode::Insert => "insert".to_string(),
        KeyCode::Left => "left".to_string(),
        KeyCode::Right => "right".to_string(),
        KeyCode::Up => "up".to_string(),
        KeyCode::Down => "down".to_string(),
        KeyCode::Home => "home".to_string(),
        KeyCode::End => "end".to_string(),
        KeyCode::PageUp => "pageup".to_string(),
        KeyCode::PageDown => "pagedown".to_string(),
        KeyCode::F(number) => format!("f{number}"),
        KeyCode::Char(' ') => "space".to_string(),
        KeyCode::Char(ch) => ch.to_string(),
        other => format!("{other:?}").to_ascii_lowercase(),
    }
}

fn code_label(code: KeyCode) -> String {
    match code {
        KeyCode::Enter => "Enter".to_string(),
        KeyCode::Esc => "Esc".to_string(),
        KeyCode::Tab => "Tab".to_string(),
        KeyCode::Backspace => "Backspace".to_string(),
        KeyCode::Delete => "Delete".to_string(),
        KeyCode::Insert => "Insert".to_string(),
        KeyCode::Left => "Left".to_string(),
        KeyCode::Right => "Right".to_string(),
        KeyCode::Up => "Up".to_string(),
        KeyCode::Down => "Down".to_string(),
        KeyCode::Home => "Home".to_string(),
        KeyCode::End => "End".to_string(),
        KeyCode::PageUp => "PageUp".to_string(),
        KeyCode::PageDown => "PageDown".to_string(),
        KeyCode::F(number) => format!("F{number}"),
        KeyCode::Char(' ') => "Space".to_string(),
        KeyCode::Char(ch) => ch.to_string(),
        other => format!("{other:?}"),
    }
}

fn key_text(key: KeyEvent) -> Option<char> {
    if key.kind != KeyEventKind::Press {
        return None;
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) || key.modifiers.contains(KeyModifiers::ALT) {
        return None;
    }
    match key.code {
        KeyCode::Char(ch) => Some(ch),
        _ => None,
    }
}

fn normalize_action_name(name: &str) -> String {
    name.trim()
        .to_ascii_lowercase()
        .replace('_', "-")
        .replace('.', "-")
        .replace(' ', "-")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn config_parse_lookup_dispatch_round_trip() {
        let registry = KeymapRegistry::from_toml_str(
            r#"
vim_mode = true

[bindings]
send = "ctrl-s"
newline = ["ctrl-j"]
"#,
        )
        .unwrap();

        assert!(registry.vim_mode_enabled());
        assert_eq!(
            registry.action_for(
                KeyContext::Main,
                key(KeyCode::Char('s'), KeyModifiers::CONTROL)
            ),
            Some(KeyAction::Accept)
        );
        assert_eq!(
            registry.dispatch(
                KeyContext::Main,
                key(KeyCode::Char('x'), KeyModifiers::empty())
            ),
            KeyDispatch::text('x')
        );
    }

    #[test]
    fn duplicate_binding_warns_and_keeps_first_action() {
        let registry = KeymapRegistry::from_toml_str(
            r#"
[bindings]
send = "enter"
newline = "enter"
"#,
        )
        .unwrap();

        assert!(registry
            .warnings()
            .iter()
            .any(|warning| warning.contains("assigned to both")));
        assert_eq!(
            registry.action_for(KeyContext::Main, key(KeyCode::Enter, KeyModifiers::empty())),
            Some(KeyAction::InsertNewline)
        );
    }

    #[test]
    fn vim_state_machine_enters_insert_and_deletes_line_with_dd() {
        let registry = KeymapRegistry::default();
        let mut vim = VimState::new(true);
        assert_eq!(vim.mode(), VimMode::Insert);
        vim.set_enabled(true);
        assert_eq!(vim.mode(), VimMode::Normal);

        let insert = registry.dispatch(
            vim.context(),
            key(KeyCode::Char('i'), KeyModifiers::empty()),
        );
        assert_eq!(vim.handle_dispatch(insert).effect, VimEffect::None);
        assert_eq!(vim.mode(), VimMode::Insert);

        let normal = registry.dispatch(vim.context(), key(KeyCode::Esc, KeyModifiers::empty()));
        assert!(vim.handle_dispatch(normal).consumed);
        assert_eq!(vim.mode(), VimMode::Normal);

        let d = registry.dispatch(
            vim.context(),
            key(KeyCode::Char('d'), KeyModifiers::empty()),
        );
        assert_eq!(vim.handle_dispatch(d.clone()).effect, VimEffect::None);
        assert_eq!(vim.handle_dispatch(d).effect, VimEffect::DeleteLine);
    }

    #[test]
    fn default_keymap_has_no_conflicts_and_lists_composer_power_actions() {
        let registry = KeymapRegistry::default();
        assert!(registry.warnings().is_empty());
        assert_eq!(
            registry.action_for(
                KeyContext::Main,
                key(KeyCode::Char('r'), KeyModifiers::CONTROL)
            ),
            Some(KeyAction::HistorySearch)
        );
        assert_eq!(
            registry.action_for(KeyContext::Main, key(KeyCode::Char('r'), KeyModifiers::ALT)),
            Some(KeyAction::ToggleReasoning)
        );
        let rows = registry.help_rows();
        assert!(rows
            .iter()
            .any(|row| row.action == KeyAction::HistorySearch && row.bindings.contains("Ctrl-R")));
        assert!(rows
            .iter()
            .any(|row| row.action == KeyAction::KillToLineEnd && row.bindings.contains("Ctrl-K")));
        assert!(rows
            .iter()
            .any(|row| row.action == KeyAction::Redo && row.bindings.contains("Ctrl-Shift-Z")));
    }
}
