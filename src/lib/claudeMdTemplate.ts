/**
 * CLAUDE.md 단비 연동 블록 — 프로젝트별 Claude Code 안내문. 연결 시트의
 * "CLAUDE.md 블록 복사" 와 사이드바 우클릭이 같은 본문을 쓴다. `danbi:start`
 * / `danbi:end` 마커 사이는 단비가 관리하므로 재복사 시 교체하면 된다.
 */
export function buildClaudeMdBlock(project: string): string {
  const tpl = `<!--
  단비(Danbi) vault 연동 블록. 이 마커 사이는 단비가 관리한다.
  기존 CLAUDE.md 에 병합하는 규칙:
  1. 파일에 <!-- danbi:start --> ... <!-- danbi:end --> 마커가 이미 있으면
     그 사이만 이 블록 전체로 교체한다. 마커 밖의 사용자 콘텐츠는 건드리지 않는다.
  2. 마커가 없으면 파일 맨 끝에 이 블록 전체를 append 한다.
  3. 사용자 고유 섹션 (프로젝트 특화 빌드 명령, 팀 컨벤션 등) 은 마커 밖에 두면 유지된다.
-->
<!-- danbi:start v=2 project=${project} -->
# ${project} — 단비 vault 연동 가이드

이 저장소는 단비(Danbi) MCP 서버의 "${project}" 전용 엔드포인트에 연결돼 있다.
단비 vault는 이 프로젝트의 **외부 장기 기억**이다 — 결정·디버깅·TODO·학습이
세션을 넘어 누적된다.

## 핵심 원칙 (이것만 기억하면 됨)

1. **답변 전에 단비부터 물어본다** — 일반론 대신 vault의 맥락으로 답한다.
2. **중요한 건 즉시 기록한다** — 기술 결정·버그 원인·TODO는 ${"`danbi_log`"}로 바로.
3. **쓸 때는 연결한다** — 관련 문서를 ${"`[[wiki-link]]`"}로 걸어 그래프에 누적시킨다.

## 도구 목록

**읽기** (vault 전체 조회 가능):
- \`danbi_briefing\` — 최근 활동·ghost 제안·고아 파일·daily 노트를 한 JSON으로 반환
- \`danbi_recent\` — 최근 수정된 문서 목록
- \`danbi_search\` — tantivy 전문 검색 (한국어 n-gram 지원)
- \`danbi_context\` — 질문 하나로 관련 문서의 해당 섹션 + 연결 문서 목록을 글자 예산 안에서 한 번에. **search → read 반복 대신 먼저**
- \`danbi_related\` — 문서 하나의 링크·백링크·내용상 가까운 문서·같은 주제 묶음
- \`danbi_read\` — 특정 프로젝트/도메인 파일의 전체 내용
- \`danbi_daily\` — 오늘 + 1주/1달/1년 전 daily 노트
- \`danbi_list_projects\` — vault 전체 프로젝트·도메인 트리

**쓰기** (이 엔드포인트는 "${project}"로 자동 clamp — 다른 프로젝트로 잘못 쓸 위험 0):
- \`danbi_log\` — 오늘 daily 노트에 append. project 파라미터 **생략**.
- \`danbi_append\` — 임의 도메인 파일에 append (없으면 자동 생성). project 파라미터 **생략**.
- \`danbi_create_folder\` — 프로젝트 안에 1~2단계 sub-folder 생성 (예: \`stats\`, \`daily/2026-05\`). 카테고리·시기별 누적용. project 파라미터 **생략**.
- \`danbi_create_file\` — **폴더 + 파일을 한 번에 보장 + 내용 append**. \`danbi_create_folder\` + \`danbi_append\` 두 호출을 한 번으로 합친 편의 도구. 매일 통계 자동 기록 같은 자동화에 적합. project 파라미터 **생략**.

## Read — 언제 무엇을 읽는가

### 세션 시작 (새 대화 첫 턴)

반드시 이 순서로:

1. \`danbi_briefing\` 호출 → 응답에서 다음 확인:
   - \`daily.today_notes[0]\` 있으면 오늘 이미 기록된 내용이 있다는 뜻
   - \`activity.recent_summaries[0..3]\` 로 직전 작업 맥락 파악
   - \`ghost_suggestions\` 에 pending 있으면 사용자에게 알릴지 판단
2. \`daily.today_notes\` 또는 어제 노트 있으면 \`danbi_read\` 로 읽고 **한 줄 요약**
3. 읽은 요약 + 진행 중이던 TODO가 있으면 **그것부터 이어서 제안**

### 질문 응답 중

| 사용자가 이런 뉘앙스로 물으면 | 이 도구를 먼저 |
|---|---|
| "예전에 이거 어떻게 정했지?" | \`danbi_context\` (부족하면 \`danbi_read\`) |
| "이 문서랑 관련된 건?" | \`danbi_related\` |
| "지난주/어제 뭐 했지?" | \`danbi_briefing\` activity |
| "1년 전 오늘" / "비슷한 작업 한 적 있나" | \`danbi_daily\` |
| "X 관련 기존 노트" | \`danbi_search\` 쿼리 |
| "다른 프로젝트에선 어떻게?" | \`danbi_list_projects\` → 해당 프로젝트 \`danbi_read\` |

**금지**: 일반론으로 답하기. 검색 안 돌리고 추측하기. "아마도" "일반적으로"로 시작하기.

## Write — 언제 무엇을 기록하는가

${"`danbi_log`"}를 호출하는 **트리거**:

| 트리거 | 기록 예시 |
|---|---|
| 기술 결정 확정 | "JWT refresh 7일로 정함. 이유: 모바일 세션 유지 vs 보안 트레이드오프" |
| 버그 원인 발견 | "CORS 에러는 /api/auth preflight 미설정. next.config.js headers 에 추가" |
| 세션 종료 전 TODO | "내일: RLS 정책 테이블 4개 더 마이그레이션, 스키마 dump 비교" |
| 노하우 (되풀이하면 좋은 것) | "Supabase RLS는 service_role 키 쓰면 자동 우회 — 마이그레이션 스크립트에서 유용" |
| 재발 방지 (또 겪지 말아야 할 것) | "Tauri v2 webview에서 window.prompt 작동 안 함 → 인라인 input 대신" |
| 아키텍처 전환 | "REST → tRPC 마이그레이션 시작. 이유: 타입 안전성 + codegen 불필요" |

**기록 형식**:
- 짧은 markdown 섹션. \`###\` 헤더 + 2-4줄 본문.
- 코드는 핵심만 5줄 이내.
- 잡담·회고·감상 금지 — "결정 / 원인 / TODO / 노하우 / 재발 방지" 중 하나로 분류.

특정 도메인 파일(예: \`notes/auth.md\`)을 갱신해야 하면 ${"`danbi_append`"}.

### 폴더로 누적할 때

같은 카테고리·날짜·주제로 \`.md\` 가 계속 쌓이는 패턴이면 sub-folder 를 먼저 만들어 분리한다. 단비는 폴더를 최대 2단계까지 지원한다 (\`<folder>/<sub>/<file>.md\`).

| 시나리오 | 권장 구조 |
|---|---|
| 매일 통계·메트릭 자동 기록 | \`daily/YYYY-MM/DD.md\` 또는 \`stats/YYYY-MM-DD.md\` |
| 주제별 리서치 누적 | \`research/<topic>.md\` |
| 회의록 / 인터뷰 | \`meetings/YYYY-MM-DD-<who>.md\` |
| 드래프트·임시 메모 | \`drafts/<title>.md\` |

호출 패턴 — 두 가지 옵션:

**A. 한 번 호출 (권장 · 자동화에 적합)**
\`\`\`
danbi_create_file(domain="stats/2026-05-17.md", content="...")
\`\`\`
폴더 없으면 만들고, 파일 없으면 만들고, content 가 있으면 append. 모두 idempotent — 매 실행마다 안전하게 호출 가능.

**B. 두 호출로 분리 (폴더 미리 만들고 시간차로 채울 때)**
\`\`\`
1. danbi_create_folder(folder="stats")              ← 첫 날 한 번만 (idempotent)
2. danbi_append(domain="stats/2026-05-17.md", ...)  ← 그 안에 append
\`\`\`

### 기록 타이밍

- **즉시**: 위 트리거에 해당하는 순간이 오면 다른 일 끝나기 전에 바로 호출
- **세션 종료 시**: 대화가 자연스럽게 끝날 때 누락된 트리거가 있으면 몰아서
- **이중 기록 방지**: 같은 세션에서 동일 내용 중복 호출 금지 (체크 후 추가)

## Link — 언제 [[wiki-link]]를 넣는가

단비는 문서 간 연결이 쌓일수록 vault가 똑똑해진다. 그래프 뷰에서 확정 링크(실선)와 ghost 제안(점선)을 구분 렌더한다.

### 위키링크 문법

- 같은 프로젝트 내: \`[[파일명.md]]\`
- 다른 프로젝트: \`[[프로젝트명/파일명.md]]\`

### 언제 링크를 삽입하는가

${"`danbi_log`"}·${"`danbi_append`"} 로 기록할 때 다음 중 하나라도 해당되면 본문에 \`[[...]]\` 자연스럽게 포함:

1. 언급하는 개념이 기존 문서에 이미 다뤄졌음 (\`danbi_search\` 결과로 확인)
2. 같은 프로젝트 내 관련 결정/토픽이 있음
3. 답변 근거로 \`danbi_read\` 한 문서를 인용함

예시:
\`\`\`markdown
### JWT refresh 7일로 확정

세션 만료 UX 개선 위해 기존 1일 → 7일. 보안 이슈는 [[notes/auth-security.md]]
에 정리된 리스크 매트릭스 기준으로 수용 가능 범위로 판단. 관련 토큰 만료
처리는 [[notes/token-refresh-flow.md]] 참조.
\`\`\`

### Ghost 제안 처리

\`danbi_briefing\` 응답의 \`ghost_suggestions\` 는 단비가 제안한 "아직 확정 안 된 링크"다. 작업 중 해당 source 문서를 편집하게 되면, 제안된 target이 정말 관련 있는지 판단하고 ${"`[[target]]`"}을 본문에 삽입. 확정되면 그래프의 점선이 실선으로 바뀐다.

## 안티 패턴

- **❌ \`danbi_log\` 누락**: "이건 단비에 기록하기엔 너무 작은데..." — 작은 결정일수록 나중에 찾기 어렵다. 기록한다.
- **❌ 일반 지식으로 답**: "일반적으로 JWT는..." — 이 vault가 어떤 결정을 내렸는지가 중요하다. 먼저 검색.
- **❌ 링크 없이 기록**: 관련 문서가 있는데 \`[[링크]]\` 안 걺 — 그래프가 자라지 않음.
- **❌ project 파라미터 수동 지정**: 이 엔드포인트는 "${project}"로 clamp됨. 생략이 맞다.
<!-- danbi:end -->
`;
  return tpl;
}
