<p align="center">
  <img src="src/assets/danbi-app-icon.png" alt="Danbi" width="120" height="120" />
</p>

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="src/assets/danbi-wordmark-dark.png">
    <img src="src/assets/danbi-wordmark-light.png" alt="Danbi" width="220">
  </picture>
</p>

<p align="center">
  Claude Code · Codex 같은 AI 와 함께 자라는 나만의 연구실.<br/>
  macOS 데스크톱 앱. 모든 데이터는 로컬에. AI 키 없이도 80%, 키 한 줄로 100%.
</p>

<p align="center">
  <a href="https://github.com/dean-studio/danbi/releases/latest"><img src="https://img.shields.io/badge/download-DMG-blue" alt="download" /></a>
  <a href="https://github.com/dean-studio/danbi/releases"><img src="https://img.shields.io/badge/macOS-11%2B-black" alt="macOS 11+" /></a>
  <img src="https://img.shields.io/badge/license-All%20rights%20reserved-lightgrey" alt="license" />
  <img src="https://img.shields.io/badge/version-0.2.0-success" alt="version 0.2.0" />
</p>

---

## 한 마디로

세 가지 도구가 표면상 비슷해 보이지만 정체성이 다릅니다:

- **옵시디언** — 사용자가 vault 의 주인. 직접 채우고 직접 연결.
- **Raycast** — 사용자가 명령의 주인. 도구로 가는 통로.
- **단비** — **AI 가 vault 의 주인이고, 사용자가 그 결과를 정밀하게 통제**.

단비는 Claude Code · Codex · Cursor 같은 외부 AI 에이전트가 vault 를 직접 읽고 쓰도록 MCP 서버를 내장합니다. 사용자가 단비 앱을 안 열어둬도 AI 가 일하는 동안 vault 는 알아서 쌓이고 검색됩니다.

## 핵심 가치

- 🟡 **LLM API 키가 필수가 아니에요** — 추론은 외부 AI 가 자기 환경에서. 단비는 의미 검색·자동 요약 같은 옵션 기능을 켤 때만 키를 받습니다.
- 🔵 **MCP 서버 내장** — Claude Code · Cursor 가 vault 를 직접 읽고·씁니다. 단비가 외부 AI 의 공유 뇌가 됩니다.
- 🟢 **데이터는 Mac 에만 머물러요** — 모든 vault 파일은 로컬에 저장되고, git 으로 자동 버전 관리됩니다.

---

## 원리 — 한눈에

```
┌──────────────────────┐         ┌──────────────────────────────┐
│  외부 AI 에이전트     │   MCP    │  단비 (로컬 macOS 앱)         │
│  Claude Code/Cursor  │ ───────▶ │                              │
│  Codex · 직접 curl   │  HTTP    │  ┌────────────────────────┐  │
└──────────────────────┘          │  │ axum MCP 서버          │  │
            ▲                     │  │ (프로젝트별 endpoint)   │  │
            │                     │  └─────────┬──────────────┘  │
            │ 8KB 청크만 전달      │            │                  │
            └─────────────────────┤  ┌─────────▼──────────────┐  │
                                  │  │ tantivy BM25 + n-gram   │  │
                                  │  │ (+ Gemini 임베딩 옵션)  │  │
                                  │  └─────────┬──────────────┘  │
                                  │            │                  │
                                  │  ┌─────────▼──────────────┐  │
                                  │  │ ~/Danbi_Vault (git)     │  │
                                  │  │  Projects/<name>/*.md   │  │
                                  │  └─────────────────────────┘  │
                                  └──────────────────────────────┘
```

작동 흐름:

1. **AI 가 묻는다** — Claude Code 가 `mcp__danbi__danbi_search` 같은 tool 을 호출.
2. **단비가 좁힌다** — 로컬 BM25 인덱스로 vault 전체에서 관련 청크만 8개 추림.
3. **8KB 만 돌려준다** — vault 가 100MB 라도 LLM 컨텍스트엔 8KB 만 들어감.
4. **AI 가 답하고, 단비가 다시 적는다** — `danbi_log`/`danbi_append` 로 결정·
   TODO 가 daily 노트에 누적. 다음 세션이 이걸 다시 읽음.

세 가지 기술 결정이 이 구조를 가능하게 합니다:

- **MCP 서버를 앱에 내장** — Claude Code 가 단비를 자기 도구처럼 부른다.
- **BM25 를 로컬에서** — 임베딩 호출 없이 즉시 청크를 추리니 검색은 공짜 + 빠름.
- **Vault = git repo** — 모든 변경이 자동 커밋. 어제로 돌아갈 수 있음.

단비 자체는 *추론* 을 하지 않습니다. AI 는 자기 환경 (Claude · Gemini 등) 에서
돌고, 단비는 그 AI 의 **기억과 검색만 담당** 합니다. 그래서 키 없이도
80% 가 작동하고, 키를 넣어도 그 키는 OS Keychain 에만 머무릅니다.

---

## 왜 단비? — 토큰을 아끼고, 컨텍스트가 죽지 않는 이유

LLM 과 일하면서 가장 비싼 건 매번 같은 맥락을 다시 설명하는 일입니다.
단비는 *맥락을 vault 에 적어두고, 필요한 조각만 꺼내 던지는* 4단계로 그
비용을 깎습니다.

### 1️⃣ 프로젝트마다 독립된 MCP endpoint + SKILL

프로젝트를 만들면 단비가 자동으로:

- 그 프로젝트 전용 **MCP endpoint** 를 띄우고
- 그 프로젝트 전용 **SKILL.md** 를 `~/.claude/skills/danbi-<slug>/` 에 설치합니다

Claude Code · Cursor 는 이 endpoint 에 붙어서 *해당 프로젝트만* 읽고·씁니다.
다른 프로젝트의 노이즈가 컨텍스트에 섞이지 않으니, 매 턴 보내는 토큰이
줄어듭니다.

### 2️⃣ 세션 컨텍스트가 리셋돼도 단비가 기억합니다

LLM 의 컨텍스트 윈도우는 새 세션마다 비워집니다. 단비 vault 는 영속이고,
중요한 결정·버그 원인·TODO 가 매번 `danbi_log` 로 daily 노트에 누적됩니다.

> 다음 날 Claude Code 가 새로 켜져도 → `danbi_briefing` 한 번 →
> "어제 무슨 결정 했고, 어디까지 했고, 다음 무엇을 해야 하는지" **즉시 회복**.

긴 시스템 프롬프트로 매번 복기시키지 않아도 되니, 첫 턴부터 토큰이 짧습니다.

### 3️⃣ BM25 + 한국어 n-gram 검색 = 가장 큰 토큰 절약

큰 vault 를 통째로 LLM 에 넣으면 비싸고 느립니다. 단비는 [tantivy](https://github.com/quickwit-oss/tantivy)
기반 BM25 + 한국어 n-gram 인덱스로:

- 수백만 글자 vault → 관련 청크 8개만 추려서 LLM 에 전달
- 별도 임베딩 인덱스 / 비용 없음 — **로컬에서 즉시**
- AI 가 "X 결정 어떻게 했지?" 라고 물으면 단비가 8KB 정도만 던지고 답이 옵니다

### 4️⃣ (옵션) Gemini embedding 으로 의미 검색까지

`gemini-embedding-001` 은 **무료**. 한 번 등록하면 단비가 BM25 결과와 의미
검색 결과를 [RRF 하이브리드](https://en.wikipedia.org/wiki/Reciprocal_rank_fusion)
로 합쳐서 던집니다.

- "JWT 결정" 검색 → "토큰 만료 정책" 같은 동의어 노트까지 잡힘
- 동의어를 찾기 위해 LLM 을 부르지 않으니 검색 비용 0

### 📊 실제 측정 — 2026-05-21 (오늘 하루)

같은 컨트리뷰터의 같은 노트북에서, **단비 MCP 를 한 번이라도 호출한 세션 (ON)**
과 **호출하지 않은 세션 (OFF)** 을 자동 분류해 비교한 수치입니다.
26개 세션, 총 1,189 턴, Claude Opus 4.7 기준.

| 모드 | 세션 | 턴 | input | cache_read | output | wall | 추정 비용 |
|---|---:|---:|---:|---:|---:|---:|---:|
| 🟢 단비 ON  |  2 | 1,008 | 1,780 | 112M | 577K | 11h23m | **$251.99** |
| ⚪ 단비 OFF | 24 | 1,543 | 6,918 | 193M | 1,078K | 30h11m | **$505.52** |

| 비율 (ON ÷ OFF) | 결과 | 의미 |
|---|---:|---|
| 입력 토큰 (턴당) | **0.39×** | 단비 ON 세션은 같은 작업에 입력 토큰 **약 1/2.5** 만 씁니다 |
| Wall-clock (턴당) | **0.58×** | 응답 받기까지 시간도 **42% 단축** |
| 추정 비용 | **0.50×** | 같은 양의 작업에 **비용 절반** |

> 측정 도구는 `scripts/token-stats.py` 로 누구나 자기 환경에서 재현할 수
> 있습니다. 결과는 `Vault/Projects/단비/stats/token-usage/<날짜>.md` 에
> 누적됩니다. (단비 OFF 세션은 다른 동시 진행 사이드 프로젝트 — 이 측정은
> "단비를 쓸 때 / 안 쓸 때" 의 *해당 사용자의 실제 작업 분포* 비교입니다.)

**핵심:** 단비 자체는 아무 LLM 호출도 하지 않습니다. 토큰을 아끼는 건
"외부 AI 에이전트에게 vault 의 *필요한 조각만* 골라 던지는" 구조 덕분입니다.

---

## 기능

### 키 0개로도 되는 것

- 마크다운 에디터 (BlockNote)
- BM25 + 한국어 n-gram 키워드 검색
- 그래프 뷰 (Louvain community + ghost suggestions)
- Wiki-link 클릭 (`[[notes/foo.md]]` ↔ 다른 도메인)
- Quick Capture (⌃Space) — 글로벌 단축키로 어디서든 메모
- 프로젝트별 MCP endpoint + SKILL 파일 자동 생성
- Git 자동 커밋 (저장 = 커밋)
- 휴지통 (30일 자동 만료)
- daily 노트 + Reminiscence (1주·1달·1년 전)

### AI 연동 켜면 추가

옵션 — Google Gemini API 키 (무료) 또는 AWS Bedrock (자격증명 자동 감지) 한 번 등록하면:

- **의미 검색** — RRF 하이브리드. "JWT 결정" 검색 → "토큰 만료 정책" 같은 동의어 문서까지
- **자연어 질의** — Claude Code 가 자연스러운 문장으로 vault 탐색해도 정확
- **Daily 노트 요약 + HTML 추출** — 한 클릭으로 카드형 공유 페이지
- **purpose / schema 자동 작성** — vault 의 다른 노트들 grounding 으로 자동 채움
- **관련 노트 제안 (ghost links)** — vault 가 알아서 연결을 제안

LLM 자동화는 모두 사용자 명시 트리거만 — 백그라운드 자동 호출 없음. 진행은 화면 우상단 toast + 사이드바 + 종 아이콘 알림 셋 다에서 노출.

## 설치

1. [Releases](https://github.com/dean-studio/danbi/releases) 에서 최신 `Danbi_*.dmg` 다운로드 (Apple Silicon)
2. DMG 더블클릭 → `단비.app` 을 Applications 폴더로 드래그
3. Launchpad 에서 단비 실행 → 4단계 마법사 (Welcome → AI 연동 → Vault → Template)

요구사항: macOS 11+ (Apple Silicon — Intel 빌드는 추후)

## 첫 5분

상세 가이드는 [notes/getting-started.md](https://github.com/dean-studio/danbi/blob/main/docs/getting-started.md) 에. 요약:

1. 첫 프로젝트 만들기 (사이드바 +)
2. 사이드바에서 프로젝트 우클릭 → "Claude Code 설치 명령 복사"
3. 터미널에 붙여넣기
4. 이제 Claude Code 에 "단비 vault 에 기록해줘" / "X 프로젝트 보고 답해줘" 시킬 수 있음

## 자주 쓰는 단축키

| 단축키 | 동작 |
|---|---|
| ⌃Space | Quick Capture |
| ⌘K | 검색 |
| ⌘G | 그래프 |
| ⌘, | 설정 |
| ⌘S | 저장 |
| ⌘F | 노트 안 검색 |

## 기술

- Tauri v2 (Rust 백엔드 + macOS native)
- Vite + React + TypeScript + Tailwind + shadcn/ui
- BlockNote 에디터
- tantivy BM25 검색
- 자체 JSON 벡터 스토어 + Gemini / Bedrock embedding
- MCP HTTP 서버 (axum) + 프로젝트별 scoped endpoint
- Git 기반 vault 버전 관리

## 문서

- [getting-started](docs/getting-started.md) — 5분 입문
- [ai-integration](docs/ai-integration.md) — Gemini / Bedrock 설정
- [troubleshooting](docs/troubleshooting.md) — 자주 보는 에러
- [comparison](docs/comparison-obsidian-raycast.md) — 옵시디언 / Raycast 와 어떻게 다른가

## 빌드 (개발자)

```bash
# 개발 모드
pnpm install
pnpm tauri dev

# DMG 빌드 (Apple Developer 인증서 필요)
pnpm bundle
```

자세한 출시 절차는 [release-checklist](docs/release-checklist.md) 참조.

## 라이선스

© 2026 [Dean Works inc.](https://dean.kr) — All rights reserved.

소스 코드 열람 목적으로 공개합니다. 다음 행위는 허용되지 않습니다:

- 코드 또는 자산의 복제 · 재배포 · 수정 후 배포
- fork 후 자체 빌드물의 공개 배포
- 상업적 사용 또는 파생 제품 제작

버그 리포트 · 토론 · pull request 는 환영합니다. 라이선스 정책은 향후
별도 LICENSE 파일로 명시될 수 있습니다.
