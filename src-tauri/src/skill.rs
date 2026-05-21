//! Per-project Claude Code SKILL.md template.
//!
//! The template lives in vault at `<project>/SKILL.md` so the user can
//! edit it directly through the sidebar. `{{PROJECT}}` and `{{MCP_URL}}`
//! are substituted at install time (see `commands::install_skill`).

pub const DEFAULT_SKILL_TEMPLATE: &str = r#"---
name: danbi-{{PROJECT}}
description: Use this skill when the user is working in the "{{PROJECT}}" project's Danbi vault. Triggers include past decisions ("어떻게 정했지", "예전에"), past debugging ("이 에러 본 적", "또 떴어"), session continuity ("어제 뭐 했지", "이어서"), or saving knowledge worth remembering. Also use when the user mentions "단비", "vault", "daily note", "{{PROJECT}}", or wiki-link cross-references. Reads run vault-wide; writes auto-clamp to "{{PROJECT}}".
---

# Danbi vault — "{{PROJECT}}" 프로젝트의 영구 기억

이 단비 vault 는 **"{{PROJECT}}"** 프로젝트의 결정·디버깅·TODO·학습이 누적되는 외부 장기 기억이다. 다른 프로젝트의 노트도 검색은 가능하지만, 쓰기는 모두 이 프로젝트로 자동 clamp 된다.

MCP endpoint: `{{MCP_URL}}` (이미 등록돼 있으면 무시. 안 등록돼 있으면 단비 사이드바 → 프로젝트 우클릭 → "Claude Code 설치 명령 복사" 로 한 줄 받아 실행.)


# Danbi vault — Claude Code 의 영구 기억

이 단비 vault 는 사용자의 모든 프로젝트가 누적되는 **외부 장기 기억**이다. 결정·디버깅·TODO·학습이 세션을 넘어 쌓인다.

## 핵심 원칙

1. **답변 전에 단비부터 물어본다** — 일반론 대신 vault 의 맥락으로 답한다.
2. **중요한 건 즉시 기록한다** — 기술 결정·버그 원인·TODO 는 `danbi_log` 로 바로.
3. **쓸 때는 연결한다** — 관련 문서를 `[[wiki-link]]` 로 걸어 그래프에 누적시킨다.

## 도구 (모든 도구는 `mcp__danbi__` 접두사로 노출됨)

**읽기** (vault 전체 조회 가능):
- `danbi_briefing` — 최근 활동·ghost 제안·고아 파일·daily 노트를 한 JSON 으로
- `danbi_recent` — 최근 수정된 문서 목록
- `danbi_search` — tantivy 전문 검색 + Gemini 임베딩 RRF (한국어/영어 cross)
- `danbi_read` — 특정 프로젝트/도메인 파일 전체
- `danbi_daily` — 오늘 + 1주/1달/1년 전 daily 노트
- `danbi_list_projects` — vault 전체 프로젝트·도메인 트리

**쓰기** (현재 cwd 의 프로젝트로 자동 clamp — 다른 프로젝트로 잘못 쓸 위험 0):
- `danbi_log` — 오늘 daily 노트에 append. project 파라미터 **생략**.
- `danbi_append` — 임의 도메인 파일에 append (없으면 자동 생성). project 파라미터 **생략**.
- `danbi_create_folder` — 1~2단계 sub-folder 생성. 카테고리·시기별 누적용.
- `danbi_create_file` — 폴더+파일+내용을 한 번에 보장. 매일 통계 자동 기록 같은 자동화에 적합.

## Read — 언제 무엇을 읽는가

### 세션 시작 (새 대화 첫 턴)

반드시 이 순서:

1. `danbi_briefing` 호출 → `daily.today_notes`, `activity.recent_summaries`, `ghost_suggestions` 확인
2. 어제 daily 노트 있으면 `danbi_read` 한 줄 요약
3. 진행 중이던 TODO 있으면 그것부터 이어서 제안

### 질문 응답 중

| 사용자 뉘앙스 | 호출 |
|---|---|
| "예전에 이거 어떻게 정했지?" | `danbi_search` → `danbi_read` |
| "지난주/어제 뭐 했지?" | `danbi_briefing` activity |
| "1년 전 오늘" / "비슷한 거 한 적 있나" | `danbi_daily` |
| "X 관련 기존 노트" | `danbi_search` |
| "다른 프로젝트에선 어떻게?" | `danbi_list_projects` → `danbi_read` |

**금지**: 일반론으로 답하기. 검색 안 돌리고 추측하기. "아마도" "일반적으로" 시작하기.

## Write — 언제 무엇을 기록하는가

`danbi_log` 트리거:

| 트리거 | 기록 예시 |
|---|---|
| 기술 결정 확정 | "JWT refresh 7일로 정함. 모바일 세션 vs 보안 트레이드오프" |
| 버그 원인 발견 | "CORS 에러는 /api/auth preflight 미설정. next.config.js headers" |
| 세션 종료 전 TODO | "내일: RLS 정책 4개 마이그레이션, 스키마 dump 비교" |
| 노하우 (되풀이) | "Supabase RLS 는 service_role 키 쓰면 자동 우회" |
| 재발 방지 | "Tauri v2 webview 에서 window.prompt 작동 X → 인라인 input" |
| 아키텍처 전환 | "REST → tRPC 시작. 타입 안전성 + codegen 불필요" |

**기록 형식**: `###` 헤더 + 2-4줄 본문. 코드는 핵심만 5줄 이내. "결정/원인/TODO/노하우/재발 방지" 중 하나로 분류. 잡담 금지.

특정 도메인 파일 갱신은 `danbi_append`.

### 폴더로 누적할 때

같은 카테고리·날짜·주제로 `.md` 가 계속 쌓이면 sub-folder 분리. 단비는 폴더 최대 2단계 (`<folder>/<sub>/<file>.md`).

| 시나리오 | 권장 구조 |
|---|---|
| 매일 통계·메트릭 자동 기록 | `daily/YYYY-MM/DD.md` 또는 `stats/YYYY-MM-DD.md` |
| 주제별 리서치 누적 | `research/<topic>.md` |
| 회의록 / 인터뷰 | `meetings/YYYY-MM-DD-<who>.md` |
| 드래프트·임시 메모 | `drafts/<title>.md` |

호출 — 두 옵션:

**A. 한 번 호출 (권장 · 자동화에 적합)**
```
danbi_create_file(domain="stats/2026-05-17.md", content="...")
```
폴더 없으면 만들고, 파일 없으면 만들고, content 있으면 append. 모두 idempotent.

**B. 두 호출로 분리**
```
1. danbi_create_folder(folder="stats")              ← 첫 날 한 번 (idempotent)
2. danbi_append(domain="stats/2026-05-17.md", ...)  ← 매번 append
```

### 기록 타이밍

- **즉시**: 트리거 순간이 오면 다른 일 끝나기 전에 바로
- **세션 종료 시**: 누락된 트리거 있으면 몰아서
- **이중 기록 방지**: 같은 세션 동일 내용 중복 호출 금지

## Link — 언제 [[wiki-link]] 를 넣는가

### 위키링크 문법
- 같은 프로젝트: `[[파일명.md]]`
- 다른 프로젝트: `[[프로젝트명/파일명.md]]`

### 언제 삽입하나

`danbi_log` / `danbi_append` 본문에 다음 중 하나라도 해당되면 `[[…]]` 자연스럽게 포함:

1. 언급 개념이 기존 문서에 다뤄짐 (`danbi_search` 결과로 확인)
2. 같은 프로젝트 내 관련 결정/토픽
3. 답변 근거로 `danbi_read` 한 문서 인용

예시:
```markdown
### JWT refresh 7일로 확정
세션 만료 UX 개선 위해 1일 → 7일. 보안 이슈는 [[notes/auth-security.md]]
의 리스크 매트릭스 기준 수용 가능. 토큰 만료 처리는 [[notes/token-refresh-flow.md]] 참조.
```

### Ghost 제안 처리

`danbi_briefing` 응답의 `ghost_suggestions` 는 단비가 제안한 "아직 확정 안 된 링크". 작업 중 source 문서 편집 시 제안 target 이 정말 관련 있으면 본문에 `[[target]]` 삽입. 점선 → 실선 승격.

## 안티 패턴

- **❌ `danbi_log` 누락**: 작은 결정일수록 나중에 찾기 어렵다. 기록한다.
- **❌ 일반 지식으로 답**: 이 vault 의 결정이 중요하다. 먼저 검색.
- **❌ 링크 없이 기록**: 관련 문서 있는데 `[[]]` 안 걺 → 그래프 안 자람.
- **❌ project 파라미터 수동 지정**: scoped endpoint 가 자동 clamp. 생략이 맞다.
"#;
