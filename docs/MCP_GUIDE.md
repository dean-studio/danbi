# 단비 MCP 사용 가이드

단비를 **Model Context Protocol (MCP) 서버**로 켜두면 Claude Code, Cursor, Zed 같은 외부 AI 에이전트가 **내 로컬 vault를 검색·읽기·기록**할 수 있어요. 세션이 끝나도 대화가 마크다운으로 남고, 다음 세션의 AI가 어제의 나를 기억합니다.

## 목차

1. [왜 이게 중요한가](#1-왜-이게-중요한가)
2. [MCP 서버 켜기](#2-mcp-서버-켜기)
3. [Claude Code 연결](#3-claude-code-연결)
4. [Cursor 연결](#4-cursor-연결)
5. [다른 클라이언트 / 직접 호출](#5-다른-클라이언트--직접-호출)
6. [제공되는 도구 6개](#6-제공되는-도구-6개)
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

### 설정 파일 위치

```
~/.claude/mcp.json
```

없으면 파일을 새로 만드세요.

### 내용

```json
{
  "mcpServers": {
    "danbi": {
      "url": "http://127.0.0.1:47921/mcp",
      "headers": {
        "Authorization": "Bearer <여기에-토큰-붙여넣기>"
      }
    }
  }
}
```

단비 설정의 **"Claude Code 설정 스니펫 복사"** 버튼을 누르면 이 형식 그대로 클립보드에 담깁니다.

### 적용 확인

Claude Code 재시작 후, 세션에서 `/mcp` 입력:

```
/mcp

servers
  danbi · 6 tools available ✓
```

### 첫 테스트

```
단비에 어떤 프로젝트가 등록돼 있어?
```

Claude가 `danbi_list_projects` 를 호출하고 결과를 요약해서 답해야 정상입니다.

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

## 6. 제공되는 도구 6개

| 도구 | 파라미터 | 용도 | LLM 호출 |
|---|---|---|---|
| `danbi_list_projects` | 없음 | 등록된 프로젝트와 각 도메인 파일 리스트 | ❌ |
| `danbi_search` | `query: string`, `limit?: number` | tantivy 전문 검색 (한국어 지원) | ❌ |
| `danbi_read` | `project: string`, `domain: string` | 특정 파일의 전체 마크다운 | ❌ |
| `danbi_log` | `project: string`, `content: string` | 해당 프로젝트의 오늘 `daily/YYYY-MM-DD.md` 에 append | ❌ |
| `danbi_append` | `project: string`, `domain: string`, `content: string` | 특정 파일에 append. 파일이 없으면 생성 | ❌ |
| `danbi_recent` | `limit?: number` | 최근 수정된 파일 top N | ❌ |

**LLM 호출 없음** = 외부 AI가 tool을 부르는 비용 외에 단비가 추가로 Bedrock에 호출하지 않습니다. 네트워크 왕복 + 로컬 디스크 I/O + tantivy 검색만.

### 입력 스키마 상세

각 도구의 `inputSchema`는 `tools/list` 응답으로 확인할 수 있어요.

```json
{
  "name": "danbi_search",
  "inputSchema": {
    "type": "object",
    "properties": {
      "query": { "type": "string" },
      "limit": { "type": "integer", "default": 8 }
    },
    "required": ["query"]
  }
}
```

### 모든 쓰기 작업은 자동 git commit

`danbi_log`, `danbi_append` 가 호출될 때마다:
1. 수정 전 스냅샷 commit
2. 실제 파일 쓰기
3. 수정 후 commit

즉 외부 AI가 실수로 이상한 걸 써도 **단비에서 "되돌리기"** 가능.

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

### 도구가 호출되긴 하는데 에러만 뜸

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
