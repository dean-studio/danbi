use crate::error::{DanbiError, DanbiResult};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const CONFIG_FILENAME: &str = "config.json";
pub const DEFAULT_VAULT_DIRNAME: &str = "Danbi_Vault";

/// Provider-specific settings. Internally tagged by `kind` so the on-disk
/// JSON keeps a flat shape — important for backward compatibility with
/// pre-C2 configs, which already had `{ "kind": "bedrock", "auth_mode": …,
/// "region": … }` at this level.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProviderConfig {
    Bedrock {
        /// "profile" | "manual" | "env"
        auth_mode: String,
        /// AWS profile name when auth_mode = "profile"
        #[serde(default)]
        profile: Option<String>,
        /// AWS region, e.g. "us-east-1"
        region: String,
    },
    Nvidia {
        /// Keychain reference where the NVIDIA API key is stored. The actual
        /// secret never lives in `config.json` — only this pointer does.
        /// Conventional value: "keychain:danbi-nvidia".
        api_key_ref: String,
    },
    Openai {
        /// Keychain reference for the OpenAI API key.
        api_key_ref: String,
        /// Optional base URL override — lets users point at Azure OpenAI
        /// endpoints or self-hosted OpenAI-compatible proxies. `None`
        /// means the canonical `https://api.openai.com/v1`.
        #[serde(default)]
        base_url: Option<String>,
    },
    Anthropic {
        /// Keychain reference for the native Anthropic API key.
        api_key_ref: String,
    },
    Google {
        /// Keychain reference for the Gemini API key.
        api_key_ref: String,
    },
    Ollama {
        /// Base URL of the local Ollama server. Defaults to
        /// `http://localhost:11434` when unset.
        #[serde(default)]
        base_url: Option<String>,
    },
    Voyage {
        /// Keychain reference for the Voyage AI API key. Voyage only
        /// exposes embedding + rerank endpoints (no chat), so in practice
        /// this provider is only picked as `embed_provider`, not as the
        /// main LLM provider.
        api_key_ref: String,
    },
}

impl ProviderConfig {
    /// Stable string identifier used by UI.
    pub fn kind_str(&self) -> &'static str {
        match self {
            ProviderConfig::Bedrock { .. } => "bedrock",
            ProviderConfig::Nvidia { .. } => "nvidia",
            ProviderConfig::Openai { .. } => "openai",
            ProviderConfig::Anthropic { .. } => "anthropic",
            ProviderConfig::Google { .. } => "google",
            ProviderConfig::Ollama { .. } => "ollama",
            ProviderConfig::Voyage { .. } => "voyage",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelsConfig {
    pub routing: Option<String>,
    pub writer: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppearanceConfig {
    /// "dark" | "light" | "system"
    pub theme: String,
    /// Whether the sidebar-hidden compact layout is preferred
    #[serde(default)]
    pub compact: bool,
    /// Show unread-change count on the menu-bar tray icon when the main
    /// window is hidden. Reset to zero whenever the user opens the popover
    /// or the main window.
    #[serde(default = "default_tray_badge")]
    pub tray_badge: bool,
    /// Per-file unseen indicator (small dot) next to a domain in the
    /// sidebar. Computed in the frontend by comparing modified_ms with
    /// localStorage `danbi.lastSeen[<project>/<domain>]`.
    #[serde(default = "default_true")]
    pub unseen_sidebar_dots: bool,
    /// Per-project unseen counter — small "N" next to the project name.
    /// Same lastSeen source as `unseen_sidebar_dots`.
    #[serde(default = "default_true")]
    pub unseen_project_count: bool,
}

fn default_tray_badge() -> bool {
    true
}

fn default_true() -> bool {
    true
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            theme: "dark".into(),
            compact: false,
            tray_badge: true,
            unseen_sidebar_dots: true,
            unseen_project_count: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditorConfig {
    /// 자동 저장 (ctrl+s 없이도 blur에 저장)
    pub autosave: bool,
    /// Soft-wrap long lines
    pub word_wrap: bool,
}

impl Default for EditorConfig {
    fn default() -> Self {
        Self {
            autosave: false,
            word_wrap: true,
        }
    }
}

/// Keyboard shortcuts. Each value uses Tauri/electron-accelerator style:
///   "CommandOrControl+Shift+D", "Control+Space", "Alt+Space", etc.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShortcutsConfig {
    pub quick_capture: String,
}

/// Sticky state shared between Quick Capture invocations — remembers the
/// project/domain the user last wrote into so the next popup opens with the
/// same chip pre-selected.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CaptureState {
    pub last_project: Option<String>,
    pub last_domain: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpConfig {
    pub enabled: bool,
    pub port: u16,
    /// Bearer token required on every request. Generated on first enable.
    pub token: String,
}

impl Default for McpConfig {
    fn default() -> Self {
        // 0.1: 단비의 핵심 가치 = "외부 AI 의 외부 뇌" 이므로 MCP 서버는
        // 기본 ON. 사용자가 처음 설치하면 바로 Claude Code · Cursor 와
        // 연결 가능. 토큰은 첫 mcp_status 호출 시 자동 생성됨.
        Self {
            enabled: true,
            port: 47921,
            token: String::new(),
        }
    }
}

/// One-way mirror of the vault to a user-chosen folder (typically inside
/// iCloud Drive, Dropbox, OneDrive, …). Writes only; the mirror is never
/// read back to modify the vault.
///
/// Danbi treats this as a *backup*, not a sync — editing the mirror folder
/// will be silently overwritten on the next run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupConfig {
    /// Master toggle. When false, watcher-driven backups are skipped
    /// regardless of other fields.
    pub enabled: bool,
    /// Destination folder. Must be absolute and must NOT live inside the
    /// vault itself (guarded by `backup::validate_destination`).
    pub path: Option<String>,
    /// Debounce window in milliseconds — how long to wait after the last
    /// vault change before firing a backup run. 5s default lets editors
    /// finish streaming autosaves before we copy.
    pub debounce_ms: u64,
    /// Glob-ish prefixes the backup walker skips. Defaults protect the git
    /// repo and danbi internal state from landing in user-visible mirror.
    pub exclude: Vec<String>,
    /// Unix seconds of the last successful backup. Surface to UI so the
    /// user can see "백업 X분 전".
    #[serde(default)]
    pub last_run_at: Option<i64>,
    /// Human-readable message from the last run — "42 files copied"
    /// or "destination permission denied". Surface in Settings.
    #[serde(default)]
    pub last_message: Option<String>,
}

impl Default for BackupConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            path: None,
            debounce_ms: 5_000,
            exclude: default_backup_excludes(),
            last_run_at: None,
            last_message: None,
        }
    }
}

fn default_backup_excludes() -> Vec<String> {
    vec![".git".into(), ".danbi".into(), ".DS_Store".into()]
}

/// LLM 사용량 대시보드 설정. USD→KRW 환율은 사용자가 Settings에서
/// 바꿀 수 있도록 값만 들고 있는다. 은행 API 를 호출하지 않는 이유:
/// 오프라인 동작을 보장하고 싶고, ±50원 수준 오차는 "예상 금액" 용도에
/// 충분하다. 기본값은 분기마다 CLAUDE.md 체크 시 함께 갱신.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageConfig {
    /// 1 USD = N KRW. UI 에서 정수로 보여주되 내부적으로는 float.
    pub krw_per_usd: f64,
}

impl Default for UsageConfig {
    fn default() -> Self {
        // 2026-05 기준 ~1,380 KRW/USD. 분기 업데이트.
        Self {
            krw_per_usd: 1_380.0,
        }
    }
}

impl Default for ShortcutsConfig {
    fn default() -> Self {
        // 처음 설치하면 글로벌 단축키 OFF — macOS 한국어 IME (Control+Space)
        // 와 시스템 단축키 충돌 방지. 사용자가 Settings 에서 명시적으로
        // 등록할 때만 활성화. 빈 문자열 = 비활성.
        Self {
            quick_capture: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DanbiConfig {
    pub version: u32,
    pub vault_path: Option<String>,
    pub provider: Option<ProviderConfig>,
    pub models: ModelsConfig,
    /// Optional independent provider for embeddings. If None, the main
    /// `provider` is used — matching the old behaviour. When set, vault
    /// indexing calls this provider instead, letting users pair a paid
    /// LLM (Bedrock/Anthropic/...) with a free embedding backend
    /// (Ollama `nomic-embed-text` is the canonical case).
    #[serde(default)]
    pub embed_provider: Option<ProviderConfig>,
    /// Embedding model id. If None, the resolved embed provider's
    /// `default_embed_model()` is used.
    #[serde(default)]
    pub embed_model: Option<String>,
    /// 자동화 (요약·purpose 작성·ghost 제안) 에 사용할 LLM 모델 id.
    /// embed provider 와 같은 키를 재사용하므로, 같은 provider 안의 다른
    /// LLM 모델 ID 를 골라준다. 비어있으면 `resolve_summarize_model` 의
    /// provider 별 디폴트 사용.
    #[serde(default)]
    pub automation_model: Option<String>,
    /// Names of registered projects (each owns a folder under Projects/)
    #[serde(default)]
    pub projects: Vec<String>,
    /// Default domain filenames suggested when a project is created
    #[serde(default = "default_domains")]
    pub default_domains: Vec<String>,
    /// Sub-folders auto-created inside every new project (e.g. "daily").
    #[serde(default = "default_folders")]
    pub default_folders: Vec<String>,
    #[serde(default)]
    pub appearance: AppearanceConfig,
    #[serde(default)]
    pub editor: EditorConfig,
    #[serde(default)]
    pub shortcuts: ShortcutsConfig,
    #[serde(default)]
    pub capture: CaptureState,
    #[serde(default)]
    pub mcp: McpConfig,
    #[serde(default)]
    pub backup: BackupConfig,
    #[serde(default)]
    pub usage: UsageConfig,
    /// 사용자가 구성한 프로젝트 그룹. 순서는 사이드바 표시 순서 그대로.
    /// 그룹에 속하지 않은 프로젝트는 사이드바 상단 "Ungrouped" 영역에
    /// 표시된다. project_groups 가 비어 있으면 기존 평면 리스트 동작.
    #[serde(default)]
    pub project_groups: Vec<ProjectGroup>,
    /// 프로젝트별 "마지막으로 확인한" Unix timestamp (초). 사이드바 배지는
    /// 이 값 이후에 쌓인 커밋·파일 변경을 집계해서 표시한다.
    #[serde(default)]
    pub project_last_seen_at: std::collections::HashMap<String, i64>,
    /// 도메인별 "마지막으로 확인한" Unix timestamp (초). 키 포맷:
    /// "<project>/<domain>" 예: "보니_에이전트/notes/index.md".
    /// 사이드바 도메인 행 옆 점/배지가 이 타임스탬프 이후 mtime 으로
    /// 갱신된 파일에만 표시된다. 도메인 클릭 시 해당 키를 갱신해서
    /// 배지를 끈다.
    #[serde(default)]
    pub domain_last_seen_at: std::collections::HashMap<String, i64>,
    /// 프로젝트별 lucide-react 아이콘 이름. 예: `{"보니_에이전트": "Bot"}`.
    /// 사이드바·홈·그래프 등 어디서든 같은 아이콘으로 표시되도록
    /// 단일 source of truth 로 둔다. 비어있으면 기본 폴더 아이콘.
    #[serde(default)]
    pub project_icons: std::collections::HashMap<String, String>,
    /// 프로젝트별 강조 색 키. 예: `{"보니_에이전트": "yellow"}`. 사이드바
    /// 활성 행·프로젝트 헤더·expanded 카드 테두리에 일관 적용된다. 비어있으면
    /// 기본 accent-blue 톤. 키만 저장하고 hex 매핑은 프론트의 팔레트
    /// 테이블에서 결정 — 테마 업데이트 시 데이터 마이그레이션 없이 톤만
    /// 바꿀 수 있다.
    #[serde(default)]
    pub project_colors: std::collections::HashMap<String, String>,
    /// 사용자가 온보딩에서 고른 사용 패턴. UI 표시·배너 분기에 쓴다.
    /// "claude_code" / "builtin" / "minimal" / 비어있음(레거시).
    #[serde(default)]
    pub preset: Option<String>,
}

/// 한 개의 프로젝트 그룹. 그룹은 순서를 갖고, 그 안의 프로젝트들도
/// 순서를 유지한다. 프로젝트 이름은 `DanbiConfig.projects` 에 등록된
/// 이름과 동일해야 한다 (참조 무결성은 런타임에 sanitize 함수로 보정).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectGroup {
    /// 안정 식별자. 이름이 바뀌어도 저장된 참조가 깨지지 않도록.
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub projects: Vec<String>,
    /// 사이드바에서 그룹 섹션이 접혀 있는지 여부. 기본은 펼침.
    #[serde(default)]
    pub collapsed: bool,
}

fn default_domains() -> Vec<String> {
    vec!["ui.md".into(), "backend.md".into(), "plan.md".into()]
}

fn default_folders() -> Vec<String> {
    vec!["daily".into()]
}


pub fn default_vault_path() -> DanbiResult<PathBuf> {
    let home = dirs::home_dir()
        .ok_or_else(|| DanbiError::Config("home directory not found".into()))?;
    Ok(home.join(DEFAULT_VAULT_DIRNAME))
}

pub fn config_path(vault: &std::path::Path) -> PathBuf {
    vault.join(CONFIG_FILENAME)
}

/// Current schema version. Bumped whenever the on-disk shape changes in a
/// way that would confuse an older build. C2 (provider enum) is v2.
pub const CURRENT_SCHEMA_VERSION: u32 = 2;

pub fn load_config(vault: &std::path::Path) -> DanbiResult<Option<DanbiConfig>> {
    let p = config_path(vault);
    if !p.exists() {
        return Ok(None);
    }
    let s = std::fs::read_to_string(&p)?;
    let cfg: DanbiConfig = serde_json::from_str(&s)?;
    let migrated = migrate(cfg);
    // If the version stamp was behind, write the upgraded config back so the
    // next load is already normalized. We do this lazily rather than eagerly
    // on app start to avoid touching disk when nothing actually needs to
    // change.
    if migrated.version < CURRENT_SCHEMA_VERSION {
        let mut bumped = migrated.clone();
        bumped.version = CURRENT_SCHEMA_VERSION;
        save_config(vault, &bumped)?;
        return Ok(Some(bumped));
    }
    Ok(Some(migrated))
}

pub fn save_config(vault: &std::path::Path, cfg: &DanbiConfig) -> DanbiResult<()> {
    std::fs::create_dir_all(vault)?;
    let p = config_path(vault);
    let mut cfg = cfg.clone();
    if cfg.version < CURRENT_SCHEMA_VERSION {
        cfg.version = CURRENT_SCHEMA_VERSION;
    }
    let s = serde_json::to_string_pretty(&cfg)?;
    std::fs::write(&p, s)?;
    Ok(())
}

/// Applies in-place migrations for older schema versions. The v1 layout used a
/// struct-shaped `ProviderConfig { kind: String, auth_mode, profile, region }`
/// rather than a tagged enum. serde's internally-tagged enum happens to parse
/// that JSON fine (because the v1 kind was always "bedrock"), so no field
/// reshuffling is needed — the only work is bumping the version stamp.
fn migrate(cfg: DanbiConfig) -> DanbiConfig {
    // Place for future migrations. For now, v1 → v2 is purely a stamp bump.
    cfg
}
