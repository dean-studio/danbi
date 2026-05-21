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
