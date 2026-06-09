import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

// --- Types mirrored from Rust ---

// Mirrors Rust's `ProviderConfig` (internally-tagged enum on `kind`).
// The on-disk JSON keeps a flat shape, so fields for each variant sit
// alongside `kind` at the same level.
export type ProviderConfig =
  | {
      kind: "bedrock";
      auth_mode: "profile" | "manual" | "env";
      profile?: string | null;
      region: string;
    }
  | { kind: "nvidia"; api_key_ref: string }
  | { kind: "openai"; api_key_ref: string; base_url?: string | null }
  | { kind: "anthropic"; api_key_ref: string }
  | { kind: "google"; api_key_ref: string }
  | { kind: "ollama"; base_url?: string | null }
  | { kind: "voyage"; api_key_ref: string };

export type ModelsConfig = {
  routing?: string | null;
  writer?: string | null;
};

export type AppearanceConfig = {
  theme: "dark" | "light" | "system";
  compact: boolean;
  tray_badge?: boolean;
  unseen_sidebar_dots?: boolean;
  unseen_project_count?: boolean;
};

export type EditorConfig = {
  autosave: boolean;
  word_wrap: boolean;
};

export type ShortcutsConfig = {
  quick_capture: string;
};

export type CaptureState = {
  last_project: string | null;
  last_domain: string | null;
};

export type McpConfig = {
  enabled: boolean;
  port: number;
  token: string;
};

export type BackupConfig = {
  enabled: boolean;
  path: string | null;
  debounce_ms: number;
  exclude: string[];
  last_run_at: number | null;
  last_message: string | null;
};

export type BackupReport = {
  copied: number;
  skipped: number;
  removed: number;
  bytes: number;
  duration_ms: number;
};

// ---- Vector index ----
export type VectorIndexStats = {
  count: number;
  oldest: number | null;
  newest: number | null;
  model: string | null;
};

export type VectorReindexReport = {
  total: number;
  embedded: number;
  skipped: number;
  removed: number;
};

export type VectorHit = {
  project: string;
  domain: string;
  score: number;
};

// ---- Review queue ----
export type ReviewStatus = "pending" | "resolved" | "dismissed";
export type ReviewItem = {
  id: string;
  kind: string;
  project: string | null;
  domain: string | null;
  reason: string;
  status: ReviewStatus;
  created_at: number;
  resolved_at: number | null;
};
export type ReviewStore = { items: ReviewItem[] };

// ---- Project context (Karpathy-style purpose.md + schema.md) ----
export type ProjectContextStatus = {
  has_purpose: boolean;
  has_schema: boolean;
  purpose: string | null;
  schema: string | null;
  purpose_clipped: boolean;
  schema_clipped: boolean;
};

export type McpProjectEndpoint = {
  project: string;
  id: string;
  url: string;
};

export type McpStatus = {
  enabled: boolean;
  running: boolean;
  port: number;
  url: string;
  token: string;
  projects: McpProjectEndpoint[];
};

export type CaptureContext = {
  projects: string[];
  domains: Record<string, string[]>;
  last_project: string | null;
  last_domain: string | null;
};

export type SearchHit = {
  project: string;
  domain: string;
  relevance: number;
  snippet: string;
};

export type SearchResponse = {
  hits: SearchHit[];
  summary: string;
};

export type DanbiConfig = {
  version: number;
  vault_path?: string | null;
  provider?: ProviderConfig | null;
  models: ModelsConfig;
  embed_provider?: ProviderConfig | null;
  embed_model?: string | null;
  automation_model?: string | null;
  projects: string[];
  default_domains: string[];
  default_folders: string[];
  appearance: AppearanceConfig;
  editor: EditorConfig;
  shortcuts: ShortcutsConfig;
  capture: CaptureState;
  mcp: McpConfig;
  backup: BackupConfig;
  usage?: {
    krw_per_usd: number;
    mcp_tracking: boolean;
    mcp_retention_days: number;
  };
  project_groups?: ProjectGroup[];
  project_last_seen_at?: Record<string, number>;
  project_icons?: Record<string, string>;
  project_colors?: Record<string, string>;
  preset?: "claude_code" | "builtin" | "minimal" | null;
};

export type AwsProfile = {
  name: string;
  region: string | null;
  source: "credentials" | "config";
};

export type AwsDetection = {
  has_credentials_file: boolean;
  has_config_file: boolean;
  profiles: AwsProfile[];
};

export type BedrockModel = {
  id: string;
  name: string | null;
  provider: string | null;
  on_demand: boolean;
  modalities_in: string[];
  modalities_out: string[];
};

export type BedrockTestResult = {
  ok: boolean;
  detail: string;
};

// Mirrors Rust's `providers::ModelInfo`. Used by NVIDIA listing today — the
// Bedrock path still returns its own `BedrockModel` shape for compatibility
// with the existing model picker. Fields line up deliberately so UI code can
// share filter/sort helpers once we consolidate.
export type ModelInfo = {
  id: string;
  name: string | null;
  provider: string | null;
  on_demand: boolean;
  modalities_in: string[];
  modalities_out: string[];
};

export type ProviderTestResult = {
  ok: boolean;
  detail: string;
};

export type AuthInput =
  | { kind: "profile"; name: string }
  | { kind: "manual"; label: string }
  | { kind: "env" };

export type DomainNode = {
  /** "ui.md" or "daily/2026-05-11.md" — path relative to project folder. */
  name: string;
  bytes: number;
  modified_ms: number | null;
  /** First H1/H2 heading sniffed from the file body (sans `#`s, trimmed,
   *  capped). `daily/**` files skip this server-side to keep large
   *  journals cheap. Renderer shows it dim next to the filename. */
  title?: string | null;
};

export type SubfolderNode = {
  /** Fully-qualified folder path from the project root, e.g. "daily" or
   *  "daily/2026-01". The frontend uses this verbatim when calling
   *  rename/delete/move APIs. */
  name: string;
  domains: DomainNode[];
  /** One more level of nesting. Currently capped at depth 2 — these
   *  children themselves never carry further `subfolders`. */
  subfolders: SubfolderNode[];
};

export type ProjectNode = {
  name: string;
  domains: DomainNode[];
  subfolders: SubfolderNode[];
};

export type VaultTree = {
  vault_path: string;
  projects: ProjectNode[];
};

export type VaultChangedPayload = {
  vault: string;
  count: number;
};

export type RoutingContext = {
  projects: string[];
  domains: Record<string, string[]>;
  sticky_project: string | null;
  sticky_domain: string | null;
};

export type Intent =
  | "append"
  | "rewrite"
  | "summarize"
  | "ask"
  | "compound"
  | "unknown";

export type CompoundCitation = {
  project: string;
  domain: string;
  note: string;
};

export type CompoundSource = {
  project: string;
  domain: string;
  content: string;
};

export type CompoundPlan = {
  summary: string;
  detail: string;
  draft: string;
  sources: CompoundCitation[];
};

export type CompoundPreview = {
  target_project: string;
  target_domain: string;
  plan: CompoundPlan;
  sources: CompoundSource[];
  approx_input_chars: number;
  approx_output_chars: number;
};

export type RoutingResult = {
  intent: Intent;
  project: string | null;
  domain: string | null;
  confidence: number;
  needs_clarification: boolean;
  clarification_type: "project" | "domain" | null;
  candidate_projects: string[];
  candidate_domains: string[];
  summary: string;
};

export type EditOp =
  | { op: "append"; content: string }
  | { op: "insert_after"; heading: string; content: string }
  | { op: "replace_section"; heading: string; new_body: string }
  | { op: "rewrite_all"; content: string };

export type PlanPreview = {
  summary: string;
  detail: string;
  draft: string;
  op: EditOp | null;
  answer: string | null;
};

export type ApplyResult = {
  project: string;
  domain: string;
  commit_before: string | null;
  commit_after: string | null;
  bytes_before: number;
  bytes_after: number;
};

export type Extracted = {
  filename: string;
  mime: string;
  kind: "pdf" | "docx" | "text" | "unknown";
  text: string;
  bytes: number;
  truncated: boolean;
};

export type Attachment = {
  filename: string;
  kind: string;
  text: string;
  truncated: boolean;
};

export type CommitSummary = {
  id: string;
  summary: string;
  ts: number;
};

export type ExportRecord = {
  id: string;
  project: string;
  source_domain: string;
  created_at: number;
  provider: string;
  model: string;
  md_path: string;
  html_path: string;
  md_bytes: number;
};

export type WikiLink = {
  project: string;
  domain: string;
};

export type LinkIndex = {
  outgoing: Record<string, WikiLink[]>;
  incoming: Record<string, WikiLink[]>;
};

export type VaultSuggestion =
  | { kind: "Orphan"; project: string; domain: string }
  | { kind: "Empty"; project: string; domain: string }
  | { kind: "EmptyProject"; project: string }
  | {
      kind: "Oversized";
      project: string;
      domain: string;
      bytes: number;
    };

export type DailyNoteRef = {
  project: string;
  domain: string;
  date: string;
  bytes: number;
  modified_ms: number | null;
};

export type DailySnapshot = {
  today: string;
  today_notes: DailyNoteRef[];
  one_week_ago: DailyNoteRef[];
  one_month_ago: DailyNoteRef[];
  one_year_ago: DailyNoteRef[];
};

export type QaCitation = {
  project: string;
  domain: string;
  note: string;
};

export type QaAnswer = {
  answer: string;
  citations: QaCitation[];
  sources: string[];
};

export type BriefingCommit = {
  id: string;
  summary: string;
  ts: number;
  files: string[];
};

export type BriefingRangeInfo = {
  range: string;
  since_ts: number;
  until_ts: number;
};

export type BriefingResult = {
  project: string;
  range: BriefingRangeInfo;
  commits: BriefingCommit[];
  changed_files: string[];
  summary: string;
};

// Briefing dashboard — "오늘의 단비" aggregate payload.
export type GhostSuggestion = {
  project: string;
  id: string;
  source_domain: string;
  target_domain: string;
  reason: string;
  created_at: number;
};

export type ActivityWindow = {
  days: number;
  commit_count: number;
  changed_files: string[];
  recent_summaries: string[];
};

export type DashboardSnapshot = {
  generated_at: string;
  ghost_suggestions: GhostSuggestion[];
  healing: VaultSuggestion[];
  daily: DailySnapshot;
  activity: ActivityWindow;
};

// ---- MCP inbound dashboard (v0.4.0) ----
//
// Counts content tokens that external agents (Claude Code / Codex) saved
// into the vault via the MCP server. The numbers are estimates from
// cl100k_base — see the disclaimer field on every payload.

export type McpInboundRange = "today" | "7d" | "30d" | "90d" | "all";

export type McpClientBreakdown = {
  client: string;
  tokens: number;
  calls: number;
};

export type McpToolBreakdown = {
  tool: string;
  tokens: number;
  calls: number;
};

export type McpDailyPoint = {
  date: string; // YYYY-MM-DD, local time
  tokens: number;
  calls: number;
};

export type McpDomainStub = {
  domain: string;
  tokens: number;
  calls: number;
};

export type McpProjectStats = {
  project: string;
  tokens: number;
  calls: number;
  by_client: McpClientBreakdown[];
  top_domains: McpDomainStub[];
};

export type McpAnomaly = {
  project: string;
  domain: string;
  date: string;
  tokens: number;
  baseline: number;
  multiple: number;
};

export type McpCostEstimate = {
  model_stem: string;
  usd_per_mtok_input: number;
  krw_per_usd: number;
  krw: number;
  usd: number;
  reference_only: boolean;
};

export type McpTopContributor = {
  project: string;
  domain: string;
  tokens: number;
  calls: number;
};

export type McpHeatmap = {
  /// `cells[dow][hour]`. dow 0 = Sunday.
  cells: number[][];
  max_cell: number;
  total_tokens: number;
};

export type McpVaultSummary = {
  range: string;
  from_ms: number;
  to_ms: number;
  total_tokens: number;
  total_calls: number;
  by_client: McpClientBreakdown[];
  by_tool: McpToolBreakdown[];
  by_project: McpProjectStats[];
  daily: McpDailyPoint[];
  top_contributors: McpTopContributor[];
  anomalies: McpAnomaly[];
  cost_estimate: McpCostEstimate;
  heatmap: McpHeatmap;
  disclaimer: string;
  estimated: boolean;
};

export type McpProjectDetail = {
  project: string;
  range: string;
  from_ms: number;
  to_ms: number;
  total_tokens: number;
  total_calls: number;
  by_client: McpClientBreakdown[];
  by_tool: McpToolBreakdown[];
  by_domain: McpDomainStub[];
  daily: McpDailyPoint[];
  cost_estimate: McpCostEstimate;
  disclaimer: string;
  estimated: boolean;
};

export type McpDomainDetail = {
  project: string;
  domain: string;
  range: string;
  from_ms: number;
  to_ms: number;
  total_tokens: number;
  total_calls: number;
  by_client: McpClientBreakdown[];
  by_tool: McpToolBreakdown[];
  daily: McpDailyPoint[];
  cost_estimate: McpCostEstimate;
  disclaimer: string;
  estimated: boolean;
};

// ---- Graph view ----

export type GraphNode = {
  id: string;
  project: string;
  domain: string;
  label: string;
  bytes: number;
  community: number;
  degree: number;
};

export type GraphEdge = {
  source: string;
  target: string;
  kind: "confirmed" | "ghost" | "soft";
  score: number;
  ghost_id: string | null;
  ghost_project: string | null;
  reason: string | null;
};

export type SparseCommunity = { id: number; members: string[] };
export type HubNode = { id: string; degree: number };
export type SurprisingEdge = {
  source: string;
  target: string;
  source_community: number;
  target_community: number;
};

export type GraphInsights = {
  isolated: string[];
  sparse_communities: SparseCommunity[];
  bridges: string[];
  hubs: HubNode[];
  surprising: SurprisingEdge[];
};

export type GraphData = {
  nodes: GraphNode[];
  edges: GraphEdge[];
  insights: GraphInsights;
};

export type GhostStatus = "pending" | "accepted" | "rejected";

export type GhostLink = {
  id: string;
  source_domain: string;
  target_domain: string;
  reason: string;
  status: GhostStatus;
  created_at: number;
};

export type GhostStore = {
  links: GhostLink[];
  last_scan_at: number | null;
};

export type SampleFile = { name: string; content: string };
export type SampleProject = { name: string; files: SampleFile[] };
export type VaultTemplate = {
  id: string;
  name: string;
  description: string;
  icon: string;
  default_domains: string[];
  default_folders: string[];
  sample_project: SampleProject | null;
};

export const VAULT_CHANGED_EVENT = "vault:changed";

export function onVaultChanged(
  cb: (p: VaultChangedPayload) => void,
): Promise<UnlistenFn> {
  return listen<VaultChangedPayload>(VAULT_CHANGED_EVENT, (e) => cb(e.payload));
}

export type TrashEntry = {
  id: string;
  project: string;
  /** Path relative to the project root (file: domain, folder: folder path). */
  original_path: string;
  /** "file" | "folder". */
  kind: string;
  /** Unix epoch seconds. */
  deleted_at: number;
  size_bytes: number;
};

export type ReindexProgress = {
  phase: "embedding" | "waiting" | "done";
  done: number;
  total: number;
  last_file: string | null;
  wait_secs: number | null;
};

export const REINDEX_PROGRESS_EVENT = "vector:reindex_progress";
export const REINDEX_DONE_EVENT = "vector:reindex_done";

export function onReindexProgress(
  cb: (p: ReindexProgress) => void,
): Promise<UnlistenFn> {
  return listen<ReindexProgress>(REINDEX_PROGRESS_EVENT, (e) => cb(e.payload));
}

export function onReindexDone(
  cb: (r: VectorReindexReport) => void,
): Promise<UnlistenFn> {
  return listen<VectorReindexReport>(REINDEX_DONE_EVENT, (e) => cb(e.payload));
}

function uint8ArrayToBase64(bytes: Uint8Array): string {
  // btoa does not like raw binary; build the binary string in chunks first.
  const CHUNK = 0x8000;
  let binary = "";
  for (let i = 0; i < bytes.length; i += CHUNK) {
    const slice = bytes.subarray(i, i + CHUNK);
    binary += String.fromCharCode.apply(null, slice as unknown as number[]);
  }
  return btoa(binary);
}

// --- IPC wrappers ---

export const ipc = {
  ping: () => invoke<string>("ping"),
  defaultVault: () => invoke<string>("default_vault"),
  loadConfig: (vaultPath?: string) =>
    invoke<DanbiConfig | null>("load_config", { vaultPath: vaultPath ?? null }),
  saveConfig: (vaultPath: string, cfg: DanbiConfig) =>
    invoke<void>("save_config", { vaultPath, cfg }),
  detectAws: () => invoke<AwsDetection>("detect_aws"),
  storeManualCredentials: (p: {
    label: string;
    accessKeyId: string;
    secretAccessKey: string;
    sessionToken?: string;
  }) =>
    invoke<void>("store_manual_credentials", {
      label: p.label,
      accessKeyId: p.accessKeyId,
      secretAccessKey: p.secretAccessKey,
      sessionToken: p.sessionToken ?? null,
    }),
  deleteManualCredentials: (label: string) =>
    invoke<void>("delete_manual_credentials", { label }),
  listBedrockModels: (auth: AuthInput, region: string) =>
    invoke<BedrockModel[]>("list_bedrock_models", { auth, region }),
  testBedrock: (auth: AuthInput, region: string, modelId?: string) =>
    invoke<BedrockTestResult>("test_bedrock", {
      auth,
      region,
      modelId: modelId ?? null,
    }),
  storeNvidiaApiKey: (apiKey: string) =>
    invoke<string>("store_nvidia_api_key", { apiKey }),
  deleteNvidiaApiKey: () => invoke<void>("delete_nvidia_api_key"),
  listNvidiaModels: () => invoke<ModelInfo[]>("list_nvidia_models"),
  testNvidia: (apiKey?: string, modelId?: string) =>
    invoke<ProviderTestResult>("test_nvidia", {
      apiKey: apiKey ?? null,
      modelId: modelId ?? null,
    }),

  storeOpenaiApiKey: (apiKey: string) =>
    invoke<string>("store_openai_api_key", { apiKey }),
  deleteOpenaiApiKey: () => invoke<void>("delete_openai_api_key"),
  listOpenaiModels: () => invoke<ModelInfo[]>("list_openai_models"),
  testOpenai: (apiKey?: string, baseUrl?: string, modelId?: string) =>
    invoke<ProviderTestResult>("test_openai", {
      apiKey: apiKey ?? null,
      baseUrl: baseUrl ?? null,
      modelId: modelId ?? null,
    }),

  storeAnthropicApiKey: (apiKey: string) =>
    invoke<string>("store_anthropic_api_key", { apiKey }),
  deleteAnthropicApiKey: () => invoke<void>("delete_anthropic_api_key"),
  listAnthropicModels: () => invoke<ModelInfo[]>("list_anthropic_models"),
  testAnthropic: (apiKey?: string, modelId?: string) =>
    invoke<ProviderTestResult>("test_anthropic", {
      apiKey: apiKey ?? null,
      modelId: modelId ?? null,
    }),

  storeGoogleApiKey: (apiKey: string) =>
    invoke<string>("store_google_api_key", { apiKey }),
  deleteGoogleApiKey: () => invoke<void>("delete_google_api_key"),
  listGoogleModels: () => invoke<ModelInfo[]>("list_google_models"),
  testGoogle: (apiKey?: string, modelId?: string) =>
    invoke<ProviderTestResult>("test_google", {
      apiKey: apiKey ?? null,
      modelId: modelId ?? null,
    }),

  storeVoyageApiKey: (apiKey: string) =>
    invoke<string>("store_voyage_api_key", { apiKey }),
  deleteVoyageApiKey: () => invoke<void>("delete_voyage_api_key"),
  listVoyageModels: () => invoke<ModelInfo[]>("list_voyage_models"),
  testVoyage: (apiKey?: string, modelId?: string) =>
    invoke<ProviderTestResult>("test_voyage", {
      apiKey: apiKey ?? null,
      modelId: modelId ?? null,
    }),

  listOllamaModels: (baseUrl?: string) =>
    invoke<ModelInfo[]>("list_ollama_models", { baseUrl: baseUrl ?? null }),
  testOllama: (baseUrl?: string, modelId?: string) =>
    invoke<ProviderTestResult>("test_ollama", {
      baseUrl: baseUrl ?? null,
      modelId: modelId ?? null,
    }),

  initVault: (vaultPath: string) => invoke<void>("init_vault", { vaultPath }),
  listTree: (vaultPath: string) => invoke<VaultTree>("list_tree", { vaultPath }),
  createProject: (
    vaultPath: string,
    name: string,
    defaultDomains: string[],
    defaultFolders?: string[],
  ) =>
    invoke<void>("create_project", {
      vaultPath,
      name,
      defaultDomains,
      defaultFolders: defaultFolders ?? null,
    }),
  renameProject: (vaultPath: string, oldName: string, newName: string) =>
    invoke<void>("rename_project", { vaultPath, old: oldName, new: newName }),
  deleteProject: (vaultPath: string, name: string) =>
    invoke<void>("delete_project", { vaultPath, name }),
  createDomain: (vaultPath: string, project: string, domain: string) =>
    invoke<string>("create_domain", { vaultPath, project, domain }),
  renameDomain: (
    vaultPath: string,
    project: string,
    oldName: string,
    newName: string,
  ) =>
    invoke<string>("rename_domain", {
      vaultPath,
      project,
      old: oldName,
      new: newName,
    }),
  deleteDomain: (vaultPath: string, project: string, domain: string) =>
    invoke<void>("delete_domain", { vaultPath, project, domain }),
  createFolder: (vaultPath: string, project: string, folder: string) =>
    invoke<void>("create_folder", { vaultPath, project, folder }),
  renameFolder: (
    vaultPath: string,
    project: string,
    oldName: string,
    newName: string,
  ) =>
    invoke<void>("rename_folder", {
      vaultPath,
      project,
      old: oldName,
      new: newName,
    }),
  deleteFolder: (vaultPath: string, project: string, folder: string) =>
    invoke<void>("delete_folder", { vaultPath, project, folder }),
  /** Move a domain file. `toFolder = null` (or omitted) drops it back to
   *  the project root. Returns the new domain name (may include the
   *  folder prefix). */
  moveDomain: (
    vaultPath: string,
    project: string,
    from: string,
    toFolder: string | null,
  ) =>
    invoke<string>("move_domain", {
      vaultPath,
      project,
      from,
      toFolder,
    }),
  /** Install (or refresh) a per-project Danbi skill. Lands at
   *  `~/.claude/skills/danbi-<slug>/SKILL.md` with the project's scoped
   *  MCP endpoint baked in. Returns the absolute path. */
  installSkill: (project: string) =>
    invoke<string>("install_skill", { project }),
  /** True if THIS project's skill file exists. */
  skillStatus: (project: string) =>
    invoke<boolean>("skill_status", { project }),
  trashList: () => invoke<TrashEntry[]>("trash_list"),
  trashRestore: (id: string) =>
    invoke<TrashEntry>("trash_restore", { id }),
  trashPurge: (id: string) => invoke<void>("trash_purge", { id }),
  trashEmpty: () => invoke<number>("trash_empty"),
  readDoc: (vaultPath: string, project: string, domain: string) =>
    invoke<string>("read_doc", { vaultPath, project, domain }),
  writeDoc: (
    vaultPath: string,
    project: string,
    domain: string,
    content: string,
  ) => invoke<void>("write_doc", { vaultPath, project, domain, content }),
  saveAsset: (
    vaultPath: string,
    project: string,
    filename: string,
    bytes: Uint8Array,
  ) =>
    invoke<string>("save_asset", {
      vaultPath,
      project,
      filename,
      bytesB64: uint8ArrayToBase64(bytes),
    }),
  resolveAsset: (vaultPath: string, project: string, relPath: string) =>
    invoke<string>("resolve_asset", { vaultPath, project, relPath }),
  startWatching: (vaultPath: string) =>
    invoke<void>("start_watching", { vaultPath }),
  stopWatching: () => invoke<void>("stop_watching"),

  routeMessage: (
    message: string,
    ctx: RoutingContext,
    attachments?: Attachment[],
  ) =>
    invoke<RoutingResult>("route_message", {
      message,
      ctx,
      attachments: attachments ?? null,
    }),
  previewPlan: (params: {
    message: string;
    project: string;
    domain: string;
    intent: Intent;
    attachments?: Attachment[];
  }) =>
    invoke<PlanPreview>("preview_plan", {
      message: params.message,
      project: params.project,
      domain: params.domain,
      intent: params.intent,
      attachments: params.attachments ?? null,
    }),
  extractFilePath: (path: string) =>
    invoke<Extracted>("extract_file_path", { path }),
  extractFileBytes: (filename: string, bytes: Uint8Array) =>
    invoke<Extracted>("extract_file_bytes", {
      filename,
      bytesB64: uint8ArrayToBase64(bytes),
    }),
  applyPlan: (params: {
    project: string;
    domain: string;
    intent: Intent;
    userMessage: string;
    summary: string;
    op: EditOp;
  }) =>
    invoke<ApplyResult>("apply_plan", {
      project: params.project,
      domain: params.domain,
      intent: params.intent,
      userMessage: params.userMessage,
      summary: params.summary,
      op: params.op,
    }),
  undoLast: () => invoke<string | null>("undo_last"),
  recentCommits: (limit?: number) =>
    invoke<CommitSummary[]>("recent_commits", { limit: limit ?? 100 }),
  buildLinkIndex: () => invoke<LinkIndex>("build_link_index"),
  vaultSuggestions: () => invoke<VaultSuggestion[]>("vault_suggestions"),

  quickCapture: (
    message: string,
    project?: string,
    domain?: string,
  ) =>
    invoke<QuickCaptureResult>("quick_capture", {
      message,
      project: project ?? null,
      domain: domain ?? null,
    }),
  toggleCapture: () => invoke<void>("toggle_capture"),
  hideCapture: () => invoke<void>("hide_capture"),
  resizeCapture: (height: number) =>
    invoke<void>("resize_capture", { height }),
  captureOpenHit: (project: string, domain: string) =>
    invoke<void>("capture_open_hit", { project, domain }),
  summarizeDaily: (project: string, domain: string) =>
    invoke<{
      summary_md: string;
      html: string;
      provider: string;
      model: string;
      export_id: string;
    }>("summarize_daily", { project, domain }),
  exportDocHtml: (project: string, domain: string) =>
    invoke<{ html: string; export_id: string }>("export_doc_html", {
      project,
      domain,
    }),
  openHtmlPreview: (html: string, title: string) =>
    invoke<string>("open_html_preview", { html, title }),
  saveHtmlToPath: (path: string, html: string) =>
    invoke<void>("save_html_to_path", { path, html }),
  listExports: (project?: string, sourceDomain?: string) =>
    invoke<ExportRecord[]>("list_exports", {
      project: project ?? null,
      sourceDomain: sourceDomain ?? null,
    }),
  openExport: (id: string) => invoke<string>("open_export", { id }),
  composePurposeSchema: (project: string, kind: "purpose" | "schema") =>
    invoke<{ markdown: string; provider: string; model: string }>(
      "compose_purpose_schema",
      { project, kind },
    ),
  applyCaptureShortcut: (accelerator: string) =>
    invoke<void>("apply_capture_shortcut", { accelerator }),
  validateShortcut: (accelerator: string) =>
    invoke<void>("validate_shortcut", { accelerator }),
  captureContext: () => invoke<CaptureContext>("capture_context"),
  searchVault: (query: string) =>
    invoke<SearchResponse>("search_vault", { query }),
  searchLocal: (query: string, limit?: number) =>
    invoke<SearchHit[]>("search_local", { query, limit: limit ?? 10 }),
  searchFull: (query: string, limit?: number) =>
    invoke<SearchHit[]>("search_full", { query, limit: limit ?? 20 }),
  compoundPreview: (params: {
    topic: string;
    project: string;
    targetDomain: string;
    maxSources?: number;
  }) =>
    invoke<CompoundPreview>("compound_preview", {
      topic: params.topic,
      project: params.project,
      targetDomain: params.targetDomain,
      maxSources: params.maxSources ?? null,
    }),
  compoundApply: (params: {
    project: string;
    domain: string;
    draft: string;
    summary: string;
    userMessage: string;
  }) =>
    invoke<ApplyResult>("compound_apply", {
      project: params.project,
      domain: params.domain,
      draft: params.draft,
      summary: params.summary,
      userMessage: params.userMessage,
    }),
  dailySnapshot: () => invoke<DailySnapshot>("daily_snapshot"),
  ensureTodayNote: (project: string) =>
    invoke<string>("ensure_today_note", { project }),
  listTemplates: () => invoke<VaultTemplate[]>("list_templates"),
  applyTemplate: (vaultPath: string, templateId: string) =>
    invoke<void>("apply_template", { vaultPath, templateId }),

  mcpStatus: () => invoke<McpStatus>("mcp_status"),
  mcpEnable: (port?: number) =>
    invoke<McpStatus>("mcp_enable", { port: port ?? null }),
  mcpDisable: () => invoke<McpStatus>("mcp_disable"),
  mcpRotateToken: () => invoke<McpStatus>("mcp_rotate_token"),
  mcpProjectEndpoint: (project: string) =>
    invoke<McpProjectEndpoint>("mcp_project_endpoint", { project }),

  ghostList: (project: string) =>
    invoke<GhostStore>("ghost_list", { project }),
  ghostScan: (project: string) =>
    invoke<GhostStore>("ghost_scan", { project }),
  ghostAccept: (project: string, id: string) =>
    invoke<GhostStore>("ghost_accept", { project, id }),
  ghostReject: (project: string, id: string) =>
    invoke<GhostStore>("ghost_reject", { project, id }),

  goalsList: (project: string, includeArchived = false) =>
    invoke<Goal[]>("goals_list", { project, includeArchived }),
  goalsAdd: (project: string, title: string, note?: string | null) =>
    invoke<Goal>("goals_add", { project, title, note: note ?? null }),
  goalsEdit: (
    project: string,
    id: string,
    patch: { title?: string; note?: string | null },
  ) => {
    const args: Record<string, unknown> = {
      project,
      id,
      title: patch.title ?? null,
      note: null,
      clearNote: false,
    };
    if (patch.note === null) {
      args.clearNote = true;
    } else if (patch.note !== undefined) {
      args.note = patch.note;
    }
    return invoke<Goal>("goals_edit", args);
  },
  goalsArchive: (project: string, id: string) =>
    invoke<Goal>("goals_archive", { project, id }),
  goalsUnarchive: (project: string, id: string) =>
    invoke<Goal>("goals_unarchive", { project, id }),
  goalsDelete: (project: string, id: string) =>
    invoke<void>("goals_delete", { project, id }),

  projectActivityOverview: (days = 30) =>
    invoke<ActivityOverview>("project_activity_overview", { days }),
  openProjectInMain: (project: string) =>
    invoke<void>("open_project_in_main", { project }),
  quitApp: () => invoke<void>("quit_app"),

  projectQaAsk: (project: string, question: string) =>
    invoke<QaAnswer>("project_qa_ask", { project, question }),
  projectBriefing: (project: string, range: "today" | "yesterday" | "last_week") =>
    invoke<BriefingResult>("project_briefing", { project, range }),
  dashboardSnapshot: () => invoke<DashboardSnapshot>("dashboard_snapshot"),

  dashboardMcpInbound: (range: McpInboundRange) =>
    invoke<McpVaultSummary>("dashboard_mcp_inbound", { range }),
  dashboardMcpInboundProject: (project: string, range: McpInboundRange) =>
    invoke<McpProjectDetail>("dashboard_mcp_inbound_project", { project, range }),
  dashboardMcpInboundDomain: (
    project: string,
    domain: string,
    range: McpInboundRange,
  ) =>
    invoke<McpDomainDetail>("dashboard_mcp_inbound_domain", {
      project,
      domain,
      range,
    }),

  usageExportJson: (path: string) =>
    invoke<void>("usage_export_json", { path }),
  usageExportCsv: (path: string) =>
    invoke<void>("usage_export_csv", { path }),
  usageRetentionSweep: () => invoke<number>("usage_retention_sweep"),
  usageSetMcpTracking: (enabled: boolean) =>
    invoke<void>("usage_set_mcp_tracking", { enabled }),
  usageSetMcpRetention: (days: number) =>
    invoke<number>("usage_set_mcp_retention", { days }),
  buildGraph: (project?: string) =>
    invoke<GraphData>("build_graph", { project: project ?? null }),
  backupNow: () => invoke<BackupReport>("backup_now"),
  backupValidatePath: (path: string) =>
    invoke<void>("backup_validate_path", { path }),
  projectContextStatus: (project: string) =>
    invoke<ProjectContextStatus>("project_context_status", { project }),
  projectContextEnsure: (project: string) =>
    invoke<void>("project_context_ensure", { project }),
  cacheClear: () => invoke<void>("cache_clear"),
  reviewsList: () => invoke<ReviewStore>("reviews_list"),
  reviewsResolve: (id: string, status: ReviewStatus) =>
    invoke<ReviewStore>("reviews_resolve", { id, status }),
  vectorStats: () => invoke<VectorIndexStats>("vector_stats"),
  vectorReindex: (modelId?: string, batchSize?: number) =>
    invoke<VectorReindexReport>("vector_reindex", {
      modelId: modelId ?? null,
      batchSize: batchSize ?? null,
    }),
  vectorReindexProject: (project: string, modelId?: string, batchSize?: number) =>
    invoke<VectorReindexReport>("vector_reindex_project", {
      project,
      modelId: modelId ?? null,
      batchSize: batchSize ?? null,
    }),
  vectorClear: () => invoke<void>("vector_clear"),
  vectorSearch: (query: string, limit?: number, modelId?: string) =>
    invoke<VectorHit[]>("vector_search", {
      query,
      limit: limit ?? null,
      modelId: modelId ?? null,
    }),
  usageMonthToDate: () => invoke<UsageSummary>("usage_month_to_date"),
  usageSetRate: (krwPerUsd: number) =>
    invoke<void>("usage_set_rate", { krwPerUsd }),
  vectorEstimateReindex: (modelId?: string) =>
    invoke<VectorEstimateResponse>("vector_estimate_reindex", {
      modelId: modelId ?? null,
    }),
  projectJournalView: (project: string) =>
    invoke<ProjectJournalView>("project_journal_view", { project }),
  projectUpdates: () => invoke<Record<string, number>>("project_updates"),
  projectMarkSeen: (project: string) =>
    invoke<void>("project_mark_seen", { project }),
  domainUpdates: () =>
    invoke<Record<string, "new" | "modified">>("domain_updates"),
  domainMarkSeen: (project: string, domain: string) =>
    invoke<void>("domain_mark_seen", { project, domain }),
  projectMarkAllRead: (project: string) =>
    invoke<void>("project_mark_all_read", { project }),
  vaultMarkAllRead: () => invoke<number>("vault_mark_all_read"),
  groupsSet: (groups: ProjectGroup[]) =>
    invoke<ProjectGroup[]>("groups_set", { groups }),

  hidePopover: () => invoke<void>("hide_popover"),
  openMainWindow: () => invoke<void>("open_main_window"),
  autostartStatus: () => invoke<boolean>("autostart_status"),
  autostartSet: (enabled: boolean) =>
    invoke<void>("autostart_set", { enabled }),
  trayBadgeCount: () => invoke<number>("tray_badge_count"),
  trayBadgeReset: () => invoke<void>("tray_badge_reset"),
  trayBadgeSetEnabled: (enabled: boolean) =>
    invoke<void>("tray_badge_set_enabled", { enabled }),
};

export type TriggerKind =
  | "decision"
  | "cause"
  | "todo"
  | "knowhow"
  | "pitfall"
  | "other";

export type JournalEntry = {
  date: string;
  title: string;
  kind: TriggerKind;
  preview: string;
};

export type DayCounts = {
  date: string;
  decision: number;
  cause: number;
  todo: number;
  knowhow: number;
  pitfall: number;
  other: number;
};

export type ProjectJournalView = {
  project: string;
  today: string;
  today_counts: DayCounts;
  recent_entries: JournalEntry[];
  last_7_days: DayCounts[];
  /** "daily/YYYY-MM-DD.md" → 그 파일에 등장한 trigger kind 들 (dedup,
   *  decision→other 순). 사이드바에서 파일 옆 chip 표시용. */
  daily_file_kinds: Record<string, TriggerKind[]>;
};

export type ProjectGroup = {
  id: string;
  label: string;
  projects: string[];
  collapsed: boolean;
};

export type VectorReindexEstimate = {
  total_files: number;
  fresh_files: number;
  pending_files: number;
  pending_chars: number;
  pending_tokens: number;
  model: string;
};

export type VectorEstimateResponse = {
  estimate: VectorReindexEstimate;
  krw: number;
  krw_per_usd: number;
};

export type UsageRoleSummary = {
  role: string;
  input_tokens: number;
  output_tokens: number;
  krw: number;
  top_model: string | null;
};

export type UsageSummary = {
  from_ms: number;
  to_ms: number;
  total_krw: number;
  krw_per_usd: number;
  by_role: UsageRoleSummary[];
  calls: number;
};

export type Goal = {
  id: string;
  title: string;
  note?: string | null;
  created_at: number;
  archived_at?: number | null;
};

export type ProjectActivity = {
  project: string;
  commits: number;
  mcp_calls: number;
  mcp_tokens: number;
  activity_score: number;
  last_activity_at?: number | null;
};

export type ActivityOverview = {
  days: number;
  from_ms: number;
  to_ms: number;
  total_commits: number;
  total_mcp_calls: number;
  total_mcp_tokens: number;
  by_project: ProjectActivity[];
};

export type QuickCaptureResult =
  | {
      status: "stored";
      project: string;
      domain: string;
      intent: string;
      summary: string;
      commit_after: string | null;
    }
  | {
      status: "needs_clarification";
      clarification_type: "project" | "domain" | null;
      candidate_projects: string[];
      candidate_domains: string[];
      project: string | null;
    };
