import { useEffect, useState } from "react";
import {
  PrimaryButton,
  SecondaryButton,
  TextField,
  WizardShell,
} from "@/components/WizardShell";
import {
  ipc,
  type DanbiConfig,
  type VaultTemplate,
} from "@/lib/ipc";
import { cn } from "@/lib/utils";
import { Wordmark } from "@/components/Wordmark";

// 0.1 온보딩 흐름:
//   Welcome → (선택) AI 연동 → Vault → Template
// "AI 연동" 단계는 검색 품질을 한 단계 올려주는 임베딩 provider 를 받는
// 곳. 0.1 에서는 Google Gemini (무료 한도) + AWS Bedrock (Titan Embed v2)
// 두 옵션 노출. "나중에 설정" 으로 건너뛸 수 있고, Settings 에서 언제든
// 다시 켤 수 있다. 추후 LLM 호출 자동화 (purpose 자동 작성 등) 도
// 같은 provider 키를 재사용해서 추가될 예정.
type Step = "welcome" | "embed" | "vault" | "template";

function stepIndex(s: Step): number {
  if (s === "welcome") return 0;
  if (s === "embed") return 1;
  if (s === "vault") return 2;
  if (s === "template") return 3;
  return 0;
}

export function Onboarding({ onDone }: { onDone: (cfg: DanbiConfig) => void }) {
  // 0.1 온보딩은 Welcome → Vault → Template 3단계만 노출. LLM/credentials
  // 단계의 state 는 모두 제거됐고 cfg 빌드는 항상 provider=null 형태.
  const [step, setStep] = useState<Step>("welcome");
  const [vaultPath, setVaultPath] = useState<string>("");
  const [templates, setTemplates] = useState<VaultTemplate[]>([]);
  const [selectedTemplate, setSelectedTemplate] = useState<string>("developer");
  // 임베딩 provider 선택. "none" 이면 BM25 only 모드 (Skip 해도 단비
  // 정상 동작). "gemini" 면 무료 한도 + 키 입력. "bedrock" 이면 사용자
  // ~/.aws/credentials 또는 환경변수에서 자동 감지된 자격증명 사용.
  type EmbedKind = "none" | "gemini" | "bedrock";
  const [embedKind, setEmbedKind] = useState<EmbedKind>("none");
  const [geminiKey, setGeminiKey] = useState<string>("");
  const [bedrockRegion, setBedrockRegion] = useState<string>("us-east-1");

  useEffect(() => {
    ipc.defaultVault().then(setVaultPath).catch(() => {});
    ipc
      .listTemplates()
      .then((list) => {
        setTemplates(list);
        if (list[0]) setSelectedTemplate(list[0].id);
      })
      .catch(() => {});
  }, []);

  const total = 4;
  const stepNumber = stepIndex(step) + 1;

  function next(to: Step) {
    setStep(to);
  }

  // --- Welcome ---
  if (step === "welcome") {
    // Welcome 은 다른 step 들과 다르게 워드마크를 최상단 hero 로 두고
    // 제목·소제목·핵심 포인트는 그 아래 한 컬럼으로 흐른다.
    return (
      <WizardShell
        step={stepNumber}
        total={total}
        title=""
        footer={
          <>
            <span className="text-caption-sm text-mute">Step 1 of {total}</span>
            <PrimaryButton onClick={() => next("embed")}>
              시작하기
            </PrimaryButton>
          </>
        }
      >
        <div className="flex flex-1 flex-col items-center justify-center gap-7">
          <Wordmark className="h-14 w-auto" />
          <div className="flex flex-col items-center gap-3 text-center">
            <h1 className="text-[28px] font-semibold leading-[1.2] tracking-[-0.01em] text-ink">
              단비에 오신 것을 환영해요
            </h1>
            <p className="max-w-[560px] text-[15px] leading-[1.7] text-body">
              <span className="font-semibold text-on-dark">
                Claude Code · Codex
              </span>{" "}
              같은 AI 와 함께{" "}
              <span className="font-semibold text-on-dark">자라는</span>{" "}
              나만의{" "}
              <span className="font-semibold text-on-dark">연구실</span>이에요.
              쓰면 쓸수록 vault 가 깊어지고, AI 가 그 깊이를 알아챕니다.
            </p>
          </div>

          {/* 3개 핵심 셀링 포인트. 각 항목이 한 가지 색조로 강조돼서
              사용자가 첫 화면에서 단비 정체성을 한눈에 인지하도록.
              - yellow (warning/highlight 톤): LLM 키 불필요 — 가장 큰 안심
              - blue (accent 톤): MCP 차별점
              - green (security 톤): 로컬 보존 + git */}
          <div className="flex w-full max-w-[600px] flex-col gap-2.5">
            <div className="rounded-lg border border-accent-yellow/30 bg-accent-yellow-soft/30 px-4 py-3">
              <div className="flex items-start gap-2.5 text-[14px] leading-[1.6]">
                <span className="mt-0.5 grid h-5 w-5 shrink-0 place-items-center rounded-full bg-accent-yellow text-[11px] font-bold text-canvas">
                  ✦
                </span>
                <div className="flex-1 text-body">
                  <span className="font-semibold text-accent-yellow">
                    LLM API 키가 필수가 아니에요
                  </span>{" "}
                  — 추론은 외부 AI 가 자기 환경에서. 단비는 의미 검색·자동
                  요약 같은 옵션 기능을 켤 때만 키를 받아요.
                </div>
              </div>
            </div>
            <div className="rounded-lg border border-accent-blue/30 bg-accent-blue-soft/30 px-4 py-3">
              <div className="flex items-start gap-2.5 text-[14px] leading-[1.6]">
                <span className="mt-0.5 grid h-5 w-5 shrink-0 place-items-center rounded-full bg-accent-blue text-[11px] font-bold text-on-primary">
                  ⇆
                </span>
                <div className="flex-1 text-body">
                  <span className="font-semibold text-accent-blue">
                    MCP 서버 내장
                  </span>{" "}
                  — Claude Code · Cursor 가 vault 를 직접 읽고·쓸 수 있어요.
                  단비가 외부 AI 의 공유 뇌가 됩니다.
                </div>
              </div>
            </div>
            <div className="rounded-lg border border-accent-green/30 bg-accent-green-soft/30 px-4 py-3">
              <div className="flex items-start gap-2.5 text-[14px] leading-[1.6]">
                <span className="mt-0.5 grid h-5 w-5 shrink-0 place-items-center rounded-full bg-accent-green text-[11px] font-bold text-on-primary">
                  ✓
                </span>
                <div className="flex-1 text-body">
                  <span className="font-semibold text-accent-green">
                    데이터는 Mac 에만 머물러요
                  </span>{" "}
                  — 모든 vault 파일은 로컬에 저장되고, git 으로 자동 버전
                  관리됩니다.
                </div>
              </div>
            </div>
          </div>

          {/* AI 연동 했을 때 / 안 했을 때 차이를 미리 보여주는 카드.
              제목은 또렷하게 ink, 본문은 body 톤으로 가독성 확보. */}
          <div className="w-full max-w-[600px] rounded-xl border border-hairline bg-surface-elevated p-5 leading-[1.55]">
            <div className="mb-4 flex items-center gap-2 text-[12px] font-semibold uppercase tracking-[0.6px] text-accent-blue">
              <span className="grid h-5 w-5 place-items-center rounded-full bg-accent-blue text-[11px] font-bold text-on-primary">
                ?
              </span>
              AI 연동을 켜면 추가로 가능한 것
            </div>
            <div className="grid grid-cols-2 gap-x-6 gap-y-4 text-[13px]">
              <div>
                <div className="text-[14px] font-semibold text-ink">
                  의미 검색
                </div>
                <div className="mt-1 text-body">
                  "JWT 결정" 검색 → "토큰 만료 정책" 같은 동의어 문서까지
                </div>
              </div>
              <div>
                <div className="text-[14px] font-semibold text-ink">
                  자연어 질의
                </div>
                <div className="mt-1 text-body">
                  Claude Code 가 자연스러운 문장으로 vault 탐색해도 정확
                </div>
              </div>
              <div>
                <div className="text-[14px] font-semibold text-ink">
                  자동 요약 · HTML
                </div>
                <div className="mt-1 text-body">
                  "오늘 daily 요약 HTML 로 만들어줘" 같은 자동화
                </div>
              </div>
              <div>
                <div className="text-[14px] font-semibold text-ink">
                  관련 노트 제안
                </div>
                <div className="mt-1 text-body">
                  ghost links — vault 가 알아서 연결을 제안
                </div>
              </div>
            </div>
            <div className="mt-4 flex items-center gap-3 border-t border-hairline pt-3 text-[12px]">
              <span className="inline-flex items-center gap-1.5 text-mute">
                <span className="grid h-5 w-5 place-items-center rounded-md bg-accent-blue-soft text-[10px] font-bold text-accent-blue">
                  G
                </span>
                <span className="text-on-dark">Google Gemini</span>
                <span className="text-stone">·</span>
                <span className="text-stone">무료</span>
              </span>
              <span className="text-stone">·</span>
              <span className="inline-flex items-center gap-1.5 text-mute">
                <span className="grid h-5 w-5 place-items-center rounded-md bg-surface-card text-[9px] font-bold text-on-dark">
                  AWS
                </span>
                <span className="text-on-dark">Bedrock</span>
                <span className="text-stone">·</span>
                <span className="text-stone">Titan v2</span>
              </span>
              <span className="ml-auto text-[11px] italic text-stone">
                다음 단계에서 선택
              </span>
            </div>
          </div>
        </div>
      </WizardShell>
    );
  }

  // --- AI 연동 (선택) ---
  if (step === "embed") {
    const canProceed =
      embedKind === "none" ||
      (embedKind === "gemini" && geminiKey.trim().length > 0) ||
      (embedKind === "bedrock" && bedrockRegion.trim().length > 0);
    return (
      <WizardShell
        step={stepNumber}
        total={total}
        title="AI 연동 (선택)"
        subtitle="임베딩 provider 를 연결하면 검색이 BM25 + 의미 기반 RRF 하이브리드로 한 단계 올라갑니다. Claude Code 가 자연어로 vault 를 탐색할 때 특히 효과적이에요. 건너뛰어도 단비는 BM25 키워드 검색만으로 정상 동작합니다."
        footer={
          <>
            <SecondaryButton onClick={() => next("welcome")}>
              ← 뒤로
            </SecondaryButton>
            <div className="flex items-center gap-2">
              <SecondaryButton
                onClick={() => {
                  setEmbedKind("none");
                  setGeminiKey("");
                  next("vault");
                }}
              >
                나중에 설정
              </SecondaryButton>
              <PrimaryButton disabled={!canProceed} onClick={() => next("vault")}>
                다음
              </PrimaryButton>
            </div>
          </>
        }
      >
        <div className="flex flex-col gap-3">
          <ProviderRadio
            active={embedKind === "none"}
            onClick={() => setEmbedKind("none")}
            badge="무료 · 키 0개"
            badgeTone="green"
            title="연결 안 함 (BM25 만 사용)"
            desc="키워드 + 한국어 n-gram 검색만으로 동작합니다. 사용자가 적은 단어를 정확히 기억할 때 충분히 좋아요."
          />

          <ProviderRadio
            active={embedKind === "gemini"}
            onClick={() => setEmbedKind("gemini")}
            badge="추천 · 무료"
            badgeTone="blue"
            logo="G"
            title="Google Gemini 임베딩"
            desc="gemini-embedding-001 · 무료 한도 (RPD 제한 있음) · 카드 등록 불필요. Google 계정으로 키 발급."
          >
            {embedKind === "gemini" && (
              <div className="mt-3 flex items-center gap-2">
                <input
                  type="password"
                  value={geminiKey}
                  onChange={(e) => setGeminiKey(e.target.value)}
                  placeholder="AI…"
                  autoFocus
                  className="h-10 flex-1 rounded-md border border-hairline bg-surface px-3 font-mono text-[13px] text-ink outline-none focus:border-hairline-strong"
                />
                <a
                  href="https://aistudio.google.com/app/apikey"
                  target="_blank"
                  rel="noreferrer"
                  className={cn(
                    "inline-flex h-10 items-center rounded-md border border-hairline bg-surface px-3 text-[12px] text-body",
                    "hover:border-hairline-strong hover:text-on-dark",
                  )}
                >
                  키 발급 ↗
                </a>
              </div>
            )}
          </ProviderRadio>

          <ProviderRadio
            active={embedKind === "bedrock"}
            onClick={() => setEmbedKind("bedrock")}
            badge="기업 친화"
            badgeTone="stone"
            logo="AWS"
            title="AWS Bedrock (Titan Embed v2)"
            desc="amazon.titan-embed-text-v2 · ~/.aws/credentials 또는 AWS_* 환경변수의 자격증명을 자동 감지합니다. 별도 키 입력 없이 region 만 지정하세요."
          >
            {embedKind === "bedrock" && (
              <div className="mt-3 flex items-center gap-2">
                <span className="text-[12px] text-mute">Region</span>
                <input
                  type="text"
                  value={bedrockRegion}
                  onChange={(e) => setBedrockRegion(e.target.value)}
                  placeholder="us-east-1"
                  autoFocus
                  className="h-10 w-[160px] rounded-md border border-hairline bg-surface px-3 font-mono text-[13px] text-ink outline-none focus:border-hairline-strong"
                />
                <span className="text-[11px] text-stone">
                  자격증명은 단비에 저장하지 않아요 — AWS SDK 가 자동 로드.
                </span>
              </div>
            )}
          </ProviderRadio>

          <p className="mt-1 text-center text-[12px] text-stone">
            추후 LLM 자동화 (purpose 자동 작성 등) 가 도입되면 같은 provider 키를 재사용합니다.
          </p>
        </div>
      </WizardShell>
    );
  }

  // --- Vault ---
  if (step === "vault") {
    return (
      <WizardShell
        step={stepNumber}
        total={total}
        title="Vault 위치"
        subtitle="프로젝트 마크다운과 설정이 저장될 폴더입니다. 나중에 바꿀 수 있어요."
        footer={
          <>
            <SecondaryButton onClick={() => next("embed")}>
              ← 뒤로
            </SecondaryButton>
            <PrimaryButton
              disabled={!vaultPath}
              onClick={() => next("template")}
            >
              다음
            </PrimaryButton>
          </>
        }
      >
        <TextField
          label="Vault Path"
          value={vaultPath}
          onChange={setVaultPath}
          monospace
        />
        <div className="mt-3 text-[12px] leading-[1.5] text-mute">
          폴더가 없으면 자동으로 생성됩니다.
        </div>
      </WizardShell>
    );
  }

  // --- Template ---
  if (step === "template") {
    const sel = templates.find((t) => t.id === selectedTemplate);
    return (
      <WizardShell
        step={stepNumber}
        total={total}
        title="시작 템플릿"
        subtitle="Vault를 어떻게 쓰실 건가요? 기본 도메인과 샘플 프로젝트를 자동으로 세팅합니다."
        footer={
          <>
            <SecondaryButton onClick={() => next("vault")}>← 뒤로</SecondaryButton>
            <PrimaryButton
              disabled={!vaultPath || !sel}
              onClick={async () => {
                if (!sel) return;
                // 사용자가 embed step 에서 Gemini 키를 입력했으면 Keychain
                // 에 저장하고 cfg.embed_* 를 채운다. 키 저장은 cfg 쓰기 전에
                // 끝나야 — config 안의 keychain ref 가 빈 키를 가리키지
                // 않게.
                let embedProvider: DanbiConfig["embed_provider"] = null;
                let embedModel: string | null = null;
                if (embedKind === "gemini" && geminiKey.trim()) {
                  try {
                    await ipc.storeGoogleApiKey(geminiKey.trim());
                    embedProvider = {
                      kind: "google",
                      api_key_ref: "keychain:danbi-google",
                    };
                    embedModel = "gemini-embedding-001";
                  } catch (e) {
                    console.error("[danbi] gemini key store failed", e);
                  }
                } else if (embedKind === "bedrock" && bedrockRegion.trim()) {
                  // AWS 자격증명은 SDK 가 ~/.aws/credentials 또는 환경변수
                  // 에서 자동 로드한다. 단비 cfg 에는 region 만 적어둔다.
                  // auth_mode = "env" 가 디폴트 패턴 — 사용자가 더 정밀히
                  // profile 지정하려면 Settings 의 임베딩 패널에서 변경.
                  embedProvider = {
                    kind: "bedrock",
                    auth_mode: "env",
                    profile: null,
                    region: bedrockRegion.trim(),
                  };
                  embedModel = "amazon.titan-embed-text-v2:0";
                }

                // 재실행 시 기존 cfg 의 group/theme/shortcuts/MCP token/
                // backup 등은 보존 — spread + LLM/embed 필드만 패치.
                const existing = await ipc.loadConfig().catch(() => null);
                const cfg: DanbiConfig = existing
                  ? {
                      ...existing,
                      vault_path: vaultPath,
                      provider: null,
                      models: { routing: null, writer: null },
                      embed_provider: embedProvider,
                      embed_model: embedModel,
                      preset: "claude_code",
                      default_domains:
                        sel.default_domains.length > 0
                          ? sel.default_domains
                          : existing.default_domains,
                      default_folders:
                        sel.default_folders ?? existing.default_folders,
                    }
                  : {
                      version: 2,
                      vault_path: vaultPath,
                      provider: null,
                      models: { routing: null, writer: null },
                      embed_provider: embedProvider,
                      embed_model: embedModel,
                      preset: "claude_code",
                      projects: [],
                      default_domains:
                        sel.default_domains.length > 0
                          ? sel.default_domains
                          : ["ui.md", "backend.md", "plan.md"],
                      default_folders: sel.default_folders ?? ["daily"],
                      appearance: { theme: "dark", compact: false },
                      editor: { autosave: false, word_wrap: true },
                      // 처음 설치 직후엔 글로벌 단축키 OFF — macOS 한국어
                      // IME 토글 등 시스템 단축키와 충돌. 사용자가 Settings
                      // 에서 직접 등록할 때만 활성화.
                      shortcuts: { quick_capture: "" },
                      capture: { last_project: null, last_domain: null },
                      backup: {
                        enabled: false,
                        path: null,
                        debounce_ms: 5000,
                        exclude: [".git", ".danbi", ".DS_Store"],
                        last_run_at: null,
                        last_message: null,
                      },
                      mcp: { enabled: true, port: 47921, token: "" },
                    };
                await ipc.saveConfig(vaultPath, cfg);
                try {
                  await ipc.applyTemplate(vaultPath, sel.id);
                } catch (e) {
                  console.error("[danbi] applyTemplate failed", e);
                }
                onDone(cfg);
              }}
            >
              완료
            </PrimaryButton>
          </>
        }
      >
        <div className="grid grid-cols-2 gap-2">
          {templates.map((t) => {
            const active = t.id === selectedTemplate;
            return (
              <button
                key={t.id}
                onClick={() => setSelectedTemplate(t.id)}
                className={cn(
                  "flex flex-col items-stretch rounded-md border p-3 text-left transition-colors",
                  active
                    ? "border-accent-blue bg-accent-blue-soft"
                    : "border-hairline bg-surface hover:border-hairline-strong",
                )}
              >
                <div className="flex items-center justify-between">
                  <span
                    className={cn(
                      "text-[13px] font-medium",
                      active ? "text-on-dark" : "text-ink",
                    )}
                  >
                    {t.name}
                  </span>
                  {active && (
                    <span className="rounded-xs bg-accent-blue-soft px-1 py-0.5 text-[10px] font-medium uppercase tracking-[0.4px] text-accent-blue">
                      선택
                    </span>
                  )}
                </div>
                <div className="mt-1 text-caption-sm leading-[1.5] text-body">
                  {t.description}
                </div>
                {t.default_domains.length > 0 && (
                  <div className="mt-2 flex flex-wrap gap-1">
                    {t.default_domains.slice(0, 4).map((d) => (
                      <span
                        key={d}
                        className="inline-flex items-center rounded-xs bg-surface-elevated px-1.5 py-0.5 font-mono text-[10px] text-on-dark-mute"
                      >
                        {d}
                      </span>
                    ))}
                    {t.default_domains.length > 4 && (
                      <span className="text-[11px] text-mute">
                        +{t.default_domains.length - 4}
                      </span>
                    )}
                  </div>
                )}
                {t.sample_project && (
                  <div className="mt-1 text-[12px] text-mute">
                    샘플 프로젝트: {t.sample_project.name}
                  </div>
                )}
              </button>
            );
          })}
        </div>
      </WizardShell>
    );
  }

  return null;
}


/** Reusable radio-card row used in the AI 연동 step. Visually behaves
 *  like the PresetCard from earlier wizards but slimmer — content can
 *  spill below (e.g. for the conditional API-key input field). */
function ProviderRadio({
  active,
  onClick,
  title,
  desc,
  badge,
  badgeTone,
  logo,
  children,
}: {
  active: boolean;
  onClick: () => void;
  title: string;
  desc: string;
  badge?: string;
  badgeTone?: "green" | "blue" | "stone" | "yellow";
  logo?: string;
  children?: React.ReactNode;
}) {
  const toneCls =
    badgeTone === "green"
      ? "bg-accent-green-soft text-accent-green"
      : badgeTone === "blue"
        ? "bg-accent-blue-soft text-accent-blue"
        : badgeTone === "yellow"
          ? "bg-accent-yellow-soft text-accent-yellow"
          : "bg-surface-card text-mute";
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "flex flex-col rounded-lg border p-4 text-left transition-colors",
        active
          ? "border-accent-blue bg-accent-blue-soft/40"
          : "border-hairline bg-surface-elevated hover:border-hairline-strong",
      )}
    >
      <div className="flex items-start gap-3">
        {logo && (
          <span className="grid h-8 w-8 shrink-0 place-items-center rounded-md bg-surface-card text-[12px] font-semibold text-on-dark">
            {logo}
          </span>
        )}
        <div className="flex-1">
          <div className="flex items-center gap-2">
            <span className="text-[14px] font-medium text-ink">{title}</span>
            {badge && (
              <span
                className={cn(
                  "rounded-xs px-1.5 py-0.5 text-[10px] uppercase tracking-[0.4px]",
                  toneCls,
                )}
              >
                {badge}
              </span>
            )}
          </div>
          <div className="mt-1 text-[12.5px] leading-[1.55] text-mute">
            {desc}
          </div>
        </div>
      </div>
      {children && <div onClick={(e) => e.stopPropagation()}>{children}</div>}
    </button>
  );
}
