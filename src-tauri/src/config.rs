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
    /// 자동 저장 — 입력이 멈추면 디바운스 후, blur 시에도 저장. 기본 켜짐.
    /// 꺼도 문서 전환 시 flush 는 항상 한다 (DocView).
    pub autosave: bool,
    /// Soft-wrap long lines
    pub word_wrap: bool,
}

impl Default for EditorConfig {
    fn default() -> Self {
        Self {
            autosave: true,
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

/// 쉬는 시간 정리 (dreaming). vault 가 한동안 조용하면 프로젝트별로
/// 깨진 링크·오래된 문서·daily 에 묻힌 결정을 찾아 검토함에 올린다.
/// 문서는 절대 자동으로 고치지 않는다 — 사용자가 승인한 것만 기록.
/// 실행 시각·거절 기억은 `.danbi/dreaming.json` 에 있다 (config 가 아님).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DreamingConfig {
    /// 자동 실행 ON/OFF. 꺼도 Settings 의 "지금 정리" 는 동작한다.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 승격 제목 다듬기 등에 AI 연동 (embed provider) 의 LLM 을 쓸지.
    /// 기본 OFF — LLM 없이도 전 과정이 동작한다.
    #[serde(default)]
    pub use_llm: bool,
    /// 마지막 vault 변경 뒤 이 분만큼 조용해야 한 번 돈다.
    #[serde(default = "default_dream_idle_minutes")]
    pub idle_minutes: u64,
    /// 한 번에 검토함에 올리는 최대 건수 — 쌓여서 피로해지지 않게.
    #[serde(default = "default_dream_max_per_run")]
    pub max_per_run: usize,
    /// 확실한 링크 수리(같은 문서로 정해지는 것)는 검토함에 올리지 않고
    /// 바로 고친다. 단비 이름으로 커밋되어 되돌릴 수 있다. 기본 ON.
    #[serde(default = "default_true")]
    pub auto_fix_links: bool,
}

fn default_dream_idle_minutes() -> u64 {
    20
}

fn default_dream_max_per_run() -> usize {
    8
}

impl Default for DreamingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            use_llm: false,
            idle_minutes: default_dream_idle_minutes(),
            max_per_run: default_dream_max_per_run(),
            auto_fix_links: true,
        }
    }
}

/// LLM 사용량 대시보드 설정. v0.8.0 부터 비용/환율 계산은 제거되고
/// 토큰량만 집계·표시한다.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageConfig {
    /// MCP inbound 이벤트 추적 ON/OFF. v0.4.0 기준 기본 ON.
    /// 끄면 MCP 핸들러는 record_mcp_inbound 자체를 스킵해 디스크 쓰기 0.
    #[serde(default = "default_true")]
    pub mcp_tracking: bool,
    /// MCP inbound 이벤트 보존 기간 (일). 이 기간 이전 이벤트는
    /// 매일 자정 sweep 에서 archive 파일로 옮기고 usage.jsonl 에서
    /// 삭제한다. 0 또는 음수면 retention 비활성 (영구 보존).
    #[serde(default = "default_retention_days")]
    pub mcp_retention_days: i64,

    // ---------- v0.7.0 Claude Code 사용량 ----------
    /// Claude Code transcript 추적 ON/OFF. 기본 ON.
    /// (transcript 는 사용자 자기 디스크의 자기 파일이라 권한 이슈 없음)
    #[serde(default = "default_true")]
    pub claude_code_tracking: bool,

    // ---------- v0.7.0 Tray ----------
    /// 메뉴바 tray 아이콘에 "오늘 토큰 / 비용" mini 노출.
    /// 끄면 tray 자체는 그대로 (단비 본 앱 toggle), 사용량 popover 만 비활성.
    #[serde(default = "default_true")]
    pub tray_usage: bool,
    /// 메뉴바 텍스트로 오늘 토큰 압축 표기 (예: `1.2M`).
    /// 너비 차지 — 기본 OFF.
    #[serde(default)]
    pub tray_label: bool,
}

fn default_retention_days() -> i64 {
    365
}

impl Default for UsageConfig {
    fn default() -> Self {
        Self {
            mcp_tracking: true,
            mcp_retention_days: 365,
            claude_code_tracking: true,
            tray_usage: true,
            tray_label: false,
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
    #[serde(default)]
    pub dreaming: DreamingConfig,
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
    /// 검토함에서 "제외" 한 고아 문서 경로 (`<project>/<domain>`). healing
    /// 스캐너 결과를 보여줄 때 걸러낸다 — 리뷰 큐에 복제하지 않는다.
    #[serde(default)]
    pub review_excluded_paths: Vec<String>,
    /// 사용자가 "보관" 한 프로젝트. 사이드바 '쉬는 중' 아래로 내려가고
    /// 할 일함·시간 분배에서 빠진다. 파일은 그대로 둔다.
    #[serde(default)]
    pub project_archived: Vec<String>,
    /// "다시 시작" 누른 시각 (Unix 초). 손대지 않은 지 오래된 프로젝트도
    /// 이 시각부터 다시 활동 중으로 친다.
    #[serde(default)]
    pub project_resumed_at: std::collections::HashMap<String, i64>,
    /// 이 일수 넘게 손대지 않은 프로젝트를 '쉬는 중' 으로 접는다.
    /// 비어 있으면 [`DEFAULT_DORMANT_DAYS`], 0 = 끔. (`Default` 파생으로
    /// 만든 새 설정도 기본값을 따르도록 Option 으로 둔다.)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dormant_days: Option<u32>,
    /// 연결 파일 없이 사용자가 직접 프로젝트에 묶은 코드 폴더
    /// (절대 경로 → 프로젝트). 시간 분배에서 이 폴더 안의 Claude Code
    /// 작업을 그 프로젝트로 센다. 하위 폴더도 포함.
    #[serde(default)]
    pub folder_projects: std::collections::HashMap<String, String>,
}

pub const DEFAULT_DORMANT_DAYS: u32 = 30;

impl DanbiConfig {
    /// 0 이면 '쉬는 중' 자동 접기를 쓰지 않는다.
    pub fn dormant_days(&self) -> u32 {
        self.dormant_days.unwrap_or(DEFAULT_DORMANT_DAYS)
    }
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
    // 디버그 빌드 전용: 실제 vault 를 건드리지 않고 복제본으로 dev 앱을 띄울 때.
    #[cfg(debug_assertions)]
    if let Some(dir) = std::env::var_os("DANBI_DEV_VAULT").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
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
    std::fs::write(&p, &s)?;

    // Keep the fixed pointer in sync. The read side (`load_config()` with no
    // arg, `current_vault_path()`, and the MCP server) always resolves the
    // active vault via `default_vault_path()/config.json` → its `vault_path`
    // field. If the user picked a vault folder other than the default, that
    // pointer would otherwise never learn about it — leaving MCP/connect
    // resolving to the wrong (or a stale) vault ("project not found"). So when
    // we're writing to a non-default location, mirror the same config to the
    // default location so the two views can't drift apart.
    if let Ok(default_dir) = default_vault_path() {
        let pointer = config_path(&default_dir);
        if pointer != p {
            let _ = std::fs::create_dir_all(&default_dir);
            let _ = std::fs::write(&pointer, &s);
        }
    }
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

#[cfg(test)]
mod dreaming_default_tests {
    use super::DreamingConfig;

    /// 예전 config(필드 없음)도, 새 기본값도 확실한 링크 자동 수리는 켜짐.
    #[test]
    fn auto_fix_links_defaults_on() {
        assert!(DreamingConfig::default().auto_fix_links);
        let old: DreamingConfig = serde_json::from_str(r#"{"enabled":true,"use_llm":false,"idle_minutes":20,"max_per_run":8}"#).unwrap();
        assert!(old.auto_fix_links);
        let off: DreamingConfig = serde_json::from_str(r#"{"auto_fix_links":false}"#).unwrap();
        assert!(!off.auto_fix_links);
    }
}
