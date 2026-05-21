# 단비 (Danbi) — Claude Code 의 영구 기억

> Claude Code 는 **이번 세션** 을 잘 합니다.
> 단비는 **모든 세션의 기억** 을 남깁니다.

## 무엇인가

단비는 macOS 데스크톱 앱입니다. 화면에는 마크다운 에디터처럼 보이지만, 본질은 **Claude Code 가 영구적으로 기억할 수 있는 로컬 vault** 입니다.

- `~/Danbi_Vault/` 에 평문 마크다운으로 모든 지식이 쌓입니다
- Claude Code 가 MCP 로 vault 를 읽고 쓰면서, 이번 세션의 결정·디버깅·학습을 자동으로 기록합니다
- 다음 세션은 그 위에서 시작합니다 — Claude Code 는 더 이상 빈손으로 시작하지 않습니다

## 왜 필요한가

Claude Code 의 한계 한 가지: **메모리는 휘발성** 입니다.

- 세션이 끝나면 결정도, 디버깅 흔적도 사라집니다
- 같은 버그를 두 번 디버깅합니다
- 같은 결정을 또 내립니다 ("JWT refresh 7일이었나 14일이었나?")
- 다른 프로젝트의 노하우를 잊어버립니다

단비는 이 휘발성을 **로컬 마크다운 + 의미 검색** 으로 보완합니다.

## 어떻게 동작하나

### 1. Claude Code 가 작업하는 동안, 단비가 받아 적습니다

`CLAUDE.md` 단비 블록을 프로젝트에 붙여두면, Claude Code 는 다음 순간마다 자동으로 단비에 기록합니다:

| 트리거 | 기록 예시 |
|---|---|
| 기술 결정 | "JWT refresh 7일로 정함. 모바일 세션 유지 vs 보안" |
| 버그 원인 | "CORS 에러는 /api/auth preflight 미설정" |
| TODO | "내일: RLS 정책 4개 마이그레이션" |
| 노하우 | "Supabase RLS 는 service_role 키 쓰면 자동 우회" |
| 재발 방지 | "Tauri v2 webview 에서 window.prompt 작동 안 함" |

이 기록은 `~/Danbi_Vault/Projects/<프로젝트>/daily/YYYY-MM-DD.md` 에 누적됩니다.

### 2. 다음 세션이 그 기록을 읽고 시작합니다

새 Claude Code 세션이 열리면 자동으로:

```
1. danbi_briefing  → 어제 무슨 일이 있었지
2. danbi_recent    → 최근 수정된 노트
3. danbi_read      → 어제 daily 노트 읽고 한 줄 요약
4. 이어서 할 일 제안
```

### 3. "예전에 어떻게 정했지?" 에 즉답합니다

> 사용자: "JWT refresh 7일이었나 14일이었나?"

- Claude Code 가 `danbi_search("JWT refresh 만료 기간")` 호출
- 의미 검색이 6개월 전 결정 노트 발견
- 답: "7일이에요. 4월 12일에 정했고, 이유는 모바일 세션 유지 vs 보안 트레이드오프였어요"

키워드가 정확하지 않아도 ("토큰 만료" → "JWT refresh") 매칭됩니다. 한국어 ↔ 영어도 가로지릅니다.

### 4. 같은 버그 두 번 안 디버깅합니다

> 사용자: "또 CORS 에러 떴는데"

- Claude Code 가 자동으로 vault 검색
- 2주 전 daily 의 "원인" 항목 발견
- 즉답: "5/3 에 해결한 적 있어요. preflight OPTIONS 미설정이었네요"

### 5. 타 프로젝트의 노하우를 자동 연결합니다

> 사용자 (보니_에이전트 작업 중): "alimtalk 발송 어떻게 하지?"

- Claude Code 가 `danbi_list_projects` → 보니_패밀리에 `alimtalk-notification.md` 발견
- "보니_패밀리에 정리해두신 거 있네요. bodum_app/lib/notification.ts 가 핵심이고…"

여러 프로젝트를 동시에 굴리는 사람의 시나리오입니다.

## 다른 도구들과의 차이

### vs Claude Code 의 메모리 (`/compact`, `/clear`)
- Claude Code 메모리: 세션 종료 = 휘발
- 단비: 로컬 파일이라 영구 + 다음 세션이 자동 회상

### vs Notion / Obsidian
- Notion · Obsidian: **사람이** 직접 정리해야 가치 발생. 결국 정보의 무덤이 됨
- 단비: **AI 가** 작업 중 자동으로 정리 → 사람이 손 안 대도 누적

### vs Cursor 의 Notepad
- Cursor Notepad: 한 프로젝트 안의 노트. 프로젝트 단위 격리
- 단비: 여러 프로젝트 cross-search 가능. "다른 프로젝트에서 어떻게 했지?" 답 가능

## 핵심 구성

### 마크다운 vault
- `~/Danbi_Vault/` 가 git repo 로 자동 초기화
- 모든 편집은 자동 커밋 → 무료 undo
- 단비 앱 사라져도 사용자의 지식은 그대로 남음

### MCP 서버 내장
- Claude Code · Cursor 가 즉시 연결
- 읽기 도구 6개 (briefing / recent / search / read / daily / list)
- 쓰기 도구 2개 (log / append) — 프로젝트 단위로 자동 clamp

### 의미 검색 (벡터 임베딩)
- Tantivy BM25 + 벡터 임베딩 hybrid (Reciprocal Rank Fusion)
- 정확한 키워드 몰라도 의미로 찾음
- 무료 옵션: Voyage AI · Google Gemini · Ollama 로컬

### 에디터
- BlockNote 기반 WYSIWYG 마크다운
- 그래프 뷰로 위키링크 시각화
- daily 노트 + 1주/1개월/1년 회상

## 기술 스택

- **Tauri v2** (Rust 기반 데스크톱)
- **Vite + React + TypeScript** 프론트
- **Tantivy** 전문 검색
- **axum** MCP 서버
- **git2** vault 버전 관리

## 누구를 위한 도구인가

✅ Claude Code · Cursor 같은 AI 코딩 에이전트를 일상적으로 쓰는 사람
✅ 여러 프로젝트를 동시에 굴리는 1인 개발자 · 기획자 · 연구자
✅ 메모는 쌓이지만 활용이 안 되는 사람
✅ 로컬 우선 · BYOK 철학에 동의하는 사람

🔴 완전한 플러그인 생태계가 필요 → Obsidian
🔴 팀 실시간 협업이 핵심 → Notion
🔴 AI 호출에 한 푼도 쓰기 싫음 → Ollama 로컬 임베딩 가능 (LLM 은 Claude Code 구독으로 해결)

## 데이터와 비용

- **데이터**: 사용자 기기에만 존재. 텔레메트리 없음
- **API 키**: macOS Keychain. vault 나 config.json 에는 절대 저장 안 함
- **단비 자체 비용**: 0원. 단비 운영자에게 가는 돈 없음
- **AI 비용**:
  - LLM 추론: Claude Code 구독 (이미 쓰는 거)
  - 임베딩: Voyage AI 무료 200M 토큰 / Google Gemini 무료 일 1500 / Ollama 로컬 무료
  - 일반 사용자는 사실상 추가 비용 0

## 시작하기

1. macOS 앱 설치 (DMG)
2. 첫 실행 시 Onboarding → "Claude Code 연동" 프리셋 선택
3. 임베딩 옵션 선택 (Voyage 권장)
4. 사이드바에서 프로젝트 만들기 → 우클릭 → "CLAUDE.md 단비 블록 복사"
5. 작업 폴더의 `CLAUDE.md` 에 붙여넣기 + Claude Code MCP 등록
6. Claude Code 세션 시작 → 단비가 자동으로 받아 적기 시작

## 한 줄 요약

> 단비는 Claude Code 의 영구 기억입니다.
> 매일 받아 적고, 매일 회상합니다.
