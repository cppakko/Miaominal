use crate::theme as material_theme;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiProviderKind {
    Anthropic,
    ChatGpt,
    Cohere,
    Copilot,
    DeepSeek,
    Gemini,
    HuggingFace,
    Mistral,
    OpenAi,
    OpenRouter,
    Together,
    Xai,
    Custom,
}

impl AiProviderKind {
    pub const fn all() -> &'static [Self] {
        &[
            Self::OpenAi,
            Self::Anthropic,
            Self::DeepSeek,
            Self::Gemini,
            Self::OpenRouter,
            Self::Mistral,
            Self::Cohere,
            Self::Together,
            Self::Xai,
            Self::HuggingFace,
            Self::ChatGpt,
            Self::Copilot,
            Self::Custom,
        ]
    }

    pub const fn default_model(self) -> &'static str {
        match self {
            Self::Anthropic => "claude-3-5-sonnet-latest",
            Self::ChatGpt => "gpt-4o",
            Self::Cohere => "command-r-plus",
            Self::Copilot => "gpt-4o",
            Self::DeepSeek => "deepseek-v4-pro",
            Self::Gemini => "gemini-1.5-pro",
            Self::HuggingFace => "meta-llama/Meta-Llama-3.1-8B-Instruct",
            Self::Mistral => "mistral-large-latest",
            Self::OpenAi => "gpt-4o",
            Self::OpenRouter => "openai/gpt-4o",
            Self::Together => "meta-llama/Meta-Llama-3.1-70B-Instruct-Turbo",
            Self::Xai => "grok-2-latest",
            Self::Custom => "",
        }
    }

    pub const fn requires_api_key(self) -> bool {
        !matches!(self, Self::Custom)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiProviderConfig {
    pub id: String,
    pub name: String,
    pub kind: AiProviderKind,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub api_key_env: String,
    #[serde(default)]
    pub has_api_key: bool,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Optional context window size for the model (e.g. 200000 for 200K).
    /// When set, the chat UI shows estimated context usage as a percentage.
    #[serde(default)]
    pub context_window: Option<u64>,

    /// Temperature (0.0–2.0). When None, the provider default is used.
    #[serde(default)]
    pub temperature: Option<f64>,

    /// Maximum output tokens. When None, the provider default is used.
    #[serde(default)]
    pub max_tokens: Option<u64>,
}

impl PartialEq for AiProviderConfig {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.name == other.name
            && self.kind == other.kind
            && self.model == other.model
            && self.base_url == other.base_url
            && self.api_key_env == other.api_key_env
            && self.has_api_key == other.has_api_key
            && self.enabled == other.enabled
            && self.context_window == other.context_window
            && self.temperature.map(f64::to_bits) == other.temperature.map(f64::to_bits)
            && self.max_tokens == other.max_tokens
    }
}

impl Eq for AiProviderConfig {}

impl AiProviderConfig {
    pub fn new(kind: AiProviderKind) -> Self {
        let mut provider = Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: ai_provider_kind_label(kind).to_string(),
            kind,
            model: kind.default_model().to_string(),
            base_url: String::new(),
            api_key_env: String::new(),
            has_api_key: false,
            enabled: true,
            context_window: None,
            temperature: None,
            max_tokens: None,
        };
        provider.sanitize();
        provider
    }

    pub fn sanitize(&mut self) {
        self.id = self.id.trim().to_string();
        if self.id.is_empty() {
            self.id = uuid::Uuid::new_v4().to_string();
        }
        self.name = self.name.trim().to_string();
        if self.name.is_empty() {
            self.name = ai_provider_kind_label(self.kind).to_string();
        }
        self.model = self.model.trim().to_string();
        if self.model.is_empty() {
            self.model = self.kind.default_model().to_string();
        }
        self.base_url = self.base_url.trim().trim_end_matches('/').to_string();
        self.api_key_env = self.api_key_env.trim().to_string();
    }
}

pub fn ai_provider_kind_label(kind: AiProviderKind) -> &'static str {
    match kind {
        AiProviderKind::Anthropic => "Anthropic",
        AiProviderKind::ChatGpt => "ChatGPT",
        AiProviderKind::Cohere => "Cohere",
        AiProviderKind::Copilot => "Copilot",
        AiProviderKind::DeepSeek => "DeepSeek",
        AiProviderKind::Gemini => "Gemini",
        AiProviderKind::HuggingFace => "Hugging Face",
        AiProviderKind::Mistral => "Mistral",
        AiProviderKind::OpenAi => "OpenAI",
        AiProviderKind::OpenRouter => "OpenRouter",
        AiProviderKind::Together => "Together AI",
        AiProviderKind::Xai => "xAI",
        AiProviderKind::Custom => "Custom",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebSearchProviderKind {
    Tavily,
    Exa,
    Bocha,
    Zhipu,
    SearXng,
}

impl WebSearchProviderKind {
    pub const fn all() -> &'static [Self] {
        &[
            Self::Tavily,
            Self::Exa,
            Self::Bocha,
            Self::Zhipu,
            Self::SearXng,
        ]
    }

    pub const fn requires_api_key(self) -> bool {
        !matches!(self, Self::SearXng)
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Tavily => "Tavily",
            Self::Exa => "Exa",
            Self::Bocha => "Bocha",
            Self::Zhipu => "Zhipu",
            Self::SearXng => "SearXNG",
        }
    }
}

fn default_web_search_provider_kind() -> WebSearchProviderKind {
    WebSearchProviderKind::Tavily
}

fn default_web_search_max_results() -> u32 {
    DEFAULT_WEB_SEARCH_MAX_RESULTS
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebSearchConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_web_search_provider_kind")]
    pub kind: WebSearchProviderKind,
    #[serde(default)]
    pub api_key_env: String,
    #[serde(default)]
    pub has_api_key: bool,
    #[serde(default)]
    pub endpoint: String,
    #[serde(default = "default_web_search_max_results")]
    pub max_results: u32,
}

impl Default for WebSearchConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            kind: default_web_search_provider_kind(),
            api_key_env: String::new(),
            has_api_key: false,
            endpoint: String::new(),
            max_results: DEFAULT_WEB_SEARCH_MAX_RESULTS,
        }
    }
}

impl WebSearchConfig {
    pub fn sanitize(&mut self) {
        self.api_key_env = self.api_key_env.trim().to_string();
        self.endpoint = self.endpoint.trim().trim_end_matches('/').to_string();
        self.max_results = self
            .max_results
            .clamp(WEB_SEARCH_MAX_RESULTS_MIN, WEB_SEARCH_MAX_RESULTS_MAX);
        if !self.kind.requires_api_key() && self.api_key_env.is_empty() {
            self.has_api_key = false;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyBinding {
    #[serde(default)]
    pub ctrl: bool,
    #[serde(default)]
    pub shift: bool,
    #[serde(default)]
    pub alt: bool,
    pub key: String,
}

impl KeyBinding {
    pub fn new(ctrl: bool, shift: bool, alt: bool, key: impl Into<String>) -> Self {
        Self {
            ctrl,
            shift,
            alt,
            key: key.into(),
        }
    }

    pub fn display(&self) -> String {
        let mut result = String::new();
        if self.ctrl {
            result.push_str("Ctrl+");
        }
        if self.shift {
            result.push_str("Shift+");
        }
        if self.alt {
            result.push_str("Alt+");
        }
        result.push_str(&self.key.to_uppercase());
        result
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalKeyBindings {
    #[serde(default = "default_binding_copy")]
    pub copy: KeyBinding,
    #[serde(default = "default_binding_paste")]
    pub paste: KeyBinding,
    #[serde(default = "default_binding_search")]
    pub search: KeyBinding,
    #[serde(default = "default_binding_split_right")]
    pub split_right: KeyBinding,
    #[serde(default = "default_binding_split_down")]
    pub split_down: KeyBinding,
    #[serde(default = "default_binding_close_pane")]
    pub close_pane: KeyBinding,
    #[serde(default = "default_binding_next_tab")]
    pub next_tab: KeyBinding,
    #[serde(default = "default_binding_close_tab")]
    pub close_tab: KeyBinding,
    #[serde(default = "default_binding_reopen_tab")]
    pub reopen_tab: KeyBinding,
    #[serde(default = "default_binding_open_settings")]
    pub open_settings: KeyBinding,
}

impl Default for TerminalKeyBindings {
    fn default() -> Self {
        Self {
            copy: default_binding_copy(),
            paste: default_binding_paste(),
            search: default_binding_search(),
            split_right: default_binding_split_right(),
            split_down: default_binding_split_down(),
            close_pane: default_binding_close_pane(),
            next_tab: default_binding_next_tab(),
            close_tab: default_binding_close_tab(),
            reopen_tab: default_binding_reopen_tab(),
            open_settings: default_binding_open_settings(),
        }
    }
}

fn default_binding_copy() -> KeyBinding {
    KeyBinding::new(true, true, false, "c")
}

fn default_binding_paste() -> KeyBinding {
    KeyBinding::new(true, true, false, "v")
}

fn default_binding_search() -> KeyBinding {
    KeyBinding::new(true, true, false, "f")
}

fn default_binding_split_right() -> KeyBinding {
    KeyBinding::new(true, true, false, "\\")
}

fn default_binding_split_down() -> KeyBinding {
    KeyBinding::new(true, true, false, "-")
}

fn default_binding_close_pane() -> KeyBinding {
    KeyBinding::new(true, true, false, "w")
}

fn default_binding_next_tab() -> KeyBinding {
    KeyBinding::new(true, false, false, "tab")
}

fn default_binding_close_tab() -> KeyBinding {
    KeyBinding::new(true, false, false, "w")
}

fn default_binding_reopen_tab() -> KeyBinding {
    KeyBinding::new(true, true, false, "t")
}

fn default_binding_open_settings() -> KeyBinding {
    KeyBinding::new(true, false, false, ",")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalRightClickBehavior {
    ContextMenu,
    CopySelectionOrPaste,
}

impl TerminalRightClickBehavior {
    pub const fn uses_context_menu(self) -> bool {
        matches!(self, Self::ContextMenu)
    }
}

fn default_terminal_right_click_behavior() -> TerminalRightClickBehavior {
    TerminalRightClickBehavior::ContextMenu
}

fn default_terminal_shift_right_click_context_menu() -> bool {
    true
}

fn default_auto_collect_session_monitoring() -> bool {
    false
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LastTabCloseBehavior {
    ExitApplication,
    OpenNewHomeTab,
}

impl LastTabCloseBehavior {
    pub fn all() -> &'static [Self] {
        &[Self::ExitApplication, Self::OpenNewHomeTab]
    }
}

fn default_last_tab_close_behavior() -> LastTabCloseBehavior {
    LastTabCloseBehavior::ExitApplication
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MonitorHistoryDuration {
    OneMinute,
    FiveMinutes,
    TenMinutes,
    ThirtyMinutes,
}

impl MonitorHistoryDuration {
    pub fn history_limit(self) -> usize {
        match self {
            Self::OneMinute => 30,
            Self::FiveMinutes => 150,
            Self::TenMinutes => 300,
            Self::ThirtyMinutes => 900,
        }
    }

    pub fn all() -> &'static [Self] {
        &[
            Self::OneMinute,
            Self::FiveMinutes,
            Self::TenMinutes,
            Self::ThirtyMinutes,
        ]
    }
}

fn default_monitor_history_duration() -> MonitorHistoryDuration {
    MonitorHistoryDuration::FiveMinutes
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalVaultAutoLockDuration {
    Off,
    FiveMinutes,
    FifteenMinutes,
    OneHour,
    OneDay,
}

impl LocalVaultAutoLockDuration {
    pub fn duration(self) -> Option<std::time::Duration> {
        match self {
            Self::Off => None,
            Self::FiveMinutes => Some(std::time::Duration::from_secs(5 * 60)),
            Self::FifteenMinutes => Some(std::time::Duration::from_secs(15 * 60)),
            Self::OneHour => Some(std::time::Duration::from_secs(60 * 60)),
            Self::OneDay => Some(std::time::Duration::from_secs(24 * 60 * 60)),
        }
    }

    pub fn all() -> &'static [Self] {
        &[
            Self::Off,
            Self::FiveMinutes,
            Self::FifteenMinutes,
            Self::OneHour,
            Self::OneDay,
        ]
    }
}

fn default_local_vault_auto_lock_duration() -> LocalVaultAutoLockDuration {
    LocalVaultAutoLockDuration::Off
}

fn default_completed_onboarding_version() -> u32 {
    0
}

fn default_local_vault_enabled() -> bool {
    false
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AppLanguage {
    #[serde(rename = "en")]
    English,
    #[serde(rename = "zh-CN", alias = "zh_cn", alias = "zh-cn")]
    SimplifiedChinese,
}

impl AppLanguage {
    pub const fn native_name(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::SimplifiedChinese => "简体中文",
        }
    }

    pub const fn supported_languages() -> [Self; 2] {
        [Self::English, Self::SimplifiedChinese]
    }

    pub fn from_locale_code(locale_code: &str) -> Self {
        let normalized_locale = locale_code.trim().replace('_', "-").to_ascii_lowercase();
        if normalized_locale.starts_with("zh") {
            Self::SimplifiedChinese
        } else {
            Self::English
        }
    }
}

fn default_language() -> AppLanguage {
    AppLanguage::English
}

pub const CURRENT_ONBOARDING_VERSION: u32 = 1;
pub const FONT_SIZE_MIN: f32 = 8.0;
pub const FONT_SIZE_MAX: f32 = 32.0;
pub const LINE_HEIGHT_MIN: f32 = 12.0;
pub const LINE_HEIGHT_MAX: f32 = 40.0;
pub const STEP: f32 = 0.5;
pub const RECENT_CONNECTIONS_COUNT_MIN: u8 = 0;
pub const RECENT_CONNECTIONS_COUNT_MAX: u8 = 20;
pub const WEB_SEARCH_MAX_RESULTS_MIN: u32 = 1;
pub const WEB_SEARCH_MAX_RESULTS_MAX: u32 = 50;
pub(crate) const DEFAULT_RECENT_CONNECTIONS_COUNT: u8 = 5;
pub(crate) const DEFAULT_WEB_SEARCH_MAX_RESULTS: u32 = 10;
const SFTP_BROWSER_COLUMN_COUNT: usize = 6;
pub(crate) const DEFAULT_FONT_SIZE: f32 = 14.0;
pub(crate) const DEFAULT_LINE_HEIGHT: f32 = 18.0;
pub(crate) const DEFAULT_CELL_WIDTH: f32 = 8.0;

#[cfg(target_os = "windows")]
pub const PLATFORM_DEFAULT_FONT: &str = "Consolas";

#[cfg(target_os = "macos")]
pub const PLATFORM_DEFAULT_FONT: &str = "Menlo";

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub const PLATFORM_DEFAULT_FONT: &str = "DejaVu Sans Mono";

static AVAILABLE_FONT_FAMILIES: OnceLock<Vec<String>> = OnceLock::new();

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
static LINUX_DEFAULT_FONT_FAMILY: OnceLock<String> = OnceLock::new();

#[cfg(target_os = "windows")]
pub fn default_font_fallbacks() -> Vec<String> {
    vec!["Microsoft YaHei".to_string(), "SimSun".to_string()]
}

#[cfg(target_os = "macos")]
pub fn default_font_fallbacks() -> Vec<String> {
    vec!["PingFang SC".to_string(), "Hiragino Sans GB".to_string()]
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn default_font_fallbacks() -> Vec<String> {
    vec![
        "Noto Sans CJK SC".to_string(),
        "WenQuanYi Micro Hei".to_string(),
    ]
}

fn default_sftp_browser_hidden_columns() -> Vec<usize> {
    vec![4, 5]
}

fn sanitize_sftp_browser_hidden_columns(columns: &mut Vec<usize>) {
    columns.retain(|column| *column < SFTP_BROWSER_COLUMN_COUNT);
    columns.sort_unstable();
    columns.dedup();

    if columns.len() >= SFTP_BROWSER_COLUMN_COUNT {
        *columns = default_sftp_browser_hidden_columns();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeId {
    Light,
    #[serde(alias = "one_dark")]
    Dark,
}

impl ThemeId {
    pub const fn is_dark(self) -> bool {
        matches!(self, ThemeId::Dark)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    #[serde(default = "default_language")]
    pub language: AppLanguage,
    #[serde(default = "default_font_family")]
    pub font_family: String,
    #[serde(default = "default_font_fallbacks")]
    pub font_fallbacks: Vec<String>,
    #[serde(default = "default_font_size")]
    pub font_size: f32,
    #[serde(default = "default_line_height")]
    pub line_height: f32,
    #[serde(default = "default_theme")]
    pub theme_id: ThemeId,
    #[serde(default = "default_seed_color")]
    pub seed_color: String,
    #[serde(default = "default_recent_connections_count")]
    pub recent_connections_count: u8,
    #[serde(default)]
    pub key_bindings: TerminalKeyBindings,
    #[serde(default = "default_terminal_right_click_behavior")]
    pub terminal_right_click_behavior: TerminalRightClickBehavior,
    #[serde(default = "default_terminal_shift_right_click_context_menu")]
    pub terminal_shift_right_click_context_menu: bool,
    #[serde(default = "default_auto_collect_session_monitoring")]
    pub auto_collect_session_monitoring: bool,
    #[serde(default = "default_last_tab_close_behavior")]
    pub last_tab_close_behavior: LastTabCloseBehavior,
    #[serde(default = "default_monitor_history_duration")]
    pub monitor_history_duration: MonitorHistoryDuration,
    #[serde(default = "default_sftp_browser_hidden_columns")]
    pub local_sftp_hidden_columns: Vec<usize>,
    #[serde(default = "default_sftp_browser_hidden_columns")]
    pub remote_sftp_hidden_columns: Vec<usize>,
    #[serde(default = "default_completed_onboarding_version")]
    pub completed_onboarding_version: u32,
    #[serde(default = "default_local_vault_enabled")]
    pub local_vault_enabled: bool,
    #[serde(default = "default_local_vault_auto_lock_duration")]
    pub local_vault_auto_lock_duration: LocalVaultAutoLockDuration,
    #[serde(default)]
    pub ai_providers: Vec<AiProviderConfig>,
    #[serde(default)]
    pub selected_ai_provider_id: Option<String>,
    #[serde(default)]
    pub web_search: WebSearchConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SyncedSettings {
    #[serde(default = "default_recent_connections_count")]
    pub recent_connections_count: u8,
    #[serde(default)]
    pub key_bindings: TerminalKeyBindings,
    #[serde(default = "default_terminal_right_click_behavior")]
    pub terminal_right_click_behavior: TerminalRightClickBehavior,
    #[serde(default = "default_terminal_shift_right_click_context_menu")]
    pub terminal_shift_right_click_context_menu: bool,
    #[serde(default = "default_auto_collect_session_monitoring")]
    pub auto_collect_session_monitoring: bool,
    #[serde(default = "default_last_tab_close_behavior")]
    pub last_tab_close_behavior: LastTabCloseBehavior,
    #[serde(default = "default_monitor_history_duration")]
    pub monitor_history_duration: MonitorHistoryDuration,
    #[serde(default = "default_sftp_browser_hidden_columns")]
    pub local_sftp_hidden_columns: Vec<usize>,
    #[serde(default = "default_sftp_browser_hidden_columns")]
    pub remote_sftp_hidden_columns: Vec<usize>,
    #[serde(default = "default_completed_onboarding_version")]
    pub completed_onboarding_version: u32,
    #[serde(default = "default_local_vault_auto_lock_duration")]
    pub local_vault_auto_lock_duration: LocalVaultAutoLockDuration,
    #[serde(default)]
    pub ai_providers: Vec<AiProviderConfig>,
    #[serde(default)]
    pub web_search: WebSearchConfig,
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
pub fn default_font_family() -> String {
    PLATFORM_DEFAULT_FONT.to_string()
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn default_font_family() -> String {
    LINUX_DEFAULT_FONT_FAMILY
        .get_or_init(resolve_linux_default_font_family)
        .clone()
}

pub fn available_font_families() -> Vec<String> {
    AVAILABLE_FONT_FAMILIES
        .get_or_init(discover_system_font_families)
        .clone()
}

fn discover_system_font_families() -> Vec<String> {
    let mut database = fontdb::Database::new();
    database.load_system_fonts();

    let mut families: Vec<String> = database
        .faces()
        .filter_map(|face| face.families.first())
        .map(|(family, _)| family.trim().to_string())
        .filter(|family| !family.is_empty())
        .collect();
    sort_and_dedup_font_families(&mut families);

    if families.is_empty() {
        families.push(PLATFORM_DEFAULT_FONT.to_string());
    }

    families
}

fn sort_and_dedup_font_families(families: &mut Vec<String>) {
    families.sort_by_cached_key(|family| family.to_ascii_lowercase());
    families.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn resolve_linux_default_font_family() -> String {
    let mut database = fontdb::Database::new();
    database.load_system_fonts();

    let families = [fontdb::Family::Monospace];
    let id = database.query(&fontdb::Query {
        families: &families,
        ..Default::default()
    });

    id.and_then(|id| database.face(id))
        .and_then(|face| face.families.first())
        .map(|(family, _)| family.clone())
        .unwrap_or_else(|| PLATFORM_DEFAULT_FONT.to_string())
}

fn default_font_size() -> f32 {
    DEFAULT_FONT_SIZE
}

fn default_line_height() -> f32 {
    DEFAULT_LINE_HEIGHT
}

fn default_theme() -> ThemeId {
    ThemeId::Light
}

fn default_seed_color() -> String {
    material_theme::DEFAULT_SEED_COLOR.to_string()
}

fn default_recent_connections_count() -> u8 {
    DEFAULT_RECENT_CONNECTIONS_COUNT
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            language: default_language(),
            font_family: default_font_family(),
            font_fallbacks: default_font_fallbacks(),
            font_size: default_font_size(),
            line_height: default_line_height(),
            theme_id: default_theme(),
            seed_color: default_seed_color(),
            recent_connections_count: default_recent_connections_count(),
            key_bindings: TerminalKeyBindings::default(),
            terminal_right_click_behavior: default_terminal_right_click_behavior(),
            terminal_shift_right_click_context_menu:
                default_terminal_shift_right_click_context_menu(),
            auto_collect_session_monitoring: default_auto_collect_session_monitoring(),
            last_tab_close_behavior: default_last_tab_close_behavior(),
            monitor_history_duration: default_monitor_history_duration(),
            local_sftp_hidden_columns: default_sftp_browser_hidden_columns(),
            remote_sftp_hidden_columns: default_sftp_browser_hidden_columns(),
            completed_onboarding_version: default_completed_onboarding_version(),
            local_vault_enabled: default_local_vault_enabled(),
            local_vault_auto_lock_duration: default_local_vault_auto_lock_duration(),
            ai_providers: Vec::new(),
            selected_ai_provider_id: None,
            web_search: WebSearchConfig::default(),
        }
    }
}

impl AppSettings {
    pub fn sanitize(&mut self) {
        if self.font_family.trim().is_empty()
            || should_reset_legacy_platform_font(&self.font_family)
        {
            self.font_family = default_font_family();
        }
        self.font_size = self.font_size.clamp(FONT_SIZE_MIN, FONT_SIZE_MAX);
        self.line_height = self.line_height.clamp(LINE_HEIGHT_MIN, LINE_HEIGHT_MAX);
        self.seed_color = material_theme::normalize_seed_color(&self.seed_color)
            .unwrap_or_else(default_seed_color);
        self.recent_connections_count = self
            .recent_connections_count
            .clamp(RECENT_CONNECTIONS_COUNT_MIN, RECENT_CONNECTIONS_COUNT_MAX);
        sanitize_sftp_browser_hidden_columns(&mut self.local_sftp_hidden_columns);
        sanitize_sftp_browser_hidden_columns(&mut self.remote_sftp_hidden_columns);
        sanitize_ai_providers(&mut self.ai_providers);
        self.web_search.sanitize();
    }

    pub fn effective_font_family(&self) -> &str {
        if self.font_family.trim().is_empty() {
            PLATFORM_DEFAULT_FONT
        } else {
            self.font_family.as_str()
        }
    }

    pub fn effective_font_fallbacks(&self) -> &[String] {
        &self.font_fallbacks
    }

    pub fn should_show_onboarding(&self) -> bool {
        self.completed_onboarding_version < CURRENT_ONBOARDING_VERSION
    }

    pub fn mark_current_onboarding_completed(&mut self) {
        self.completed_onboarding_version = CURRENT_ONBOARDING_VERSION;
    }

    pub fn synced_settings(&self) -> SyncedSettings {
        SyncedSettings::from(self)
    }

    pub fn apply_synced_settings(&mut self, synced: &SyncedSettings) {
        self.recent_connections_count = synced.recent_connections_count;
        self.key_bindings = synced.key_bindings.clone();
        self.terminal_right_click_behavior = synced.terminal_right_click_behavior;
        self.terminal_shift_right_click_context_menu =
            synced.terminal_shift_right_click_context_menu;
        self.auto_collect_session_monitoring = synced.auto_collect_session_monitoring;
        self.last_tab_close_behavior = synced.last_tab_close_behavior;
        self.monitor_history_duration = synced.monitor_history_duration;
        self.local_sftp_hidden_columns = synced.local_sftp_hidden_columns.clone();
        self.remote_sftp_hidden_columns = synced.remote_sftp_hidden_columns.clone();
        self.completed_onboarding_version = synced.completed_onboarding_version;
        self.local_vault_auto_lock_duration = synced.local_vault_auto_lock_duration;
        self.ai_providers = synced.ai_providers.clone();
        sanitize_ai_providers(&mut self.ai_providers);
        self.web_search = synced.web_search.clone();
        self.web_search.sanitize();
    }
}

impl From<&AppSettings> for SyncedSettings {
    fn from(settings: &AppSettings) -> Self {
        Self {
            recent_connections_count: settings.recent_connections_count,
            key_bindings: settings.key_bindings.clone(),
            terminal_right_click_behavior: settings.terminal_right_click_behavior,
            terminal_shift_right_click_context_menu: settings
                .terminal_shift_right_click_context_menu,
            auto_collect_session_monitoring: settings.auto_collect_session_monitoring,
            last_tab_close_behavior: settings.last_tab_close_behavior,
            monitor_history_duration: settings.monitor_history_duration,
            local_sftp_hidden_columns: settings.local_sftp_hidden_columns.clone(),
            remote_sftp_hidden_columns: settings.remote_sftp_hidden_columns.clone(),
            completed_onboarding_version: settings.completed_onboarding_version,
            local_vault_auto_lock_duration: settings.local_vault_auto_lock_duration,
            ai_providers: settings.ai_providers.clone(),
            web_search: settings.web_search.clone(),
        }
    }
}

fn sanitize_ai_providers(providers: &mut Vec<AiProviderConfig>) {
    for provider in providers.iter_mut() {
        provider.sanitize();
    }

    providers.retain(|provider| !provider.id.trim().is_empty());
    providers.sort_by_cached_key(|provider| provider.name.to_ascii_lowercase());

    let mut seen = std::collections::HashSet::new();
    for provider in providers.iter_mut() {
        while !seen.insert(provider.id.clone()) {
            provider.id = uuid::Uuid::new_v4().to_string();
        }
    }
}

fn should_reset_legacy_platform_font(font_family: &str) -> bool {
    let trimmed = font_family.trim();
    trimmed == ".ZedMono" || trimmed.eq_ignore_ascii_case("monospace")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_synced_settings_preserves_local_only_settings() {
        let mut local = AppSettings {
            language: AppLanguage::SimplifiedChinese,
            font_family: "JetBrains Mono".into(),
            font_fallbacks: vec!["Noto Sans CJK SC".into()],
            font_size: 18.0,
            line_height: 26.0,
            theme_id: ThemeId::Dark,
            seed_color: "#123456".into(),
            recent_connections_count: 1,
            ..AppSettings::default()
        };
        let remote = AppSettings {
            language: AppLanguage::English,
            font_family: "Fira Code".into(),
            font_fallbacks: vec!["Sarasa Mono SC".into()],
            font_size: 13.0,
            line_height: 17.0,
            theme_id: ThemeId::Light,
            seed_color: "#abcdef".into(),
            recent_connections_count: 8,
            auto_collect_session_monitoring: true,
            last_tab_close_behavior: LastTabCloseBehavior::OpenNewHomeTab,
            monitor_history_duration: MonitorHistoryDuration::ThirtyMinutes,
            local_sftp_hidden_columns: vec![0, 1],
            remote_sftp_hidden_columns: vec![2, 3],
            local_vault_enabled: true,
            local_vault_auto_lock_duration: LocalVaultAutoLockDuration::FiveMinutes,
            ai_providers: vec![AiProviderConfig {
                id: "provider-1".into(),
                name: "OpenAI prod".into(),
                kind: AiProviderKind::OpenAi,
                model: "gpt-4o".into(),
                base_url: String::new(),
                api_key_env: String::new(),
                has_api_key: true,
                enabled: true,
                context_window: None,
            }],
            ..AppSettings::default()
        };

        local.apply_synced_settings(&remote.synced_settings());

        assert_eq!(local.language, AppLanguage::SimplifiedChinese);
        assert_eq!(local.font_family, "JetBrains Mono");
        assert_eq!(local.font_fallbacks, vec!["Noto Sans CJK SC"]);
        assert_eq!(local.font_size, 18.0);
        assert_eq!(local.line_height, 26.0);
        assert_eq!(local.theme_id, ThemeId::Dark);
        assert_eq!(local.seed_color, "#123456");
        assert_eq!(local.recent_connections_count, 8);
        assert!(local.auto_collect_session_monitoring);
        assert_eq!(
            local.last_tab_close_behavior,
            LastTabCloseBehavior::OpenNewHomeTab
        );
        assert_eq!(
            local.monitor_history_duration,
            MonitorHistoryDuration::ThirtyMinutes
        );
        assert_eq!(local.local_sftp_hidden_columns, vec![0, 1]);
        assert_eq!(local.remote_sftp_hidden_columns, vec![2, 3]);
        assert!(!local.local_vault_enabled);
        assert_eq!(
            local.local_vault_auto_lock_duration,
            LocalVaultAutoLockDuration::FiveMinutes
        );
        assert_eq!(local.ai_providers.len(), 1);
        assert_eq!(local.ai_providers[0].name, "OpenAI prod");
        assert!(local.ai_providers[0].has_api_key);
    }

    #[test]
    fn ai_provider_sanitize_fills_required_fields() {
        let mut settings = AppSettings {
            ai_providers: vec![AiProviderConfig {
                id: String::new(),
                name: String::new(),
                kind: AiProviderKind::Anthropic,
                model: String::new(),
                base_url: " https://api.example.com/ ".into(),
                api_key_env: " ANTHROPIC_API_KEY ".into(),
                has_api_key: false,
                enabled: true,
                context_window: None,
            }],
            ..AppSettings::default()
        };

        settings.sanitize();

        assert!(!settings.ai_providers[0].id.is_empty());
        assert_eq!(settings.ai_providers[0].name, "Anthropic");
        assert_eq!(
            settings.ai_providers[0].model,
            AiProviderKind::Anthropic.default_model()
        );
        assert_eq!(settings.ai_providers[0].base_url, "https://api.example.com");
        assert_eq!(settings.ai_providers[0].api_key_env, "ANTHROPIC_API_KEY");
    }

    #[test]
    fn synced_settings_omits_local_vault_enabled() {
        let settings = AppSettings {
            local_vault_enabled: true,
            ..AppSettings::default()
        };

        let serialized =
            toml::to_string(&settings.synced_settings()).expect("synced settings should serialize");

        assert!(!serialized.contains("local_vault_enabled"));
    }
}
