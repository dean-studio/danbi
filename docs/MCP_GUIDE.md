# 단비 MCP 사용 가이드

단비를 **Model Context Protocol (MCP) 서버**로 켜두면 Claude Code, Cursor, Zed 같은 외부 AI 에이전트가 **내 로컬 vault를 검색·읽기·기록**할 수 있어요. 세션이 끝나도 대화가 마크다운으로 남고, 다음 세션의 AI가 어제의 나를 기억합니다.

## 목차

1. [왜 이게 중요한가](#1-왜-이게-중요한가)
2. [MCP 서버 켜기](#2-mcp-서버-켜기)
3. [Claude Code 연결](#3-claude-code-연결)
4. [Cursor 연결](#4-cursor-연결)
5. [다른 클라이언트 / 직접 호출](#5-다른-클라이언트--직접-호출)
6. [제공되는 도구 14개](#6-제공되는-도구-14개)
7. [실전 워크플로 3가지](#7-실전-워크플로-3가지)
8. [보안](#8-보안)
9. [문제 해결](#9-문제-해결)

---

## 1. 왜 이게 중요한가

### 문제
Claude Code에서 한 시간 작업한 대화는 **세션이 끝나면 증발**합니다. 다음 날 "어제 토큰 만료 어떻게 하기로 했지?" 물으면 AI는 모릅니다.

### 해결
단비 MCP를 붙이면 AI가 **자동으로 작업 일지를 마크다운으로 남기고**, 다음 세션에서 그걸 검색해 꺼내옵니다.

```
[Day 1, Cursor 세션]
Claude: JWT refresh token 전략 결정했으니 단비에 기록할게요.
 → danbi_log("보니", "## Auth · JWT refresh 7d, access 1h")

[Day 2, 새 Claude Code 세션]
나: 어제 auth 어떻게 하기로 했지?
Claude → danbi_search("auth refresh token")
Claude: 보니/daily/2026-05-10.md 에 기록돼 있어요. access 1h, refresh 7d 로 정했네요.
```

---

## 2. MCP 서버 켜기

1. 단비 앱 실행 → `⌘,` 로 **설정 열기**
2. 좌측 사이드바에서 **MCP** 섹션 선택
3. **"켜기"** 버튼 클릭
4. 화면에 뜨는 값 확인:
   - **URL**: `http://127.0.0.1:47921/mcp` (기본 포트 47921)
   - **토큰**: 랜덤 44자 문자열 (Base64)
5. **"Claude Code 설정 스니펫 복사"** 버튼으로 JSON 전체 클립보드 복사

### 상태 확인

설정 화면에 녹색 "실행 중" 뱃지가 떠 있으면 정상. 끄고 싶을 때는 같은 자리에서 "끄기".

### 기본 동작

- 앱 재시작 시 자동 복구 (config.json에 enabled=true 저장됨)
- 트레이바에 단비가 상주해 있으면 MCP도 계속 켜져 있음
- 앱을 완전 종료 (`⌘Q`)하면 MCP도 꺼짐

---

## 3. Claude Code 연결

가장 쉬운 길은 단비가 만들어 주는 명령을 붙여넣는 것입니다.

### 방법 A — 프로젝트별 연결 (권장)

1. 단비 사이드바에서 프로젝트 우클릭 → **"Claude Code 설치 명령 복사"** (또는 프로젝트 홈의 Claude Code 줄)
2. 그 프로젝트의 코드 폴더에서 터미널에 붙여넣기:

```bash
claude mcp add --transport http danbi-<hash> "http://127.0.0.1:47921/mcp/<project-id>" \
  --header "Authorization: Bearer <토큰>"
```

`claude mcp add` 는 기본이 **local 스코프**(그 폴더에서만 보임)라 코드 폴더마다 한 번씩 실행합니다. 쓰기 도구는 그 프로젝트로 자동 clamp 되고, 읽기는 vault 전체를 봅니다. 서버 이름 `danbi-<hash>` 는 단비가 프로젝트명으로 만든 안정적인 이름이에요.

### 방법 B — 읽기·쓰기 분리 연결 (도구 수 줄이기)

프로젝트마다 `/mcp/<project-id>` 를 붙이면 세션에 같은 도구가 서버 수만큼 겹쳐 뜹니다 (예: 9개 프로젝트 × 14개 도구). 연결을 둘로 나누면 **읽기 8개 + 쓰기 6개** 만 보입니다.

| 엔드포인트 | 노출 도구 | 범위 | 붙이는 곳 |
|---|---|---|---|
| `/mcp/read` | 읽기 8개 (`danbi_briefing` · `danbi_context` · `danbi_search` · `danbi_read` · `danbi_related` · `danbi_recent` · `danbi_daily` · `danbi_list_projects`) | vault 전체 | 전역(user) 한 번 |
| `/mcp/<project-id>/write` | 쓰기 6개 (`danbi_log` · `danbi_append` · `danbi_create_file` · `danbi_create_folder` · `danbi_replace_section` · `danbi_upsert_item`) — 스키마에 `project` 가 없고 설명에 "→ {프로젝트} 에 기록" 이 적혀 있음 | 그 프로젝트로 자동 clamp | 코드 폴더마다(local) |

단비 설정 → MCP → **권장 연결** 에서 두 명령을 복사할 수 있어요.

```bash
# 1. 읽기 — 한 번만 (모든 폴더에서 보임)
claude mcp add --transport http -s user danbi_read "http://127.0.0.1:47921/mcp/read" \
  --header "Authorization: Bearer <토큰>"

# 2. 쓰기 — 각 코드 폴더 안에서 실행 (그 폴더에서만 보임)
claude mcp add --transport http -s local danbi "http://127.0.0.1:47921/mcp/<project-id>/write" \
  --header "Authorization: Bearer <토큰>"
```

- 읽기 연결에서 쓰기 도구를 부르면 거절되고, 어느 연결을 써야 하는지 알려줍니다 (반대도 같음).
- 기존 `/mcp` · `/mcp/<project-id>` 는 그대로 동작합니다. 도구 수를 줄이려면 예전에 붙인 `danbi-…` 서버를 `claude mcp list` 로 확인하고 `claude mcp remove <이름>` 으로 지우세요.

### 방법 C — 팀 공유용 `.mcp.json`

저장소에 같이 넣고 싶으면 코드 폴더 루트의 `.mcp.json` (Claude Code 프로젝트 스코프):

```json
{
  "mcpServers": {
    "danbi": {
      "type": "http",
      "url": "http://127.0.0.1:47921/mcp/<project-id>",
      "headers": { "Authorization": "Bearer <토큰>" }
    }
  }
}
```

토큰이 들어가니 공개 저장소엔 커밋하지 마세요 (`.git/info/exclude` 추천).

### 서버가 직접 알려주는 사용법

`initialize` 응답의 `instructions` 에 "세션 시작엔 `danbi_briefing`, 질문 전엔 `danbi_context`, 결정·버그 원인·TODO 는 즉시 `danbi_log`" 네 줄이 들어 있어 SKILL.md 가 없어도 에이전트가 쓰는 법을 압니다. 스코프 서버면 프로젝트명이 그 문장에 박혀 나갑니다.

### 세션 훅 (선택)

프로젝트 홈의 Claude Code 줄에서 **훅** 을 설치하면 코드 폴더의 `.claude/settings.local.json` 에:

- **SessionStart** — `/api/session-brief` 가 만든 2,500자 안팎의 브리핑(최근 바뀐 문서·오늘/어제 daily 한 줄·열린 할 일·7일 활동·정리 제안 건수)이 세션 첫 맥락으로 들어갑니다. LLM 호출 없음.
- **SessionEnd** (기본 꺼짐) — 세션 중 단비에 직접 기록하지 않았으면 자동화 모델이 결정/완료/다음 할 일을 5줄 이하로 요약해 오늘 daily 에 남깁니다.

### 적용 확인

Claude Code 세션에서 `/mcp` 입력 → `danbi … ✓` 가 보이면 정상. 첫 테스트:

```
단비에 어떤 프로젝트가 등록돼 있어?
```

Claude 가 `danbi_list_projects` 를 호출하고 프로젝트 목록을 요약해서 답해야 정상입니다.

---

## 4. Cursor 연결

Cursor는 **Settings → Features → MCP** 메뉴에서 추가합니다.

### 방법 A — GUI

1. Cursor Settings 열기 (`⌘,`)
2. Features → MCP Servers
3. **"Add Server"**
4. 입력:
   - **Name**: `danbi`
   - **Type**: `sse` 또는 `http` (Cursor 버전에 따라 표기 다름)
   - **URL**: `http://127.0.0.1:47921/mcp`
   - **Headers**: `Authorization: Bearer <토큰>`

### 방법 B — 설정 파일 직접 편집

macOS Cursor의 MCP 설정 파일:

```
~/Library/Application Support/Cursor/User/globalStorage/mcp.json
```

위 Claude Code 스니펫과 동일한 형태로 붙여넣으면 됩니다.

### 적용 확인

Cursor에서 composer 열고:

```
@danbi 오늘 어떤 작업이 있었어?
```

---

## 5. 다른 클라이언트 / 직접 호출

MCP는 JSON-RPC 2.0 over HTTP 표준이라 **curl로도 바로 호출**됩니다.

### 도구 목록

```bash
TOKEN="여기에-토큰"

curl -s -X POST http://127.0.0.1:47921/mcp \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | jq
```

### 도구 호출 예시 — 프로젝트 목록

```bash
curl -s -X POST http://127.0.0.1:47921/mcp \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 2,
    "method": "tools/call",
    "params": {
      "name": "danbi_list_projects",
      "arguments": {}
    }
  }' | jq
```

### 도구 호출 예시 — 검색

```bash
curl -s -X POST http://127.0.0.1:47921/mcp \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 3,
    "method": "tools/call",
    "params": {
      "name": "danbi_search",
      "arguments": { "query": "auth refresh token", "limit": 5 }
    }
  }' | jq
```

### 도구 호출 예시 — 오늘 작업 기록

```bash
curl -s -X POST http://127.0.0.1:47921/mcp \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 4,
    "method": "tools/call",
    "params": {
      "name": "danbi_log",
      "arguments": {
        "project": "보니",
        "content": "## 결정\n- JWT refresh token 7일, access 1시간\n- HttpOnly cookie 사용"
      }
    }
  }' | jq
```

---

## 6. 제공되는 도구 14개

모두 **LLM 호출 없음** — 외부 AI 가 tool 을 부르는 비용 외에 단비가 추가로 모델을 부르지 않습니다 (의미 검색의 쿼리 임베딩 1회 제외).

### 읽기 (8)

| 도구 | 파라미터 | 용도 |
|---|---|---|
| `danbi_briefing` | `project?` | 세션 시작용 요약 한 JSON: 오늘 daily 노트, 7일 활동(기록 수·최근 요약 6), 정리 제안 건수 + 상위 10건, 미확정 링크 제안. `project` 를 주면 그 프로젝트만 (스코프 서버면 기본값). 전체 약 4k자, 프로젝트 약 2.5k자 |
| `danbi_context` | `query`, `limit?`, `max_chars?` | 질문 하나로 관련 문서의 해당 섹션 + `[[링크]]` 이웃을 글자 예산 안에서. **search → read 반복 대신 먼저** |
| `danbi_search` | `query`, `limit?`, `project?`, `mode?` | BM25 + 한국어 n-gram, 임베딩 provider 가 있으면 RRF 하이브리드. `project` 로 한 프로젝트만. `mode`: `hybrid`(기본) · `keyword`(임베딩 호출 없이 BM25만) · `semantic`(벡터만). 결과는 `{hits, mode, semantic, note?, warning?}` — `semantic` 이 `timeout`/`error` 면 임베딩이 1.5초 안에 안 와 키워드 결과만 준 것 |
| `danbi_read` | `project`, `domain`, `heading?`, `max_chars?`, `offset?` | 문서 본문. 첫 줄은 메타 주석 (`bytes · chars · modified · links_in`, 범위를 잘랐으면 다음 `offset`). `heading` 으로 섹션만 |
| `danbi_related` | `project`, `domain`, `limit?` | 한 문서의 링크·백링크·내용상 가까운 문서·같은 주제 묶음 |
| `danbi_recent` | `limit?`, `project?`, `exclude_daily?` | 최근 수정 문서 (하위 폴더 포함, daily 는 기본 제외) |
| `danbi_daily` | `project?` | 오늘 + 1주/1달/1년 전 같은 날짜의 daily 노트 |
| `danbi_list_projects` | `project?`, `include_daily?`, `titles?` | 기본은 프로젝트당 한 줄 (파일 수·최근 수정·최상위 파일·폴더별 개수, 전체 약 7k자). `project` 를 주면 그 프로젝트의 파일 트리 |

### 쓰기 (6)

스코프 서버(`/mcp/<project-id>`, `/mcp/<project-id>/write`)에서는 `project` 파라미터가 스키마에 없고 서버가 채웁니다.

| 도구 | 파라미터 | 용도 |
|---|---|---|
| `danbi_log` | `project`, `content`, `stamp?` | 오늘 `daily/YYYY-MM-DD.md` 에 append. 첫 제목 바로 아래에 `_HH:MM · claude-code_` 줄 자동 (`stamp:false` 로 끔). 직전 기록과 본문이 같으면(재시도) 다시 쓰지 않고 `deduped:true` |
| `danbi_append` | `project`, `domain`, `content` | 파일 끝에 append. 없으면 생성 |
| `danbi_create_file` | `project`, `domain`, `content?` | 폴더+파일+내용을 한 번에 보장 (idempotent) |
| `danbi_create_folder` | `project`, `folder` | 1~2단계 sub-folder 생성 |
| `danbi_replace_section` | `project`, `domain`, `heading`, `new_body` | 헤딩 아래 본문 전체 교체 (없으면 섹션 추가) |
| `danbi_upsert_item` | `project`, `domain`, `heading`, `key`, `item` | 리스트 섹션의 한 항목 in-place 교체 / 추가 |

### 쓰기 응답 읽는 법

```json
{ "project": "보니", "domain": "daily/2026-10-15.md", "bytes": 2048, "commit": "3f9c…" }
```

- `commit` 이 문자열이면 git 에 기록된 것.
- `commit: null` + `warning` — **파일은 저장됐지만** 커밋만 실패 (다른 git 프로세스가 `index.lock` 을 잡고 있던 경우 등). 다음 쓰기 때 함께 커밋되니 **재시도하지 마세요**.
- `deduped: true` (`danbi_log`) — 같은 날 같은 본문이 이미 있어 쓰지 않음.
- 쓰기 6개·REST PUT·앱의 빠른 메모는 **하나의 전역 락**을 공유합니다. 에이전트 여러 개가 같은 daily 에 동시에 써도 유실되지 않아요.

### 오류 읽는 법

- `unknown project: 보니앨범. 혹시 보니_앨범? 등록된 프로젝트: …` — 이름 오타. 제안된 이름으로 다시.
- `not found: 보니/notes/auth-flow.md. 혹시 notes/auth.md? …` — 없는 문서. (예전엔 빈 문자열이 돌아와 "빈 문서"로 오해했음.) 새로 만들려면 `danbi_create_file`.
- `heading not found: … 이 문서의 헤딩: …` — `danbi_read(heading)` 의 헤딩 오타.
- `domain required (예: "notes/auth.md")` — 필수 인자 누락. 괄호 안이 형식 예시.
- `warning: 의미 검색 결과에서 문서 N개가 빠졌어요 …` (`danbi_search` 는 결과 객체의 `warning` 키, `danbi_context` 도 같은 키) — 임베딩 모델을 바꾼 뒤 전체 재색인을 안 한 상태. 키워드 검색엔 잡히니 결과는 쓰되, 설정 → AI 연동에서 재색인.
- `danbi_search` 의 `semantic: "timeout"` + `note` — 임베딩이 1.5초 안에 안 와서 키워드 결과만 준 것. 결과는 쓰되, 풀어 쓴 질문이면 잠시 후 `mode: "semantic"` 으로 한 번 더.

### 모든 쓰기 작업은 자동 git commit

쓰기 도구가 호출될 때마다 수정 전 `(pre)` 스냅샷 → 파일 쓰기 → 본 커밋. 외부 AI 가 실수로 이상한 걸 써도 단비에서 **되돌리기** 가 됩니다 — 되돌리기는 마지막 기록이 바꾼 **그 파일들만** 이전 상태로 돌리고 새 커밋을 남기므로, 그 사이 다른 에이전트가 쓴 노트는 사라지지 않아요. `config.json` 과 `.danbi/`(벡터 색인·캐시)는 스냅샷에 들어가지 않습니다.

---

## 7. 실전 워크플로 3가지

### 7.1. Auto-Journal

Claude Code 세션 시작할 때 시스템 프롬프트에 한 줄 추가:

> 중요한 결정이나 배운 것이 생길 때마다 `danbi_log` 로 현재 작업 중인 프로젝트의 오늘 데일리 노트에 기록해주세요.

이후 AI가 알아서 필요한 순간마다 `danbi_log` 호출. 하루 종일 작업하고 나면 `daily/2026-05-11.md` 에 자동으로 작업 일지가 쌓여 있음.

### 7.2. "어제의 나" 불러오기

새 Claude Code 세션 시작:

```
나: 단비 보니 프로젝트 최근 작업 좀 요약해줘.
AI: (danbi_recent 호출 → danbi_read 몇 개 호출)
     최근 3일간 보니/daily 에 쌓인 내용을 보면…
```

기억 인계 비용이 **토큰 몇 천 개**로 끝남.

### 7.3. 크로스 프로젝트 참조

Cursor에서 **보니 프로젝트** 코드 짜는 중인데 상식이 프로젝트에서 썼던 패턴이 기억 안 날 때:

```
나: 상식이 backend.md 에서 에러 핸들링 패턴 어떻게 했는지 찾아줘.
AI: (danbi_search("상식이 error handling") → danbi_read("상식이", "backend.md"))
     이 패턴을 쓰셨네요…
```

---

## 8. 보안

### 기본 보호막

- **`127.0.0.1`만 바인딩** — 외부 네트워크 / LAN 에서 접근 불가
- **Bearer 토큰 인증** — 잘못된/없는 토큰 → 401
- **랜덤 토큰** — 256bit Base64URL, 추측 불가

### 알아야 할 것

- 같은 Mac에서 돌아가는 **다른 프로세스**는 이론상 포트로 접근 시도 가능 — 하지만 토큰을 알아야 함
- 토큰은 `config.json` 에 평문으로 저장됨 (이미 로컬이고 Vault 자체가 민감 데이터를 가진 전제)
- 토큰을 Git에 올리거나 스크린샷에 포함시키지 마세요

### 권장 습관

- 1~2개월마다 **"새 토큰 생성"** 버튼으로 토큰 회전
- 공유 Mac에서 쓴 뒤에는 MCP **끄기**
- Claude Code/Cursor 설정 파일을 팀 저장소에 절대 커밋 ❌

---

## 9. 문제 해결

### Claude Code가 danbi를 못 찾음

```bash
# 단비 서버가 정말 살아있는지 헬스체크
curl http://127.0.0.1:47921/mcp/health
# "danbi-mcp" 가 나와야 정상
```

안 나오면:
- 단비 앱이 켜져 있는지 확인
- 설정 → MCP → 상태가 "실행 중"인지 확인
- 방화벽/VPN이 loopback을 막는지 확인

### 401 Unauthorized

- 스니펫의 토큰과 현재 단비 토큰이 같은지 비교
- 새 토큰 생성 후 Claude Code 설정 파일 업데이트 빠뜨렸는지 확인

### 포트가 이미 쓰이고 있음

기본 `47921` 이 다른 프로세스와 충돌하면:

```bash
lsof -i:47921
```

단비를 끄고 설정에서 다른 포트로 변경 (Settings → MCP 에서 포트 표시 옆 수정 가능, 또는 `~/Danbi_Vault/config.json` 직접 편집). 그다음 클라이언트 설정의 URL도 새 포트로 맞춰야 함.

### 쓰기 도구에 `project` 가 없다고 뜸 / 엉뚱한 프로젝트에 기록됨

- 스코프 서버(`/mcp/<project-id>`)에 붙었으면 `project` 는 스키마에서 빠지고 서버가 채웁니다 — 정상.
- 전역 `/mcp` 에 붙었으면 `project` 가 필수. 프로젝트별 연결(3절 방법 A/B)로 바꾸면 매번 이름을 줄 필요가 없어요.

### 도구가 호출되긴 하는데 에러만 뜸

- 오류 문장에 다음 행동이 들어 있어요 (비슷한 이름 제안, 예시 값). 그대로 따르면 대부분 해결.
- `danbi_list_projects` 로 프로젝트가 실제로 있는지 먼저 확인
- `vault_path` 가 단비 설정과 동일한지 (외부에서 이동했다면 재설정 필요)
- 토큰이 유효해도 vault 자체가 세팅 안 된 상태면 "vault not configured" 에러

### 로그 보기

단비는 stderr 에 MCP 이벤트를 출력합니다. dev 모드면:

```bash
tail -f ~/works/agent/danbi/.dev.log | grep mcp
```

프로덕션 앱이면 Console.app 에서 `danbi` 검색.

---

## 관련 문서

- [FEATURES.md](../FEATURES.md) — 단비 전체 차별화 기능 개요
- [HANDOFF.md](../HANDOFF.md) — 프로젝트 진행 상태 & 파일 맵
- 공식 MCP 명세: https://modelcontextprotocol.io/

---

**© 2026 단비 · 피드백은 GitHub issues 로 남겨주세요.**
