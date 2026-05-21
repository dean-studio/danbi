# 단비 Vault REST API 명세

페이퍼클립 루틴이 단비 vault에 리포트를 저장하기 위한 단순 REST API. 옵시디언 Local REST API 와 동일한 패턴 (PUT + raw markdown).

관련: [[project_paperclip_setup]] · [[reference_obsidian]]

## 베이스

- URL: `http://127.0.0.1:47921` (로컬 전용, 외부 노출 X)
- 인증: `Authorization: Bearer <DANBI_REST_TOKEN>`
- 토큰: 단비 → Settings → MCP 섹션 → token. `~/Danbi_Vault/config.json` 의 `mcp.token` 과 동일. (현재는 MCP 토큰을 재사용 — env 변수로 분리 운영 권장.)

## 엔드포인트

### `PUT /v1/vault/{project}/{path...}`

파일 쓰기. 폴더 자동 생성.

**Path**

- `{project}` — vault 프로젝트명. 예: `dean_works_agent`
  - 프로젝트 UUID 도 받음 (vault 의 `.danbi-id` 와 일치 시).
- `{path...}` — 슬래시 포함 경로. `.md` 로 끝나야 함. 예: `daily/2026-05-17/모닝메시지.md`
  - 폴더 깊이 최대 2단계 (`<folder>/<sub>/<file>.md`).

**Headers**

- `Authorization: Bearer <token>` (필수)
- `Content-Type: text/markdown; charset=utf-8`

**Body**: 마크다운 raw 본문 (UTF-8). 한국어 / 이모지 / 특수문자 그대로 통과. 최대 32MB.

**Query**

- 기본: 덮어쓰기 (파일 없으면 생성, 있으면 통째 교체)
- `?mode=append`: 기존 끝에 `\n\n` + 본문 추가 (없으면 새 파일 생성)

**응답 200**

```json
{
  "ok": true,
  "project": "dean_works_agent",
  "domain": "daily/2026-05-17/모닝메시지.md",
  "bytes": 1234,
  "mode": "overwrite",
  "commit": "abc123de"
}
```

**오류**

| 코드 | 의미 |
| --- | --- |
| 401 | 토큰 불일치 / 누락 |
| 400 | `.md` 아님 / `..` 포함 / 빈 path / mode 가 overwrite·append 가 아님 |
| 404 | 모르는 project |
| 422 | 폴더 깊이 초과 등 vault 정책 위반 |

## 사용 예 (페이퍼클립 prompt)

```bash
DATE=$(TZ=Asia/Seoul date +%Y-%m-%d)
cat > /tmp/report.md <<'EOF'
# 모닝 메시지

본문...
EOF
curl -fsS -X PUT \
  "http://127.0.0.1:47921/v1/vault/dean_works_agent/daily/${DATE}/모닝메시지.md" \
  -H "Authorization: Bearer ${DANBI_REST_TOKEN}" \
  -H "Content-Type: text/markdown; charset=utf-8" \
  --data-binary @/tmp/report.md
```

추가 예 — append 모드 (이미 있는 파일에 한 줄 더 쌓기):

```bash
curl -fsS -X PUT \
  "http://127.0.0.1:47921/v1/vault/dean_works_agent/notes/decisions.md?mode=append" \
  -H "Authorization: Bearer ${DANBI_REST_TOKEN}" \
  -H "Content-Type: text/markdown; charset=utf-8" \
  --data-binary @/tmp/decision-snippet.md
```

## 다른 도구들

같은 단비 데몬이 다음도 함께 노출:

- `POST /mcp` — JSON-RPC (Claude Code · Cursor 가 쓰는 표준 MCP 인터페이스)
- `POST /api/call/:tool` — JSON-RPC 없이 단일 도구 호출 (편의용 REST 래퍼)
- `GET  /api/tools` — 사용 가능한 도구 목록 + 스키마

PUT REST 는 "그냥 파일 한 통 던져넣기"용. `/api/call` 은 "도구 인자 다 챙겨서 호출하고 결과 받기"용.

## 구현 메모

- 내부 로직: `vault::create_folder` (폴더 자동 생성) + `vault::write_doc` (overwrite) 또는 read+concat+write (append)
- 입력 검증: `.md` 강제, `..` 차단, `is_safe_segment` 통과 필요, project allowlist (vault tree 의 실제 프로젝트만 허용)
- git 자동 커밋: `(pre)` 스냅샷 후 쓰기, 그 후 본 커밋. 사용자가 단비 안에서 undo 가능.
- 동일한 단비 데몬 프로세스에 라우트만 추가 — 별도 서버 띄우지 않음.

## 보안 주의

- 단비 MCP 서버는 **127.0.0.1** 바인드. 외부 네트워크에 노출 안 됨.
- 외부 머신 (예: dean.kr 서버) 에서 호출하려면 Cloudflare Tunnel / SSH 포트 포워딩 같은 별도 경로 필요.
- 토큰은 macOS Keychain 이 아닌 vault 의 `config.json` 평문에 있음 — 단비 vault 디렉토리 권한 관리 주의.
