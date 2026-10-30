/**
 * 온톨로지(문서 종류 · 관계 · 상태) 한글 라벨과 그래프 색 — 화면 전체가
 * 이 파일 하나만 본다. 프로젝트 schema.md 가 준 `ko` 가 있으면 그게 우선이고,
 * 모르는 값은 원래 이름 그대로 보여준다.
 */
import type { OntologySchema } from "@/lib/ipc";

export const DEFAULT_TYPE_KO: Record<string, string> = {
  decision: "결정",
  bug: "버그",
  feature: "기능",
  note: "메모",
  reference: "참고",
  person: "인물",
  daily: "일지",
  meta: "메타",
};

export const DEFAULT_RELATION_KO: Record<string, string> = {
  affects: "영향",
  affected_by: "영향받음",
  caused_by: "원인",
  causes: "결과",
  supersedes: "대체",
  superseded_by: "대체됨",
  relates_to: "관련",
  implements: "구현",
  implemented_by: "구현됨",
  blocks: "차단",
  blocked_by: "차단됨",
  part_of: "부분",
  has_part: "포함",
};

export const DEFAULT_STATUS_KO: Record<string, string> = {
  active: "유효",
  superseded: "대체됨",
  done: "완료",
};

/** 기본 관계의 역방향 — 들어오는 링크를 내 문서 쪽에서 읽을 때. */
const DEFAULT_INVERSE: Record<string, string> = {
  affects: "affected_by",
  affected_by: "affects",
  caused_by: "causes",
  causes: "caused_by",
  supersedes: "superseded_by",
  superseded_by: "supersedes",
  relates_to: "relates_to",
  implements: "implemented_by",
  implemented_by: "implements",
  blocks: "blocked_by",
  blocked_by: "blocks",
  part_of: "has_part",
  has_part: "part_of",
};

/** 역방향 관계 이름. schema 의 inverse 우선, 모르면 그대로. */
export function inverseRelation(name: string, schema?: MaybeSchema): string {
  const hit = schema?.relations.find((r) => r.name === name);
  if (hit?.inverse) return hit.inverse;
  return DEFAULT_INVERSE[name] ?? name;
}

/** 기본 종류 표시 순서 (범례·필터). */
export const DEFAULT_TYPE_ORDER = Object.keys(DEFAULT_TYPE_KO);

type MaybeSchema = OntologySchema | null | undefined;

function fromSchema(
  list: Array<{ name: string; ko?: string | null }> | undefined,
  name: string,
): string | null {
  const hit = list?.find((t) => t.name === name);
  return hit?.ko ? hit.ko : null;
}

export function typeLabel(name: string, schema?: MaybeSchema): string {
  return fromSchema(schema?.types, name) ?? DEFAULT_TYPE_KO[name] ?? name;
}

export function relationLabel(name: string, schema?: MaybeSchema): string {
  return (
    fromSchema(schema?.relations, name) ?? DEFAULT_RELATION_KO[name] ?? name
  );
}

export function statusLabel(name: string, schema?: MaybeSchema): string {
  return fromSchema(schema?.statuses, name) ?? DEFAULT_STATUS_KO[name] ?? name;
}

/** 여러 프로젝트 schema 를 한 번에 볼 때 — 첫 번째로 ko 를 가진 것을 쓴다. */
export function pickLabel(
  kind: "type" | "relation" | "status",
  name: string,
  schemas: Iterable<OntologySchema>,
): string {
  for (const s of schemas) {
    const list =
      kind === "type" ? s.types : kind === "relation" ? s.relations : s.statuses;
    const ko = fromSchema(list, name);
    if (ko) return ko;
  }
  if (kind === "type") return DEFAULT_TYPE_KO[name] ?? name;
  if (kind === "relation") return DEFAULT_RELATION_KO[name] ?? name;
  return DEFAULT_STATUS_KO[name] ?? name;
}

// ---- 그래프 색 (시각화 내부 전용 — chrome 에 쓰지 말 것) ----

/** 기본 종류 → 색. 토큰 accent 4색은 cssToken 이름, 나머지는 고정 hex. */
const TYPE_COLOR: Record<string, { token?: string; hex?: string }> = {
  decision: { token: "accent-blue" },
  bug: { token: "accent-red" },
  feature: { token: "accent-green" },
  reference: { token: "accent-yellow" },
  person: { hex: "#b47cff" },
  note: { hex: "#94a3b8" },
  meta: { hex: "#5eead4" },
  daily: { hex: "#5b6470" },
};

/** schema.md 로 추가된 종류용 — 이름 해시로 고정. */
const EXTRA_TYPE_PALETTE = [
  "#fb7185",
  "#ff8c42",
  "#38bdf8",
  "#facc15",
  "#4ade80",
  "#c084fc",
  "#f472b6",
  "#7fffd4",
];

/** 모르는 종류 — 중립 회색 (`{colors.ash}`). */
export const UNKNOWN_TYPE_COLOR = "#6a6b6c";

/**
 * 종류 색. `known` 이 false 면(어느 schema 에도 없는 값) 중립색.
 * `token` 은 CSS 변수 해석 함수 — 라이트 모드 accent 를 따라가게.
 */
export function typeColor(
  name: string,
  known: boolean,
  token: (name: string) => string,
): string {
  const def = TYPE_COLOR[name];
  if (def?.token) return token(def.token);
  if (def?.hex) return def.hex;
  if (!known) return UNKNOWN_TYPE_COLOR;
  let h = 0;
  for (const ch of name) h = (h * 31 + ch.charCodeAt(0)) | 0;
  return EXTRA_TYPE_PALETTE[Math.abs(h) % EXTRA_TYPE_PALETTE.length];
}

/** 상태별 노드 투명도 — 대체된 문서는 흐리게, 완료는 살짝. */
export function statusAlpha(status: string | undefined | null): number {
  if (status === "superseded") return 0.28;
  if (status === "done") return 0.6;
  return 1;
}

/** "project/domain" → [project, domain]. domain 에 '/' 가 있어도 첫 칸만 자른다. */
export function splitDocId(id: string): [string, string] {
  const i = id.indexOf("/");
  if (i < 0) return ["", id];
  return [id.slice(0, i), id.slice(i + 1)];
}
