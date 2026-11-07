//! Per-project Claude Code SKILL.md template.
//!
//! The template lives in vault at `<project>/SKILL.md` so the user can
//! edit it directly through the sidebar. `{{PROJECT}}` and `{{MCP_URL}}`
//! are substituted at install time (see `commands::install_skill`).

pub const DEFAULT_SKILL_TEMPLATE: &str = r####"---
name: danbi-{{PROJECT}}
description: Use this skill when the user is working in the "{{PROJECT}}" project's Danbi vault. Triggers include past decisions ("어떻게 정했지", "예전에"), past debugging ("이 에러 본 적", "또 떴어"), session continuity ("어제 뭐 했지", "이어서"), or saving knowledge worth remembering. Also use when the user mentions "단비", "vault", "daily note", "{{PROJECT}}", or wiki-link cross-references. Reads run vault-wide; writes auto-clamp to "{{PROJECT}}".
---

# 단비 vault — "{{PROJECT}}" 의 장기 기억

단비 vault 는 "{{PROJECT}}" 프로젝트의 장기 기억이다. 다음 세션의 나(또는 다른 에이전트)가 일을 이어받을 때 필요한 것 — 왜 그렇게 정했는지, 무엇이 왜 깨졌었는지, 어디까지 했고 다음에 무엇을 할지 — 를 남기고 찾아 쓰는 곳이다. 언제 무엇을 읽고 쓸지는 판단에 맡긴다. 읽기는 vault 전체, 쓰기는 "{{PROJECT}}" 로 자동 고정된다.

MCP endpoint: `{{MCP_URL}}` (등록 안 돼 있으면 단비 사이드바 → 프로젝트 우클릭 → "Claude Code 설치 명령 복사").

- 시작할 때 `danbi_briefing` 으로 직전 맥락을, 답하기 전에 `danbi_context` 로 이미 정한 것을 본다. 더 필요하면 `danbi_read`·`danbi_related`·`danbi_query`·`danbi_search`·`danbi_daily`.
- 남길 것은 `danbi_log`(오늘 daily) 나 `danbi_append`·`danbi_replace_section`·`danbi_upsert_item`(주제 문서)으로. 많이보다 정확하게 — 틀린 기록은 덧쌓지 말고 고치거나 새 기록에 `대체:: [[옛 문서.md]]` 로 뒤집는다.
- 관련 문서는 `[[파일.md]]`(다른 프로젝트는 `[[프로젝트/파일.md]]`)로 잇고, 원인·영향처럼 뜻이 있는 연결은 `원인:: [[x.md]]` 처럼 관계 이름을 붙인다. 쓸 수 있는 이름은 `danbi_schema`.
- 일을 마칠 때 `danbi_tidy` 로 이 프로젝트의 정리 거리(깨진 링크·daily 에 묻힌 결정·기한 지난 문서)를 보고, 확실한 것만 `danbi_tidy_apply` 로 처리한다. 애매하면 두면 사람이 고른다.
- 마칠 때 `danbi_log` 의 `next_step` 에 다음에 제일 먼저 할 일 한 줄.
- 쓰기 도구에는 project 인자를 넣지 않는다. 나머지 도구와 인자는 도구 설명을 본다.
"####;

#[cfg(test)]
mod tests {
    use super::*;

    /// 주입 층은 작게 — 규칙이 길어지면 지시 준수가 떨어진다. 세부는 도구 설명으로.
    #[test]
    fn template_stays_small() {
        let chars = DEFAULT_SKILL_TEMPLATE.chars().count();
        assert!(chars < 2500, "SKILL 템플릿 {chars}자 — 세부는 도구 설명으로 내릴 것");
        assert!(DEFAULT_SKILL_TEMPLATE.contains("{{MCP_URL}}"));
        assert!(DEFAULT_SKILL_TEMPLATE.contains("danbi_context"));
    }
}
