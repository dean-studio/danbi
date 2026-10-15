# Changelog

## [Unreleased]

### Fixed

- **연결 관리를 열면 "화면을 그리다 문제가 생겼어요"로 멈추던 문제** — 연결 시트의
  "모든 프로젝트 Skill 갱신" 버튼이 렌더할 때마다 새 배열을 만들어 무한 렌더
  (React #185)에 빠졌습니다. 그룹·색 설정이 비어 있을 때 같은 문제가 날 수 있던
  프로젝트 홈·사용량 화면도 함께 고쳤습니다.

## [0.8.9] — 2026-10-15

### Removed

- **Quick Capture(⌃Space 전역 팝업) 제거** — 별도 창·전역 단축키·트레이 메뉴
  항목·설정의 "단축키" 메뉴를 없앴습니다. 메모는 홈 맨 위 빠른 메모(⌘J)에서
  적습니다. 전역 단축키 플러그인 의존성도 함께 뺐습니다.

### Changed

- **버벅임 완화** — 디스크·git 을 다루는 커맨드 102개를 화면을 그리는 메인
  스레드 밖에서 실행합니다(대시보드·활동 피드·Claude Code 사용량·검토함 등).
- **메모리 절감** — Claude Code 사용량 캐시 97MB → 18MB(같은 문자열 공유, 같은
  응답의 연속 줄을 저장 단계에서 합침). 집계 결과는 같습니다.

### Fixed

- **트레이 아이콘을 눌러도 팝오버가 안 뜨던 문제** — 포커스를 못 받은 팝오버가
  다른 데스크톱(Space)에 "열린 채"로 남아, 다음 클릭이 그걸 닫는 쪽으로 처리됐습니다.
  이제 열려 있고 포커스까지 있을 때만 닫고, 팝오버는 지금 보고 있는 데스크톱에
  뜹니다.
- **사용량 화면 기간 "오늘"에서 차트가 회색 덩어리로 보이던 문제** — 하루치
  데이터는 막대 하나가 폭 전체를 채웠습니다. 이제 그날의 시간대별 막대 24개로
  보여 줍니다.

## [0.8.8] — 2026-10-15

### Added

- **홈 = "오늘"** — 빠른 메모 입력, 에이전트 활동 피드, 검토함 요약, 지난 오늘,
  에이전트 연결 상태를 한 화면에 모았습니다. 피드의 각 행에서 문서를 열고,
  diff 를 펼쳐 보고, **그 변경만** 되돌릴 수 있습니다. 같은 문서에 이어진
  기록은 묶어서 보여 줍니다. 기존 사용량 카드는 "사용량" 화면(⌘⇧U)으로 옮겼습니다.
- **⌘K 통합 팔레트** — 문서·프로젝트·명령을 한 곳에서 찾습니다. `>` 명령,
  `@` 프로젝트 범위, `#` daily 만. ⌘P 는 `@` 모드로 열립니다.
- **문서 인스펙터 (⌘I)** — 변경 이력과 diff, 문서 단위 복원, 들어오고 나가는
  링크, ghost 링크 승인·무시를 문서 옆에서 처리합니다. 편집 중에는 ⌥⌘I.
- **검토함 (⌘⇧R)** — ghost 링크 제안, 고아·빈·큰 문서 정리 제안, 저신뢰 기록을
  한 목록에서 처리합니다. J/K 로 이동, A 승인, X 무시.
- **연결 시트** — 흩어져 있던 Claude Code·Codex·Cursor 연결·Skill·훅·토큰
  설정을 한 곳으로 모았습니다. Claude Code·Cursor 는 코드 폴더를 고르면
  `.mcp.json` / `.cursor/mcp.json` 에 읽기 서버와 이 프로젝트 전용 쓰기 서버를
  바로 기록합니다(토큰 파일은 `.git/info/exclude` 로 커밋 방지). 상태는
  연결됨 / 호출 대기 / 서버 꺼짐 / 미연결로 보여 줍니다.
- **단축키** — ⌘N 새 노트, ⌘⇧D 오늘 노트, ⌘J 빠른 메모, ⌘\\ 사이드바, ⌘1~9
  프로젝트, ⌘/ 단축키 목록.
- **MCP** — `danbi_read` 의 `heading`·`max_chars`·`offset`, 검색·최근·daily·
  브리핑의 `project` 범위, `danbi_search` 의 `mode`(keyword/semantic/hybrid),
  서버가 직접 사용법을 알려 주는 `instructions`. 임베딩 모델이 섞여 의미 검색에서
  빠지는 문서가 있으면 경고합니다.

### Changed

- **세션 시작 브리핑·프로젝트 목록 다이어트** — `danbi_briefing` 약 34k자 →
  4k자, `danbi_list_projects` 약 288k자 → 7k자. 응답 형태가 바뀌었으니 이 출력을
  직접 읽는 스크립트는 확인해 주세요(`danbi_search` 도 결과가 `{hits, …}` 객체).
- **`danbi_log` 기록에 시각·호출자 줄** (`_14:32 · claude-code_`) 이 첫 제목 아래
  자동으로 붙습니다. 직전 기록과 같은 본문(재시도)은 다시 쓰지 않습니다.
- **쓰기 서버에서는 `project` 를 넣지 않습니다** — 연결된 프로젝트로 고정됩니다.
- **사이드바** — 헤더 아이콘 8개 → 2개, 검색창, 최근 문서, 프로젝트 이름은 홈
  이동·▸ 는 펼침으로 분리.
- **자동 저장이 기본** — 1.5초 뒤 저장, 같은 문서의 연속 저장은 커밋 하나로.
- **vault git 스냅샷 범위** — 벡터 색인·캐시·설정(`config.json`)은 더 이상
  커밋하지 않습니다. 업데이트 후 첫 실행 때 정리 커밋이 하나 생깁니다(기존
  이력은 그대로). 예전 커밋의 `config.json` 에 MCP 토큰이 남아 있으니, vault
  저장소를 공유할 계획이 있다면 설정 → MCP 에서 토큰을 교체하세요.
- **검색 속도** — 문서 하나를 고친 직후 검색 약 2.7초 → 13ms, 관련 문서 조회
  240ms → 10ms. 쿼리 임베딩은 1.5초 안에 오지 않으면 키워드 결과로 대신합니다.

### Fixed

- **편집 중 에이전트가 쓴 내용을 자동 저장이 덮어쓰던 문제** — 이제 저장 전에
  디스크와 비교하고, 바뀌었으면 "다른 곳에서 바뀌었어요" 배너로 묻습니다.
- **문서를 바꿀 때 이전 문서 본문이 새 문서에 저장되던 문제.**
- **여러 에이전트가 같은 노트에 동시에 쓰면 한쪽이 사라지던 문제** — 모든 쓰기
  경로가 하나의 잠금을 씁니다. 커밋 실패는 오류 대신 경고로 알려 재시도 중복을
  막습니다.
- **되돌리기가 vault 전체를 되돌려 다른 에이전트의 기록까지 지우던 문제** —
  마지막으로 바뀐 문서만 되돌립니다.
- **검색 색인이 편집할수록 커지며 느려지던 문제**, 동시 검색 중 새 문서가
  빠지던 문제, 의미 검색 결과가 0건으로 나오던 문제.
- 한글 조합 중 Enter 가 검색 결과를 열던 문제, ⌘↵ AI 재정렬이 동작하지 않던 문제.
- 온보딩을 다시 하면 provider·모델 설정이 지워지던 문제, Gemini 키 저장 실패를
  알리지 않던 문제.
- MCP 토큰 교체 시 확인 없이 연결이 끊기던 문제 — 기록한 연결 파일도 새 토큰으로
  함께 갱신합니다.
- 활동 지표가 사전 스냅샷까지 세어 약 2배로 보이던 문제.

## [0.8.7] — 2026-10-13

### Fixed

- **Claude Code 사용량이 약 2배로 부풀려지던 문제** — Claude Code 는 응답
  하나를 블록(thinking·text·tool_use)마다 한 줄씩 기록하고 줄마다 같은
  사용량을 붙입니다. 재개한 세션은 다른 파일에 같은 응답을 다시 쓰기도
  합니다. 단비가 이 줄을 모두 더해 토큰이 약 2.1배로 잡혔습니다. 이제
  `message.id` + `requestId` 가 같은 줄은 응답 1건으로 보고 한 번만 셉니다.
  이미 저장된 일별 기록도 transcript 가 온전히 남아 있는 날짜는 다시 계산합니다.
- **macOS 27 다중 모니터에서 팝오버가 엉뚱한 화면 끝에 열리던 문제** —
  메뉴바 아이콘 좌표가 다른 모니터 기준으로 들어와서 생긴 문제입니다.
  이제 클릭한 마우스 위치로 모니터를 찾아 아이콘 아래에 엽니다.

### Added

- **Claude Code 다계정 사용량** — `CLAUDE_CONFIG_DIR=~/.claude-<계정>` 으로
  계정을 나눠 쓰는 경우 `~/.claude` 와 `~/.claude-*` 의 기록을 모두 읽어
  합산합니다. 사용량 카드에 계정별 목록을, 메뉴바 팝오버에 오늘 쓴 계정을
  보여 줍니다.
- **공유 스킬 폴더 설치** — `~/.claude-shared/skills` 가 있으면 단비 스킬을
  거기에도 설치해, 이 폴더를 공유하는 모든 계정에 한 번에 적용됩니다.

## [0.8.6] — 2026-09-15

### Fixed

- **Codex MCP 연결을 코드 폴더별로 완전히 격리** — 기존 연결 버튼은 사용자
  전역 `~/.codex/config.toml`에 모든 단비 MCP를 기록해, 서로 다른 폴더에서
  실행한 Codex A/B/C가 같은 `/mcp` 목록을 공유했습니다. 이제 연결할 코드
  폴더를 선택하면 해당 폴더의 `.codex/config.toml`에 그 단비 프로젝트의
  scoped endpoint만 기록합니다. Codex가 공식 지원하는 trusted project 설정
  계층을 사용하므로 다른 저장소에는 연결이 노출되지 않습니다.
- **기존 전역 Danbi MCP 자동 정리** — 새 폴더별 연결이 성공하면 단비가
  `# danbi:mcp ...` 마커로 설치했던 전역 블록을 모두 제거합니다. 사용자가
  직접 만든 MCP 설정과 마커 없는 서버는 건드리지 않습니다.
- **프로젝트 로컬 MCP 토큰의 Git 유출 방지** — 선택 폴더가 Git 저장소면
  `.git/info/exclude`에 정확한 로컬 config 경로를 추가합니다. 해당 config가
  이미 Git에서 추적 중이면 토큰을 쓰지 않고 연결을 중단합니다. 일반 저장소,
  하위 폴더, Git worktree의 common dir도 처리합니다.

### Changed

- 프로젝트 홈과 온보딩의 Codex 연결 안내를 전역 설정 방식에서
  “코드 폴더를 선택해 해당 폴더에만 연결”하는 흐름으로 변경했습니다.

## [0.8.5] — 2026-09-09

### Fixed

- **커스텀 vault 폴더를 쓰면 MCP·에이전트 연결이 엉뚱한 vault 를 보던
  split-brain 수정** — 읽기 경로(`current_vault_path()`·MCP 서버)는 항상
  고정 포인터 `~/Danbi_Vault/config.json` 의 `vault_path` 필드로 활성 vault 를
  찾는데, 온보딩에서 기본값이 아닌 폴더를 고르면 이 포인터가 갱신되지 않아
  연결 시 `project not found` 가 나거나 예전 vault 를 가리켰습니다. 이제
  `save_config()` 가 비-기본 위치에 쓸 때 고정 포인터에도 같은 config 를
  미러링해 두 경로가 어긋나지 않습니다.
- **온보딩에서 새 vault 를 골랐는데 이전 vault 의 프로젝트 그룹이 딸려오던
  문제** — 위저드가 기존 config 를 통째로 스프레드하면서 `project_groups`·
  아이콘·색상 등 vault-scoped 메타를 그대로 복사해, 빈 새 vault 에도 유령
  그룹 헤더가 보였습니다. vault 를 실제로 바꿀 때(`switchingVault`)만
  vault-scoped 필드를 초기화하도록 분리했습니다.
- **빈 vault 첫 화면에 남의 데이터처럼 보이던 홈 대시보드 정리** — 프로젝트가
  하나도 없을 때는 인사말을 "환영" 문구로 바꾸고, 분석 카드(Claude Code
  사용량·최근 활동·커밋 등)를 모두 숨겨 첫 프로젝트를 만들 때까지 깔끔한
  빈 상태를 보여줍니다.

### Added

- **온보딩 마지막에 "에이전트 연결" 안내 스텝 추가** — 단비의 정체성(외부
  AI 의 외부 뇌)을 처음부터 각인시키기 위해, 위저드 끝에서 **Codex** 와
  **Claude Code** 를 브랜드 아이콘과 함께 미리 보여줍니다. 실제 연결은 앱을
  열면 프로젝트 홈 상단 **ConnectCard** 에서 원클릭으로 하도록 유도 (위저드
  시점엔 MCP 서버·토큰이 아직 준비 전이라 실연결 불가).
- **프로젝트 홈 상단 ConnectCard + 브랜드 로고마크** — Codex(OpenAI knot)·
  Claude(terracotta burst) 공식 마크를 인라인 SVG(`AgentIcons`)로 두고,
  연결/Skill 액션을 카드 형태로 정리했습니다.

### Changed

- **설정 좌측 사이드바 하단의 `v0.2` 하드코딩 제거** — 잘못된 버전 문자열이
  고정 노출되던 자리를 삭제 (정확한 버전은 정보 패널의 런타임 표기 사용).

## [0.8.4] — 2026-09-07

### Fixed

- **Codex MCP 연결이 `failed (0 tools)` 로 끊기던 문제 — notification 응답
  규약 위반 수정** — `notifications/initialized` 처럼 JSON-RPC **notification**
  (`id` 없는 요청) 에 대해 서버가 `200 OK` + `{"id":null,…,"result":{}}` 라는
  응답 객체를 돌려주고 있었습니다. MCP Streamable HTTP 스펙은 notification 에
  **202 Accepted + 빈 본문** 을 요구하고, Codex(rmcp) 클라이언트는 이 위반을
  치명적 프로토콜 오류로 간주해 초기화 도중 세션을 끊었습니다. 이제 `id` 가
  없는 모든 요청은 202 빈 본문으로 응답합니다. (Claude Code 는 관대해서
  영향이 없었지만, 규약을 지키는 게 맞습니다.)

### Added

- **Codex 연결 = `~/.codex/config.toml` 직접 기록 (`codex_connect`)** — 기존엔
  붙여넣을 TOML 블록을 클립보드에 복사했는데, 반복해서 누르면 `[mcp_servers.…]`
  블록이 파일에 중복으로 쌓일 위험이 있었습니다. 이제 "연결" 이 파일에 바로
  기록하고, **이미 있는 Danbi 블록은 그 자리에서 교체** 합니다 (멱등). 다른
  서버 항목·주석 등 나머지 내용은 그대로 보존하는 라인 기반 병합이라, 몇 번을
  눌러도 블록이 스택되지 않습니다.

### Changed

- **설정 → 정보 패널 버전 표기 `v0.3.0` 잔존 수정** — v0.8.2 에서
  `getVersion()` 훅(`useAppVersion`)을 도입했지만 About 다이얼로그만 반영되고
  **Settings 정보 패널의 헤더 문자열은 여전히 `v0.3.0` 하드코딩**이 남아
  있었습니다. 이제 이 자리도 런타임 번들 버전을 읽습니다.
- **프로젝트 홈 헤더를 에이전트별 2행으로 재구성** — 연결/Skill 을 하나로 묶던
  드롭다운을 없애고, **Codex** 와 **Claude Code** 를 각각 한 줄씩
  `[연결] [Skill] [Skill 수정]` 3버튼으로 펼쳐 두 에이전트를 한눈에 볼 수 있게
  했습니다. 잘 안 쓰이던 **주간 회고** 버튼은 헤더에서 제거 (기능 코드는 보존).

## [0.8.3] — 2026-09-02

### Fixed

- **메뉴바 팝오버 버벅임 — 활성 transcript 전체 재파싱 제거** — 팝오버를
  열거나 창에 포커스가 갈 때마다 `dashboard_claude_code("today")` 가
  `~/.claude/projects/**/*.jsonl` 중 **mtime/size 가 바뀐 파일을 통째로**
  다시 파싱하고 있었습니다. Claude Code 가 활성 세션 transcript 에 계속
  append 하는 탓에, 하루 300M+ 토큰을 쓰는 날엔 활성 파일 합계 400MB 이상을
  매 focus 마다 처음부터 JSON 파싱 + 전체 이벤트 정렬 → 수백 ms 스파이크로
  이어졌습니다 (write-lock 을 잡은 채라 본체 사용량 카드까지 대기).
  - transcript 는 append-only 라는 점을 이용해 **증분 tail 파싱** 으로 전환.
    `FileEntry` 에 마지막으로 소비한 오프셋(`parsed_bytes`)을 저장하고, 다음
    갱신 때 그 지점부터 새로 추가된 바이트만 파싱합니다. 파일이 줄거나
    회전되면 전체 재파싱으로 안전하게 폴백. 450MB 재파싱 → 수 KB 수준.
  - `refresh_cache` 를 (1) read-lock 스냅샷 → (2) 락 밖 델타 파싱 →
    (3) 짧은 write-lock 반영 3단계로 재구성해 락 점유 시간을 최소화.
    반영 시 오프셋이 어긋나면 델타를 버려 이중 집계를 막습니다.
  - 팝오버는 창 focus 재조회에 8초 throttle 을 걸어 focus 연타 시 백엔드를
    반복 호출하지 않습니다.
  - 같은 경로를 쓰는 **본체 Claude Code 사용량 카드도 함께 빨라집니다.**

## [0.8.2] — 2026-09-02

### Added

- **Codex 연결·Skill 지원 (프로젝트 홈)** — 프로젝트 홈 헤더의 "연결"·
  "Skill" 버튼을 **에이전트 드롭다운**으로 바꿔 Claude Code / Codex 를 각각
  선택할 수 있습니다.
  - **연결** — Claude Code 는 기존 `claude mcp add --transport http …` 명령을
    복사하고, Codex 는 `~/.codex/config.toml` 에 붙여넣을
    `[mcp_servers.danbi-<slug>]` 블록(`url` + `[…http_headers]` 에
    `Authorization = "Bearer …"` 인라인)을 복사합니다. Codex config 는 인라인
    `bearer_token` 을 거부하지만 `http_headers` 는 정식 지원해서, 환경변수 없이
    토큰을 한 번에 넘길 수 있습니다.
  - **Skill** — Claude Code(`~/.claude/skills/`) / Codex(`~/.codex/skills/`)
    별로 SKILL.md 를 설치·갱신하고, 각 항목에 설치 여부를 표시합니다. vault 의
    `<project>/SKILL.md` 는 두 에이전트 공용 원본이라 "Skill 수정" 은 그대로
    한 곳만 편집하면 됩니다.

### Changed

- `install_skill` / `skill_status` 커맨드에 `agent`(`claude` | `codex`)
  파라미터 추가. 기존 단일 인자 호출부(홈의 일괄 설치, 사이드바 우클릭)는
  기본값 `claude` 로 동작이 그대로 유지됩니다.

### Fixed

- **설정 → 정보 · About 다이얼로그의 버전 표기 고정 0.3.0 문제** — 두 곳
  모두 버전 문자열이 `0.3.0` 으로 하드코딩돼 실제 릴리즈와 어긋나 있었습니다.
  이제 `@tauri-apps/api/app` 의 `getVersion()` 으로 런타임에 실제 번들 버전을
  읽어(`useAppVersion` 훅), `tauri.conf.json` 을 단일 진실의 원천으로 삼습니다.
  다음부터 버전을 올려도 두 화면이 자동으로 따라옵니다.

## [0.8.1] — 2026-08-25

### Fixed

- **"Intel 기반 앱 지원 종료" 경고 — Apple Silicon 전용 빌드 고정** — PATH
  에서 Intel Homebrew rust(`/usr/local/bin/cargo`, host x86_64)가 rustup
  보다 먼저 잡혀 릴리즈 DMG 가 x86_64 로 빌드되던 문제. 설치된 앱이 Intel
  슬라이스라 macOS 가 미래 버전 미지원 경고를 띄웠습니다.

### Changed

- `package.json` 의 `bundle` 스크립트가 PATH 에 `~/.cargo/bin`(rustup)을
  먼저 얹고 `--target aarch64-apple-darwin` 을 명시하도록 변경.
- `src-tauri/rust-toolchain.toml` 추가 — 채널·타겟을 arm64 로 고정.
- README: "Apple Silicon · Intel" → "Apple Silicon" 전용 명시.

## [0.8.0] — 2026-08-25

### Changed

- **토큰 비용 계산 전면 제거 — 사용량 카드는 토큰량만 표시** — USD/KRW
  환산, 단가표, 환율 설정, 결제 모드(Bedrock/API/구독) 분기, 참고용 비용
  추정을 모두 걷어냈습니다. 부정확한 환산이 오히려 신뢰를 떨어뜨린다는
  판단. Claude Code 사용량 · 단비 자체 LLM 사용량 · MCP 저장 토큰 카드는
  그대로 유지되며, 이제 **토큰량만** 노출합니다.

### Removed

- 백엔드: `pricing.rs` 단가·비용 로직(토큰 집계만 남김), `Totals` 의
  usd/krw 필드, `UsageConfig` 의 `krw_per_usd`·`claude_code_mode`,
  `mcp_inbound.rs` 의 `CostEstimate`, billing history 의 `pricing_version`.
- IPC 커맨드 4개 삭제: `usage_set_claude_code_mode`, `usage_set_krw_rate`,
  `usage_month_to_date`, `usage_set_rate`. `dashboard_claude_code` 는 이제
  `CcSummary` 를 직접 반환(`enabled` 플래그 포함).
- 프론트: `CcSummaryWithMode`/`CcEffectiveMode`/`McpCostEstimate` 타입,
  카드·popover 의 ₩/$ 표기와 ModeBadge, Settings 의 결제 모드 셀렉트 +
  KRW 환율 입력. 재인덱싱 예상치는 비용 대신 처리 토큰량으로 표시.

## [0.7.2] — 2026-07-14

### Fixed

- **"로고 → 검은 화면" 치명 버그 (ErrorBoundary 부재)** — 신규 설치 후
  온보딩을 마치면 splash 로고만 뜨고 메인 윈도우가 검은 화면으로 멈추는
  현상. tray 의 "단비 열기" 도 같은 메인 윈도우를 여니 동일 증상이었습니다.
  원인은 앱 전체에 React ErrorBoundary 가 하나도 없던 것 — 렌더 도중
  어디서든 예외가 한 번 나면 React 가 트리 전체를 unmount 해버려서
  `index.html` 의 검은 배경(`#07080a`) 만 남고, 진단 정보도 전혀 남지
  않았습니다.

### Added

- **전역 ErrorBoundary** (`src/components/ErrorBoundary.tsx`) — 메인·
  popover·capture 세 윈도우 루트를 모두 감쌉니다. 이제 렌더 예외가 나도
  검은 화면 대신 오류 메시지·스택·component stack 을 보여주고, "오류
  복사 / 다시 불러오기 / 닫고 계속" 버튼을 제공합니다. IPC·store·theme
  유틸에 전혀 의존하지 않는 순수 인라인 구현이라, 그 유틸들이 예외
  원인이어도 안전하게 그려집니다.
- **홈 사용량 카드 개별 격리** — v0.7.0 에서 추가된 Claude Code 사용량 ·
  단비 자체 LLM 사용량 · MCP inbound 카드를 각각 개별 ErrorBoundary 로
  감싸, 한 카드가 예기치 못한 데이터에 터져도 사이드바·에디터는 그대로
  살아있고 그 카드 자리에만 "다시 시도" 안내가 뜹니다.

## [0.7.1] — 2026-06-24

### Fixed

- **단가표 전면 보정 (Bedrock cross-region 기준)** — v0.7.0 은 Anthropic
  API 단가 ($15/$75) 를 그대로 썼지만 사용자 환경은 AWS Bedrock cross-region
  inference 이고 단가가 정확히 1/3 ($5/$25) 입니다. 이로 인해 v0.7.0 의
  Opus 4.x 비용 추정이 약 **3× 과대 계산**되고 있었습니다. AWS 공식 가격표로
  교체:
  - Opus 4.x: input $5 / output $25 / 5m cache $6.25 / **1h cache $10** /
    cache_read $0.50 (모두 per 1M)
  - Sonnet 4.6/4.5: $3 / $15 / $3.75 / $6 / $0.30
  - Haiku 4.5: $1 / $5 / $1.25 / $2 / $0.10
  - Fable 5: $10 / $50 / $12.50 / $20 / $1.00
- **`claude-opus-4-8` 가격표 누락 수정** — v0.7.0 에서 Opus 4.8 모델이
  단가표에 없어 무료(0원) 로 계산되던 버그. 단비 카드가 ₩12만으로 표시
  됐던 06-22 의 진짜 청구액은 ₩75만 수준이었습니다.
- **5m vs 1h 캐시 분리 단가 적용** — `cache_creation` 의 breakdown
  (`ephemeral_5m_input_tokens` / `ephemeral_1h_input_tokens`) 을 읽어
  단가 다르게 적용. 1h 캐시는 5m 보다 ~1.6× 비싸요.
- **cache 단가를 절대값으로 정의** — 기존엔 `input × 1.25` / `input × 0.10`
  배수로 계산했는데, AWS 공식은 절대 USD/1M 값. 우연히 비율이 같아 결과
  같았지만 코드 표현은 정공법으로 교체.

### Added

- **영속 비용 히스토리** (`~/.danbi/cc_billing_history.jsonl`) — 어제까지
  의 일별 합계를 한 번 계산한 후 디스크에 finalize. 다음 카드 갱신 부터는
  과거 날짜는 transcript 재집계 없이 history 에서 즉시 로딩, 오늘만 실시간.
  v0.7.0 까지는 매번 1년치 transcript (수십 MB) 를 풀스캔했지만 v0.7.1 부터
  카드 갱신은 "오늘 분량만" 스캔합니다.
- **`pricing_version` 필드 박힘** — finalize 된 row 가 어떤 단가표로
  계산됐는지 기록. 향후 단가 정정으로 과거 재계산 vs 그대로 둘지 선택 가능.
- **Settings → "히스토리 초기화" 버튼** — 단가표가 바뀌었을 때 사용자가
  한 번 눌러 모든 과거 비용을 새 단가로 재 finalize 가능.

### Changed

- `pricing.rs` `PriceUsdPerMTok` struct 에 `cache_5m` / `cache_1h` /
  `cache_read` 필드 추가. 기존 모델 (OpenAI/Gemini/Voyage/Embeddings) 은
  `flat()` 헬퍼로 동일 동작 유지.
- 카드 disclaimer 문구를 절대 단가 적용 사실로 갱신.

## [0.7.0] — 2026-06-23

### Added

- **Claude Code 사용량 카드 (홈)** — `~/.claude/projects/**/*.jsonl`
  transcript 를 직접 읽어 일별 토큰·비용·세션·모델/프로젝트별
  분포·시간대 히트맵을 보여줘요. OAuth endpoint 호출 없이 자기 디스크의
  자기 파일만 사용 (권한 이슈 0). transcript 의 `message.id` 가
  `msg_bdrk_` 로 시작하면 Bedrock, 아니면 Anthropic API 로 자동 감지.
- **결제 모드별 UI 분기** — Bedrock / Anthropic API key (종량) 사용자는
  토큰 + USD/KRW 비용. Pro/Max 구독 사용자는 비용을 숨기고 토큰만 표시.
  Settings → "Claude Code 사용량 → 결제 모드" 에서 명시 선택 가능.
- **히스토리 탭** — 90일 sparkline, 1년 GitHub-잔디, 월별 추이 (12개월),
  1년 전 오늘 비교, 가장 비쌌던 날 Top 5. transcript 가 1년치 모이면
  자동으로 비교가 채워집니다.
- **단비 자체 LLM 사용량 카드** — 단비 본 앱이 라우팅/writer/embed 등으로
  Bedrock·기타 provider 에 쏜 호출의 토큰. 응답 헤더 → 단가표 → KRW.
  Claude Code 카드와 데이터 소스가 정확히 다름 (jsonl vs usage.jsonl).
- **메뉴바 popover 사용량 mini** — 메뉴바 아이콘 클릭 시 popover 에 오늘
  Claude Code 사용량 (큰 숫자 + KRW + input/output/cache 3-bar) 한 줄.
  Settings → "메뉴바 → popover 활성화" 로 OFF 가능.
- **transcript 캐시 강제 무효화** — Settings 에 "다시 인덱싱" 버튼.
  파일 mtime 비교 캐시는 보통 자동이지만, 외부에서 jsonl 을 손댄 경우
  강제로 풀스캔하고 싶을 때 사용.
- **USD → KRW 환율 편집** — Settings 에서 환율 직접 수정. 분기 갱신.

### Changed

- `pricing.rs` 가격표에 캐시 토큰 보정 단가 적용 — `cache_creation` 은
  input × 1.25, `cache_read` 는 input × 0.10. (Anthropic 공식 단가)
- 메뉴바 popover 높이 345 → 415 (사용량 mini 카드 공간).

## [0.6.0] — 2026-06-20

### Added

- **`danbi_replace_section` MCP 도구** — 지정한 markdown 헤딩 아래 본문
  전체를 새 내용으로 교체. 같은 섹션이 자주 갱신되는 문서를 append 로
  쌓지 않고 깔끔하게 갱신할 수 있어요. 헤딩이 없으면 EOF 에 새 섹션을
  추가합니다.
- **`danbi_upsert_item` MCP 도구** — 리스트 섹션 안 한 항목을 in-place
  교체 (없으면 추가). `[#id]` 마커 또는 항목 첫 줄 텍스트로 매칭. 진행
  상태가 바뀌는 체크리스트·카드 리스트에 적합. 응답에
  `mode: "update" | "add"` 가 실려서 어떤 의미였는지 분간 가능.
- 프로젝트별 SKILL.md 가이드 갱신 — 외부 Claude 세션이 append vs
  replace_section vs upsert_item 중 어느 도구를 골라야 하는지 표로 명시.
- **frontmatter `kind: list` 인지** — 사이드바 도메인 row 에 LIST 배지
  자동 노출. 외부 LLM 이 replace/upsert 로 갱신하는 문서임을 시각화.
- **변경 히스토리 popover** — 문서 헤더에 "변경 N건" 버튼. 외부 LLM 의
  upsert/replace 까지 op 별 색 토큰으로 분간해서 시간순 노출.
- **메뉴바 popover 빠른 메모** — 가장 active 한 프로젝트의 오늘 daily
  노트로 한 줄 메모 즉시 append. ⌘↵ 또는 자동 enter.
- **`danbi_log_quick` IPC** — popover 빠른 메모용 직호출 경로 (외부 LLM
  트래픽으로 잡히지 않음).
- **프로젝트 switcher ⌘P** — fuzzy match 로 키보드만으로 프로젝트 전환.
- **자동 self-healing 알림** — BriefingCard 가 윈도우 focus 시 자동
  refresh, 새 ghost / healing 제안이 들어오면 알림 toast.
- **DocView 헤더 그래프 버튼** — 현재 프로젝트를 spotlight 한 채로
  그래프 뷰 열기 (양방향 인터랙션 완성).
- **`summarize_weekly` 명령** — 지난 7일 daily 노트를 모아 한 단락 회고
  생성 → `weekly/YYYY-WW.md` 에 저장. ProjectHome 헤더의 "주간 회고"
  버튼.

### Changed

- 사이드바 sub-folder 가 프로젝트 펼칠 때마다 자동으로 다 열려있던
  동작을 모두 닫힌 상태로 시작. 현재 선택된 도메인을 포함한 폴더만
  자동 expand. (사용자 맞춤 토글은 그 세션 동안 유지.)
- REST 호출 (`/api/call/danbi_search`) 도 JSON-RPC 와 동일하게 RRF
  하이브리드 fast-path 사용 — 임베딩 provider 가 cfg 에 있으면 BM25 +
  벡터 결과를 RRF 로 병합해서 자연어 쿼리 정확도 일관성 확보.

### Fixed

- 에디터에서 텍스트 복사 시 줄끝의 CommonMark hard-break (`\` + 개행) 가
  다른 메모/문서 앱에 그대로 보이는 문제. 이제 클립보드 text/plain
  슬롯에서 trailing `\` 가 평범한 줄바꿈으로 정리됩니다.

## [0.5.0] — 2026-06-09

### Added

- **프로젝트별 Goal** — 각 프로젝트 대시보드에서 active goal 등록/archive.
  외부 Claude 세션은 MCP 응답에 자동 노출돼서 항상 인지.
- **프로젝트 활동 분포 카드** — 홈에서 30/7/90일 단위 도넛 + 랭킹 바.
- **메뉴바 popover 정리** — 단비 열기 + 최근 활동 프로젝트 퀵셔트 + 종료 버튼.
- **첫 진입 splash** — 사이드바 노출 전 단비 로고만 보이는 로딩 화면.

### Changed

- **Cmd+Q 가 종료하지 않음** — 메인 윈도우만 숨기고 tray + MCP 유지. 진짜
  종료는 popover 의 종료 버튼 / tray 의 Quit.

## [0.4.0] — 2026-06-02

### Added

- **MCP 저장 토큰 대시보드** — 외부 Claude/Cursor 세션이 MCP 로 vault 에
  쓴 토큰량을 프로젝트·도메인 단위로 집계해서 대시보드에 노출.
  사이드바에서 프로젝트 선택 시 미니 카드로도 확인 가능.

## [0.3.1] — 2026-05-27

### Fixed

- **Apple notarization 적용** — 0.3.0 부터 자동 업데이트 인프라가 들어왔지만
  Apple Developer 인증서 + notarytool 단계가 누락돼 있었습니다. 이제
  `Developer ID Application` 으로 codesign + Apple notarytool 검증 + stapler
  staple 까지 빌드 파이프라인에 포함됩니다. 다른 Mac 에서 받아도
  "확인되지 않은 개발자" 경고 없이 바로 열립니다.
- 자동 업데이트 페이로드 (`.app.tar.gz`) 도 stapled 빌드를 기준으로
  재패키징/재서명되도록 `scripts/release.sh` 에서 처리.

### Build

- `entitlements.plist` 추가 — hardened runtime 에서 Tauri webview 가
  정상 동작하도록 JIT/library validation/network 권한 명시.
- `scripts/release.sh` 가 codesign + notarize + staple + ed25519 재서명을
  순서대로 자동 수행.

## [0.3.0] — 2026-05-26

### Added

- **프로젝트별 고유 색** — 사이드바 우클릭 → 색 선택으로 프로젝트마다 고유
  accent 를 지정. 선택된 색은 전체 스코프에 동시에 적용됩니다:
  - 프로젝트 헤더 active 배경 + chevron
  - 펼친 영역 좌측 2px accent strip
  - 활성 도메인 행 배경·아이콘·타이틀 미리보기
  - 하위 폴더 아이콘 (Chevron + Folder/FolderOpen) 전부
  - 폴더 변경 카운트 배지·프로젝트 update 배지·도메인 modified dot
  ("new" 도메인 dot 은 의미 보존 위해 그린 유지)
  색은 키 (`"purple"` 등) 로 `config.json` 에 저장 — 라이트/다크 테마 전환
  시 별도 마이그레이션 없이 토큰만 자동 스왑.
- **자동 업데이트** — 앱 시작 후 3초 idle 에 GitHub Releases 의 `latest.json`
  을 조회 (24시간 throttle). 새 버전이 있으면 사이드바 footer 에 pill 로
  안내, 클릭 시 백그라운드 다운로드 → 진행률 표시 → 완료되면 자동 재시작.
  `tauri-plugin-updater` + `tauri-plugin-process` 사용. Settings → 정보 의
  "지금 확인" 버튼으로 수동 강제 체크도 가능.
- **ed25519 코드 서명** — 업데이트 번들은 `~/.tauri/danbi.key` 로 서명되며
  공개키는 `tauri.conf.json` 에 임베드. 빌드 시 `TAURI_SIGNING_PRIVATE_KEY_PATH`
  환경변수로 로드. 사용자가 받는 `.app.tar.gz` 와 `.sig` 는 GitHub Release
  에 함께 업로드.

### Build

- `pnpm bundle` 결과물에 `.app.tar.gz` + `.sig` 가 추가로 생성됨
  (`createUpdaterArtifacts: true`). 릴리즈 시 dmg + tar.gz + sig + latest.json
  4종을 GitHub Release 에 첨부.

## [0.2.0] — 2026-05-21

첫 공개 빌드. 0.1 내부 개발판에서 누적된 변경을 정리하고, "AI 가 vault 의
주인" 이라는 정체성을 굳히기 위한 가시성·기본값 정리에 집중했습니다.

### Added

- **Settings → 외관: 변경 표시 마스터 ON/OFF** — 메뉴바 뱃지·사이드바 dot·
  프로젝트 변경 카운트 3축을 한 번에 토글하는 상단 카드.
  세 항목은 마스터가 ON 일 때만 보입니다.
- **Settings → 단축키: 마스터 ON/OFF** — Quick Capture 글로벌 단축키를 한
  번에 끄거나 켭니다. OFF 일 때는 키 조합 입력·규칙 박스가 모두 숨겨져
  화면이 단순해집니다. ON 으로 전환하면 기본값 `Control+Space` 가 적용되며
  사용자가 직접 다른 조합을 녹화할 수 있습니다.
- **사이드바 프로젝트 hover 화살표** — 그룹 안 프로젝트에 마우스를 올리면
  ↑/↓ 버튼이 나타나 한 번에 위/아래로 이동. 끝줄에서는 자동 비활성화.
- **Daily 노트 자동 폴더링** — 오래된 daily 노트가 `daily/<연도>/` 로
  자동 정리됩니다.
- **Token usage 통계 스크립트** — `scripts/token-stats.py` 실행 시 단비 ON
  세션 vs OFF 세션의 토큰·비용 차이를 vault 에 스냅샷으로 누적.

### Changed

- **첫 설치 시 Quick Capture 단축키는 OFF** — macOS 한국어 IME 토글이
  Control+Space 와 충돌하던 문제를 회피하기 위해, 디폴트값을 빈 문자열로
  변경. 사용자가 Settings 에서 명시적으로 켜야 등록됩니다.
  (`ShortcutsConfig::default`, `apply_capture_shortcut`, Onboarding 모두 동기화)
- **테마 = 다크 고정** — Settings 외관에서 테마 선택 UI 를 제거. 단비는
  다크 모드 기준으로 디자인되며, 라이트는 추후 재도입 시점에서 다시.
- **컴팩트 레이아웃 옵션 제거** — 사용 빈도가 낮고 단축키 마스터 토글과
  의미가 겹쳐 외관 패널에서 삭제.
- **사이드바 알림 종 뱃지 위치 보정** — 종 아이콘 우상단에 떠 있던 카운트가
  살짝 잘려 보이던 문제 해결 (-2px down + 왼쪽으로 이동).
- **Notes sub-folder 는 기본 접힘** — 프로젝트를 펼칠 때 `notes/` 폴더는
  닫힌 상태에서 시작합니다. 폴더가 너무 많이 펼쳐져 트리가 길어지던 문제
  해결. 사용자가 펼치면 그 세션 동안만 유지.
- **모델 가격 테이블 분기 갱신** — Gemini 2.5 Flash/Pro, Voyage 3 family,
  OpenAI o4-mini 가격을 추가. `gemini-embedding-001` 은 무료 표기.
- **About 패널 정리** — "제작 도움" 섹션에서 Claude 표기 제거.
- **KRW/USD 환율 디폴트 1400 → 1380** — 비용 계산 표시 갱신.

### Internal

- **Visibility 가드 = 기존 백엔드 재사용** — 처음 시도했던 localStorage
  기반 lastSeen 맵 신설을 폐기하고, 이미 동작 중인 `domainUpdates` /
  `projectUpdates` 위에 cfg flag 가드만 얹는 방향으로 통일. source of truth
  단일화로 유지보수 비용 절감.
- **`Vault → daily/YYYY/`** 자동 마이그레이션 — 오래된 daily 노트는 연도
  폴더로 이동. 그래프·검색은 모두 동일하게 동작.

## [0.1.0] — 내부 개발판

- Phase 1–5 (Interface · Routing · Ingestion · Final Sync · Multi-provider) 완성
- Phase A (Quick Wins) — 라이트 모드, 사이드바 폰트
- Phase D (Visibility) — 브리핑 대시보드, 그래프 뷰
- Phase E (MCP Expansion) — `danbi_briefing` · `danbi_daily` 도구
- Phase F (Wiki-LLM Loop) — 관련 문서 자동 grounding
- Phase H (Local Backup) — 단방향 vault mirror
- Phase J (llm_wiki Parity) — Purpose/Schema · Relevance Model · Louvain
  community · Graph Insights · 멀티 provider 6종 · Review 시스템 ·
  벡터 검색 · 크래시 복구 큐

[0.2.0]: https://github.com/dean-studio/danbi/releases/tag/v0.2.0
