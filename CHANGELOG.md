# Changelog

단비의 모든 의미 있는 변경은 여기에 기록합니다.

형식은 [Keep a Changelog](https://keepachangelog.com/) 를 따르고, 버전 체계는
[Semantic Versioning](https://semver.org/) 입니다.

## [Unreleased]

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
