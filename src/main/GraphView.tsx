import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import ForceGraph2D, {
  type ForceGraphMethods,
  type LinkObject,
  type NodeObject,
} from "react-force-graph-2d";
import { Filter, GitBranch, Lightbulb, Palette, Search, X } from "lucide-react";
import {
  ipc,
  type GhostSuggestion,
  type GraphData,
  type GraphInsights,
  type OntologySchema,
} from "@/lib/ipc";
import {
  DEFAULT_TYPE_ORDER,
  pickLabel,
  statusAlpha,
  typeColor,
} from "@/lib/ontology";
import { cn } from "@/lib/utils";
import { useApp } from "@/state/store";

// The force-graph library mutates node/link objects to add x/y/vx/vy at
// runtime. We type those loose fields explicitly so TS stays honest.
type GNode = NodeObject & {
  id: string;
  project: string;
  domain: string;
  label: string;
  bytes: number;
  community: number;
  degree: number;
  doc_type: string;
  status?: string;
  color?: string;
  /** 관계 필터가 켜졌을 때 보이는 선에 닿지 않는 노드 — 흐리게. */
  dimmed?: boolean;
  // When set, pins the node to (fx, fy) — used to anchor the hub at the
  // origin so the layout reads as radial around it.
  fx?: number;
  fy?: number;
  isHub?: boolean;
};

type GLink = LinkObject & {
  source: string | GNode;
  target: string | GNode;
  kind: "confirmed" | "ghost" | "soft";
  score: number;
  ghost_id: string | null;
  ghost_project: string | null;
  reason: string | null;
  relation?: string;
};

type ColorMode = "type" | "project" | "community";

const COLOR_MODE_LABEL: Record<ColorMode, string> = {
  type: "종류",
  project: "프로젝트",
  community: "커뮤니티",
};

// 이름표·주제군 이름으로 쓰지 않는 종류.
const NAMELESS_TYPES = new Set(["daily", "meta"]);

// 주제군·프로젝트 색 — 20색. 앞쪽일수록 서로 멀리 떨어진 색.
const GROUP_PALETTE = [
  "#57c1ff", "#59d499", "#ffc533", "#ff6161", "#b47cff",
  "#7fffd4", "#ff8c42", "#5eead4", "#fb7185", "#a78bfa",
  "#fbbf24", "#34d399", "#60a5fa", "#f472b6", "#f87171",
  "#4ade80", "#38bdf8", "#fcd34d", "#c084fc", "#facc15",
];

/** 관계 필터 — "" 전체 · TYPED 관계 있는 선만 · 그 외 특정 관계 이름. */
const REL_TYPED = "__typed";

// "관련(relates_to)" 은 관계 없는 [[링크]] 의 기본값이라 일반 선으로 그린다.
// 화살표·이름·굵기는 영향·원인처럼 뜻이 있는 관계에만 준다.
function isTyped(relation?: string | null): boolean {
  return !!relation && relation !== "relates_to";
}

/** 노드 반지름 — 페인터·히트 영역·화살표가 같은 값을 쓴다. */
// 크기 = 연결 수(√). 허브가 크게 보여야 어디가 중심인지 한눈에 읽힌다.
function nodeRadius(n: { degree?: number; isHub?: boolean }): number {
  const baseR = Math.min(18, 3 + Math.sqrt(Math.max(0, n.degree ?? 0)) * 1.6);
  return n.isHub ? Math.max(baseR * 1.5, baseR + 5) : baseR;
}

// 노드를 원점으로 당기는 중력 — d3 의 center 는 평균 위치만 옮길 뿐 당기지
// 않아서, 강한 반발력과 만나면 클러스터가 사방으로 흩어졌다. 옵시디언의
// "중심력" 처럼 이 힘이 그래프 전체를 둥근 원판으로 모은다.
type SimNode = { x?: number; y?: number; vx?: number; vy?: number };
function gravityForce(k: number) {
  let nodes: SimNode[] = [];
  const force = (alpha: number) => {
    for (const n of nodes) {
      n.vx = (n.vx ?? 0) - (n.x ?? 0) * k * alpha;
      n.vy = (n.vy ?? 0) - (n.y ?? 0) * k * alpha;
    }
  };
  force.initialize = (ns: SimNode[]) => {
    nodes = ns;
  };
  return force;
}

// 주제군(커뮤니티)마다 원 둘레에 자리를 주고 그쪽으로 살짝 당긴다 — 같은
// 색이 한 영역에 모여 "지식 지도" 처럼 읽힌다. 큰 군집 12개만 자리를 받고
// 나머지·고립 노드는 중력만 받아 가운데에 남는다.
type ClusterNode = SimNode & { community?: number; project?: string };
function clusterForce(
  anchors: Map<string, { x: number; y: number }>,
  groupOf: (n: ClusterNode) => string,
  k: number,
) {
  let nodes: ClusterNode[] = [];
  const force = (alpha: number) => {
    for (const n of nodes) {
      const a = anchors.get(groupOf(n));
      if (!a) continue;
      n.vx = (n.vx ?? 0) + (a.x - (n.x ?? 0)) * k * alpha;
      n.vy = (n.vy ?? 0) + (a.y - (n.y ?? 0)) * k * alpha;
    }
  };
  force.initialize = (ns: ClusterNode[]) => {
    nodes = ns;
  };
  return force;
}

/** 전체 화면은 프로젝트로, 한 프로젝트 화면은 주제군(커뮤니티)으로 묶는다. */
function clusterKey(n: ClusterNode, byProject: boolean): string {
  if (byProject) return `p:${n.project ?? ""}`;
  return (n.community ?? -1) >= 0 ? `c:${n.community}` : "";
}

function clusterAnchors(
  nodes: ClusterNode[],
  byProject: boolean,
): Map<string, { x: number; y: number }> {
  const sizes = new Map<string, number>();
  for (const n of nodes) {
    const key = clusterKey(n, byProject);
    if (key) sizes.set(key, (sizes.get(key) ?? 0) + 1);
  }
  const big = Array.from(sizes.entries())
    .filter(([, c]) => c >= 3)
    .sort((a, b) => b[1] - a[1])
    .slice(0, 12);
  const radius = 9 * Math.sqrt(nodes.length);
  const m = new Map<string, { x: number; y: number }>();
  big.forEach(([id], i) => {
    const t = (i / big.length) * Math.PI * 2;
    m.set(id, { x: Math.cos(t) * radius, y: Math.sin(t) * radius });
  });
  return m;
}

/**
 * Full-screen overlay graph of the vault.
 *
 * Ghost links (AI-proposed, pending) render as dashed accent-blue strokes —
 * that's the single thing this graph does that Obsidian structurally can't.
 * Everything else (force layout, pan/zoom, hover tooltip, click-to-open) is
 * standard react-force-graph behavior.
 */
export function GraphView({
  open,
  initialProject,
  onClose,
}: {
  open: boolean;
  /** If set, the project filter dropdown is pre-selected to this project. */
  initialProject?: string | null;
  onClose: () => void;
}) {
  const selectDomain = useApp((s) => s.selectDomain);
  const [data, setData] = useState<GraphData | null>(null);
  const [loading, setLoading] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [projectFilter, setProjectFilter] = useState<string>("");

  // Re-sync the filter whenever the overlay opens or the host passes a new
  // initial project. We only honor `initialProject` at open time so the user
  // can still manually change the dropdown afterward.
  useEffect(() => {
    if (open) setProjectFilter(initialProject ?? "");
  }, [open, initialProject]);
  const [showConfirmed, setShowConfirmed] = useState(true);
  const [showGhost, setShowGhost] = useState(true);
  // 내용 유사도 선은 수천 개라 켜 두면 실제 링크가 묻힌다 — 기본 끔.
  const [showSoft, setShowSoft] = useState(false);
  // 기본은 "커뮤니티" — 대부분 문서가 아직 메모·일지라 종류 색은 한두 색에
  // 그친다. 주제군 색이 그래프를 읽히게 한다. 종류는 select 로 바꾼다.
  const [colorMode, setColorMode] = useState<ColorMode>("community");
  // 전체 화면은 주제군이 수백 개라 색이 반복된다 — 프로젝트 색으로 시작하고,
  // 한 프로젝트를 고르면 주제군 색으로. 고른 뒤엔 select 로 바꿀 수 있다.
  useEffect(() => {
    setColorMode(projectFilter ? "community" : "project");
  }, [projectFilter]);
  const [relFilter, setRelFilter] = useState<string>("");
  const [hoveredLink, setHoveredLink] = useState<GLink | null>(null);
  // 프로젝트별 온톨로지 schema — 한글 라벨(ko)과 "아는 종류" 판정용.
  const [schemas, setSchemas] = useState<Record<string, OntologySchema>>({});
  // 가독성 컨트롤 — 큰 vault 에서 노드/edge 가 한 화면에 폭주하면 그래프
  // 자체가 의미 없어진다. 0 이면 비활성. 슬라이더 UI 는 0.2 에서 도입.
  const [minDegree] = useState<number>(0);
  const [minScore] = useState<number>(0);
  const [insightsOpen, setInsightsOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [hovered, setHovered] = useState<GNode | null>(null);
  const [popup, setPopup] = useState<{
    x: number;
    y: number;
    link: GLink;
  } | null>(null);
  const containerRef = useRef<HTMLDivElement | null>(null);
  const graphRef = useRef<ForceGraphMethods<GNode, GLink>>(
    undefined as unknown as ForceGraphMethods<GNode, GLink>,
  );
  const [dims, setDims] = useState<{ w: number; h: number }>({ w: 800, h: 600 });

  // Pull the graph once per open cycle — caller toggles `open` so we don't
  // need to worry about staleness mid-session.
  useEffect(() => {
    if (!open) return;
    // 필터를 바꾸면 이전 요청의 응답은 버린다 — 프로젝트에서 열 때 먼저 나간
    // "전체" 요청이 늦게 도착해 프로젝트 그래프를 덮어쓰던 문제.
    let alive = true;
    setLoading(true);
    setErr(null);
    ipc
      .buildGraph(projectFilter || undefined)
      .then((g) => {
        if (alive) setData(g);
      })
      .catch((e) => {
        if (alive) setErr(String(e));
      })
      .finally(() => {
        if (alive) setLoading(false);
      });
    return () => {
      alive = false;
    };
  }, [open, projectFilter]);

  // 그래프에 나온 프로젝트의 schema 를 한 번씩만 받아 둔다. 실패하면 기본
  // 라벨로 충분하니 조용히 넘어간다.
  useEffect(() => {
    if (!data) return;
    const need = Array.from(new Set(data.nodes.map((n) => n.project))).filter(
      (p) => !schemas[p],
    );
    if (need.length === 0) return;
    let alive = true;
    Promise.allSettled(need.map((p) => ipc.ontologySchema(p))).then((res) => {
      if (!alive) return;
      const add: Record<string, OntologySchema> = {};
      res.forEach((r, i) => {
        if (r.status === "fulfilled") add[need[i]] = r.value;
      });
      if (Object.keys(add).length > 0) setSchemas((s) => ({ ...s, ...add }));
    });
    return () => {
      alive = false;
    };
    // schemas 는 캐시 — 의존하면 받을 때마다 다시 돈다.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [data]);

  const schemaList = useMemo(() => Object.values(schemas), [schemas]);
  const knownTypes = useMemo(() => {
    const s = new Set<string>(DEFAULT_TYPE_ORDER);
    for (const sc of schemaList) for (const t of sc.types) s.add(t.name);
    return s;
  }, [schemaList]);
  const typeKo = useCallback(
    (name: string) => pickLabel("type", name, schemaList),
    [schemaList],
  );
  const relKo = useCallback(
    (name: string) => pickLabel("relation", name, schemaList),
    [schemaList],
  );
  const statusKo = useCallback(
    (name: string) => pickLabel("status", name, schemaList),
    [schemaList],
  );

  // Tune the d3-force simulation for roomy layouts. We can't set these
  // as props (the library doesn't expose them that way); instead we
  // reach into the instance via its ref after data is loaded. Re-runs
  // whenever the graph data changes since a fresh simulation replaces
  // the previous forces.
  //
  // Single-project mode also gets a weak center gravity so orphans and
  // loosely-connected nodes drift toward the pinned hub instead of flying
  // off to the edges. On the global (multi-project) view we leave gravity
  // off so cluster separation stays readable.
  useEffect(() => {
    if (!data || data.nodes.length === 0) return;
    const inst = graphRef.current as unknown as {
      d3Force?: (
        name: string,
        force?: unknown,
      ) =>
        | {
            strength?: (v: number | ((n: unknown) => number)) => unknown;
            distance?: (v: number) => unknown;
          }
        | undefined;
    } | null;
    if (!inst?.d3Force) return;
    const singleProject = !!projectFilter;

    const charge = inst.d3Force("charge");
    if (charge && typeof charge.strength === "function") {
      // Strong repulsion so nodes don't clump into a pile even when they
      // share many edges. Single-project mode pushes harder because the
      // hub is pinned and we want an airy spray around it.
      charge.strength(singleProject ? -160 : -45);
    }

    const link = inst.d3Force("link");
    if (link && typeof link.distance === "function") {
      // Longer target edge length = more breathing room between connected
      // nodes. Paired with a weaker link strength (below) so the
      // repulsion wins out on densely-connected clusters.
      link.distance(singleProject ? 70 : 36);
    }
    if (link && typeof link.strength === "function") {
      // Default link strength (1/min-degree) pulls hard on hub-like
      // nodes, which squishes their neighborhoods together. Clamping to a
      // gentle constant lets the charge repulsion breathe.
      link.strength(0.25);
    }

    // Gentle pull toward origin so the hub-centric layout stays framed.
    // The hub itself is already pinned (fx/fy), so this mostly affects
    // peripheral nodes and long ghost-link chains.
    const center = inst.d3Force("center");
    if (center && typeof center.strength === "function") {
      center.strength(singleProject ? 0.04 : 0.02);
    }
    inst.d3Force("gravity", gravityForce(singleProject ? 0.05 : 0.09));
    inst.d3Force(
      "cluster",
      clusterForce(
        clusterAnchors(data.nodes, !singleProject),
        (n) => clusterKey(n, !singleProject),
        0.06,
      ),
    );
  }, [data, projectFilter]);

  // Resize observer — the force-graph canvas sizes off explicit w/h props, so
  // we watch the wrapper div and pass values down.
  useEffect(() => {
    if (!open || !containerRef.current) return;
    const el = containerRef.current;
    const ro = new ResizeObserver(() => {
      setDims({ w: el.clientWidth, h: el.clientHeight });
    });
    ro.observe(el);
    setDims({ w: el.clientWidth, h: el.clientHeight });
    return () => ro.disconnect();
  }, [open]);

  // Esc closes the overlay unless a popup is in front.
  useEffect(() => {
    if (!open) return;
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") {
        if (popup) {
          setPopup(null);
          return;
        }
        onClose();
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose, popup]);

  // Project color palette — stable per project name, so re-renders don't
  // shuffle colors around. Resolved from the design tokens so light mode
  // picks up its own accent values.
  // 프로젝트 색 — 문서 많은 순으로 팔레트를 배정해 큰 프로젝트끼리 색이
  // 겹치지 않게 한다.
  const projectRank = useMemo(() => {
    const sizes = new Map<string, number>();
    for (const n of data?.nodes ?? []) sizes.set(n.project, (sizes.get(n.project) ?? 0) + 1);
    const m = new Map<string, number>();
    Array.from(sizes.entries())
      .sort((a, b) => b[1] - a[1])
      .forEach(([p], i) => m.set(p, i));
    return m;
  }, [data]);
  const projectColor = useCallback(
    (project: string): string =>
      GROUP_PALETTE[(projectRank.get(project) ?? 0) % GROUP_PALETTE.length],
    [projectRank],
  );

  // Louvain community color palette — 20 distinct hues. -1 (isolated)
  // renders muted so the eye skips over them.
  const communityColor = useCallback((id: number): string => {
    if (id < 0) return "#6a6b6c";
    return GROUP_PALETTE[id % GROUP_PALETTE.length];
  }, []);

  // Build the view-model each time filters change. We keep two parallel
  // node sets: the full list (so layout positions are stable) and the
  // highlight flags. Edges can be filtered fully (force-graph handles it).
  //
  // Single-project mode: pin the highest-degree node at the origin so the
  // layout reads as a radial spray around that hub. Prefer filenames that
  // look like project overviews (index/overview/readme/plan) on ties so the
  // chosen hub matches the user's mental model. We only pin when filtered
  // to a single project — on the global view anchoring one node would warp
  // inter-project distances.
  const viewData = useMemo(() => {
    if (!data) return { nodes: [] as GNode[], links: [] as GLink[] };

    const singleProject = !!projectFilter;
    let hubId: string | null = null;
    if (singleProject && data.nodes.length > 0) {
      const stemOf = (label: string) =>
        label.replace(/\.md$/i, "").toLowerCase();
      const hubKeywords = ["index", "overview", "readme", "plan", "home"];
      const rank = (n: (typeof data.nodes)[number]) => {
        const stem = stemOf(n.label);
        const keywordBonus = hubKeywords.some((k) => stem.includes(k)) ? 1 : 0;
        return n.degree * 10 + keywordBonus;
      };
      let best = data.nodes[0];
      let bestScore = rank(best);
      for (const n of data.nodes) {
        const s = rank(n);
        if (s > bestScore) {
          best = n;
          bestScore = s;
        }
      }
      hubId = best.id;
    }

    // 1) 노드 필터링: degree 가 임계 미만이면 숨김. 단 hub 는 항상 노출.
    const visibleIds = new Set<string>();
    for (const n of data.nodes) {
      if (n.id === hubId || n.degree >= minDegree) visibleIds.add(n.id);
    }
    // 2) edge 필터링: kind 토글 + 관계 필터 + score 임계 + 양 끝 노드가
    //    모두 보일 때만.
    const links: GLink[] = data.edges
      .filter((e) => {
        if (e.kind === "ghost" && !showGhost) return false;
        if (e.kind === "soft" && !showSoft) return false;
        if (e.kind === "confirmed" && !showConfirmed) return false;
        if (relFilter === REL_TYPED && !isTyped(e.relation)) return false;
        if (relFilter && relFilter !== REL_TYPED && e.relation !== relFilter)
          return false;
        if (e.score < minScore) return false;
        if (!visibleIds.has(e.source) || !visibleIds.has(e.target)) return false;
        return true;
      })
      .map((e) => ({
        source: e.source,
        target: e.target,
        kind: e.kind,
        score: e.score,
        ghost_id: e.ghost_id,
        ghost_project: e.ghost_project,
        reason: e.reason,
        relation: e.relation,
      }));

    // 관계 필터가 켜지면 남은 선에 닿는 노드만 또렷하게. 노드 자체는 숨기지
    // 않는다(레이아웃이 매번 뒤집히지 않게).
    const touched = new Set<string>();
    if (relFilter) {
      for (const l of links) {
        touched.add(l.source as string);
        touched.add(l.target as string);
      }
    }

    const nodes: GNode[] = data.nodes
      .filter((n) => visibleIds.has(n.id))
      .map((n) => {
        const isHub = n.id === hubId;
        return {
          ...n,
          color:
            colorMode === "type"
              ? typeColor(n.doc_type, knownTypes.has(n.doc_type), cssToken)
              : colorMode === "community"
                ? communityColor(n.community)
                : projectColor(n.project),
          dimmed: !!relFilter && !touched.has(n.id),
          isHub,
          fx: isHub ? 0 : undefined,
          fy: isHub ? 0 : undefined,
        };
      });
    return { nodes, links };
  }, [
    data,
    projectFilter,
    showConfirmed,
    showGhost,
    showSoft,
    relFilter,
    colorMode,
    projectColor,
    communityColor,
    knownTypes,
    minDegree,
    minScore,
  ]);

  // 데이터에 실제로 나온 관계 — 관계 필터 select 의 선택지.
  const relationCounts = useMemo(() => {
    const m = new Map<string, number>();
    for (const e of data?.edges ?? []) {
      if (isTyped(e.relation)) m.set(e.relation!, (m.get(e.relation!) ?? 0) + 1);
    }
    return Array.from(m.entries()).sort((a, b) => b[1] - a[1]);
  }, [data]);
  const typedEdgeTotal = relationCounts.reduce((a, [, n]) => a + n, 0);

  // 종류 범례 — 지금 보이는 노드 기준 개수. 기본 종류 순서 → 나머지는 많은 순.
  const typeCounts = useMemo(() => {
    const m = new Map<string, number>();
    for (const n of viewData.nodes) m.set(n.doc_type, (m.get(n.doc_type) ?? 0) + 1);
    return Array.from(m.entries()).sort((a, b) => {
      const ia = DEFAULT_TYPE_ORDER.indexOf(a[0]);
      const ib = DEFAULT_TYPE_ORDER.indexOf(b[0]);
      if (ia >= 0 && ib >= 0) return ia - ib;
      if (ia >= 0) return -1;
      if (ib >= 0) return 1;
      return b[1] - a[1];
    });
  }, [viewData.nodes]);

  const projects = useMemo(() => {
    if (!data) return [] as string[];
    const s = new Set<string>();
    for (const n of data.nodes) s.add(n.project);
    return Array.from(s).sort();
  }, [data]);

  const q = query.trim().toLowerCase();

  // hover 한 노드 + 이웃만 또렷하게, 나머지는 흐리게 (옵시디언·llm_wiki 방식).
  const adjacency = useMemo(() => {
    const m = new Map<string, Set<string>>();
    const idOf = (v: string | GNode) => (typeof v === "string" ? v : v.id);
    for (const l of viewData.links) {
      const a = idOf(l.source);
      const b = idOf(l.target);
      if (!m.has(a)) m.set(a, new Set());
      if (!m.has(b)) m.set(b, new Set());
      m.get(a)!.add(b);
      m.get(b)!.add(a);
    }
    return m;
  }, [viewData.links]);
  const focusSet = useMemo(() => {
    if (!hovered) return null;
    const s = new Set<string>([hovered.id]);
    for (const n of adjacency.get(hovered.id) ?? []) s.add(n);
    return s;
  }, [hovered, adjacency]);

  // 노드가 많을 때 축소 화면에서도 이름을 붙여 둘 허브 — 연결 많은 순 상위 30.
  // 일지·인덱스(메타)는 연결이 많아도 이름으로 쓸모가 적어 뺀다.
  const topLabelIds = useMemo(() => {
    const sorted = viewData.nodes
      .filter((n) => !NAMELESS_TYPES.has(n.doc_type))
      .sort((a, b) => b.degree - a.degree);
    return new Set(sorted.slice(0, 30).map((n) => n.id));
  }, [viewData.nodes]);

  // 커뮤니티 이름 = 그 안에서 연결이 가장 많은 문서 이름. 범례·노드 정보에 쓴다.
  const communities = useMemo(() => {
    const m = new Map<number, { top: GNode; count: number }>();
    for (const n of viewData.nodes) {
      if (n.community < 0) continue;
      // 이름 후보는 일지·인덱스보다 일반 문서를 먼저.
      const better = (a: GNode, b: GNode) => {
        const na = NAMELESS_TYPES.has(a.doc_type) ? 0 : 1;
        const nb = NAMELESS_TYPES.has(b.doc_type) ? 0 : 1;
        return na !== nb ? na > nb : a.degree > b.degree;
      };
      const cur = m.get(n.community);
      if (!cur) m.set(n.community, { top: n, count: 1 });
      else {
        cur.count += 1;
        if (better(n, cur.top)) cur.top = n;
      }
    }
    return m;
  }, [viewData.nodes]);

  // Edge color / dash pattern per kind:
  //   confirmed → bright hairline (ground truth)
  //   ghost     → accent blue, dashed (AI proposal, waiting for user)
  //   soft      → accent yellow, thin dashed (implicit relevance hint)
  //   confirmed + 관계 → 거의 불투명 + 화살표 + 관계 이름 (가장 또렷)
  function linkColor(l: GLink): string {
    if (l.kind === "ghost") return cssToken("accent-blue");
    if (l.kind === "soft") return withAlpha(cssToken("accent-yellow"), 0.55);
    if (isTyped(l.relation)) return withAlpha(cssToken("ink"), 0.92);
    // 일반 링크는 출발 노드 색으로 옅게 물들인다 — 선이 어느 주제군에서
    // 나왔는지 보이고, 흰 거미줄처럼 보이지 않는다. 큰 vault 일수록 더 옅게.
    const base = (l.source as GNode)?.color ?? cssToken("ink");
    return withAlpha(base, crowded ? 0.1 : 0.45);
  }

  // Canvas custom node painter — draws the circle + a label that fades in
  // as the user zooms in. Matching nodes are ringed in white.
  // Below this threshold we just always show labels — small vaults read
  // better with every node named. Above it we fall back to zoom/hover only
  // so the canvas doesn't turn into a text pile.
  const crowded = (data?.nodes.length ?? 0) > 100;

  // Humanize a file label for display: drop the `.md` extension, convert
  // kebab/snake to spaces, and title-case unless the stem looks like a
  // date (YYYY-MM-DD daily notes stay verbatim). The raw filename is
  // still used for routing / search, only the rendered caption changes.
  function prettyLabel(raw: string): string {
    const stem = raw.replace(/\.md$/i, "").replace(/^daily\//, "");
    if (/^\d{4}-\d{2}-\d{2}$/.test(stem)) return stem;
    const words = stem.replace(/[-_]+/g, " ").split(/\s+/).filter(Boolean);
    return words
      .map((w) => {
        if (/^[a-z]{1,4}$/.test(w)) return w.toUpperCase();
        return w.charAt(0).toUpperCase() + w.slice(1);
      })
      .join(" ");
  }

  const nodeCanvasObject = useCallback(
    (node: GNode, ctx: CanvasRenderingContext2D, scale: number) => {
      // Hubs render noticeably larger with a soft halo so the radial
      // center is obvious at a glance — the eye should land there first.
      // 축소해도 점이 화면에서 2.5px 아래로 줄지 않게 — 그래야 종류 색이 보인다.
      const r = Math.max(nodeRadius(node), 2.5 / scale);
      const matches =
        q.length > 0 &&
        (node.label.toLowerCase().includes(q) ||
          node.project.toLowerCase().includes(q));

      const x = (node.x ?? 0) as number;
      const y = (node.y ?? 0) as number;

      // 상태(대체됨/완료) · 관계 필터 밖 노드는 흐리게. 검색·hover 대상은
      // 원래대로 보여준다.
      const focus = matches || hovered?.id === node.id;
      const inFocus = !focusSet || focusSet.has(node.id);
      const alpha = focus
        ? 1
        : statusAlpha(node.status) *
          (node.dimmed ? 0.3 : 1) *
          (inFocus ? 1 : 0.12);
      ctx.save();
      ctx.globalAlpha = alpha;

      // Soft radial halo behind the hub. Pure-canvas gradient so the
      // effect scales with the camera without hitting layout.
      if (node.isHub) {
        const haloR = r * 2.4;
        const grad = ctx.createRadialGradient(x, y, r * 0.9, x, y, haloR);
        grad.addColorStop(0, withAlpha(cssToken("ink"), 0.18));
        grad.addColorStop(1, withAlpha(cssToken("ink"), 0));
        ctx.beginPath();
        ctx.arc(x, y, haloR, 0, Math.PI * 2);
        ctx.fillStyle = grad;
        ctx.fill();
      }

      ctx.beginPath();
      ctx.arc(x, y, r, 0, Math.PI * 2);
      ctx.fillStyle = node.color ?? cssToken("ash");
      ctx.fill();

      if (node.isHub) {
        // Double ring: thicker inner stroke in the node color's white
        // highlight, thin outer ring for separation from the halo.
        ctx.strokeStyle = withAlpha(cssToken("ink"), 0.95);
        ctx.lineWidth = 2 / scale;
        ctx.stroke();
      } else if (matches || hovered?.id === node.id) {
        ctx.strokeStyle = cssToken("ink");
        ctx.lineWidth = 1.5 / scale;
        ctx.stroke();
      }

      // 이름은 점 아래 가운데 — 축소 화면에선 허브·hover 이웃만.
      const showLabel =
        !crowded ||
        scale > 1.4 ||
        matches ||
        node.isHub ||
        topLabelIds.has(node.id) ||
        (!!focusSet && focusSet.has(node.id));
      if (showLabel && inFocus) {
        // 화면 기준 글자 크기 고정 — 확대·축소해도 같은 크기로 읽힌다.
        const fontPx = (node.isHub ? 12 : 10.5) / scale;
        const label = prettyLabel(node.label);
        ctx.font = `${node.isHub ? "600 " : "500 "}${fontPx}px Inter, sans-serif`;
        ctx.fillStyle =
          matches || hovered?.id === node.id || node.isHub
            ? cssToken("ink")
            : cssToken("body");
        ctx.textAlign = "center";
        ctx.textBaseline = "top";
        const ly = y + r + 3 / scale;
        ctx.fillText(label, x, ly);
        // 대체된 문서는 이름에 취소선.
        if (node.status === "superseded") {
          const w = ctx.measureText(label).width;
          const my = ly + fontPx * 0.55;
          ctx.beginPath();
          ctx.moveTo(x - w / 2, my);
          ctx.lineTo(x + w / 2, my);
          ctx.strokeStyle = ctx.fillStyle;
          ctx.lineWidth = 1 / scale;
          ctx.stroke();
        }
      }
      ctx.restore();
    },
    [hovered, q, crowded, focusSet, topLabelIds],
  );

  const linkCanvasObject = useCallback(
    (link: GLink, ctx: CanvasRenderingContext2D, scale: number) => {
      const src = link.source as GNode;
      const tgt = link.target as GNode;
      const sx = (src?.x ?? 0) as number;
      const sy = (src?.y ?? 0) as number;
      const tx = (tgt?.x ?? 0) as number;
      const ty = (tgt?.y ?? 0) as number;
      const typed = isTyped(link.relation) && link.kind !== "soft";
      // hover 중이면 그 노드에 닿은 선만 노드 색으로 진하게, 나머지는 거의 숨김.
      const touching =
        !!hovered && (src?.id === hovered.id || tgt?.id === hovered.id);
      const color = !hovered
        ? linkColor(link)
        : touching
          ? typed
            ? withAlpha(cssToken("ink"), 0.95)
            : withAlpha(hovered.color ?? cssToken("ink"), 0.9)
          : withAlpha(cssToken("ink"), 0.03);

      // 대체된 문서에 닿은 선도 같이 흐리게 — 노드만 흐리면 선이 튄다.
      const endAlpha = Math.min(
        statusAlpha(src?.status) * (src?.dimmed ? 0.3 : 1),
        statusAlpha(tgt?.status) * (tgt?.dimmed ? 0.3 : 1),
      );
      ctx.save();
      ctx.globalAlpha = touching ? 1 : Math.max(0.25, endAlpha);

      ctx.beginPath();
      ctx.moveTo(sx, sy);
      ctx.lineTo(tx, ty);
      ctx.strokeStyle = color;
      // Thicken by score: a relevance 1.5 soft edge reads thinner than a
      // 3.0 confirmed wiki link. Confirmed stays bold so ground truth
      // still dominates visually. 관계 있는 확정 선은 한 단계 더 굵게.
      const base =
        link.kind === "ghost"
          ? 2.2
          : link.kind === "soft"
            ? 1.1
            : typed
              ? 2.8
              : touching
                ? 1.8
                : 1.0;
      const boost = Math.min(1.5, (link.score - 1.0) * 0.35);
      ctx.lineWidth = Math.max(0.8, base + boost);
      if (link.kind === "ghost") {
        ctx.setLineDash([6, 4]);
      } else if (link.kind === "soft") {
        ctx.setLineDash([2, 3]);
      } else {
        ctx.setLineDash([]);
      }
      ctx.stroke();
      ctx.setLineDash([]);

      // 화살표: 관계 선은 항상, 일반 링크는 작은 그래프이거나 hover 이웃일 때.
      if (typed || (link.kind === "confirmed" && (!crowded || touching))) {
        const dx = tx - sx;
        const dy = ty - sy;
        const len = Math.hypot(dx, dy) || 1;
        const ux = dx / len;
        const uy = dy / len;

        // 방향 화살표 — 관계는 source → target 으로 읽힌다.
        const rt = Math.max(nodeRadius(tgt ?? {}), 2.5 / scale) + 1.5;
        const ax = tx - ux * rt;
        const ay = ty - uy * rt;
        const a =
          (typed ? 1 : 0.65) * Math.max(4, 7 / Math.sqrt(Math.max(0.2, scale)));
        ctx.beginPath();
        ctx.moveTo(ax, ay);
        ctx.lineTo(ax - ux * a - uy * a * 0.55, ay - uy * a + ux * a * 0.55);
        ctx.lineTo(ax - ux * a + uy * a * 0.55, ay - uy * a - ux * a * 0.55);
        ctx.closePath();
        ctx.fillStyle = color;
        ctx.fill();

        // 관계 이름 — 확대했거나, 끝 노드/선에 hover 했거나, 그래프가 작을 때.
        const near =
          hoveredLink === link ||
          (hovered && (hovered.id === src?.id || hovered.id === tgt?.id));
        const showLabel =
          typed && (near || scale >= 1.1 || (!crowded && scale >= 0.6));
        if (showLabel) {
          const text =
            relKo(link.relation!) + (link.kind === "ghost" ? " · 제안" : "");
          const fontPx = 10 / scale;
          ctx.font = `500 ${fontPx}px Inter, sans-serif`;
          const w = ctx.measureText(text).width;
          const mx = (sx + tx) / 2;
          const my = (sy + ty) / 2;
          const padX = 3 / scale;
          const h = fontPx * 1.3;
          ctx.fillStyle = withAlpha(cssToken("canvas"), 0.88);
          ctx.fillRect(mx - w / 2 - padX, my - h / 2, w + padX * 2, h);
          ctx.fillStyle =
            link.kind === "ghost" ? cssToken("accent-blue") : cssToken("ink");
          ctx.textAlign = "center";
          ctx.textBaseline = "middle";
          ctx.fillText(text, mx, my);
        }
      }
      ctx.restore();
    },
    // linkColor 는 typedEdgeTotal 만 본다.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [hovered, hoveredLink, crowded, relKo, typedEdgeTotal],
  );

  // Accept / reject is implemented via the existing ghost_links commands.
  // The graph doesn't mutate `data` directly — it refetches so the force
  // simulation re-balances without us hand-maintaining positions.
  async function acceptGhost(link: GLink) {
    if (!link.ghost_project || !link.ghost_id) return;
    try {
      await ipc.ghostAccept(link.ghost_project, link.ghost_id);
      setPopup(null);
      const g = await ipc.buildGraph(projectFilter || undefined);
      setData(g);
    } catch (e) {
      setErr(String(e));
    }
  }

  async function rejectGhost(link: GLink) {
    if (!link.ghost_project || !link.ghost_id) return;
    try {
      await ipc.ghostReject(link.ghost_project, link.ghost_id);
      setPopup(null);
      const g = await ipc.buildGraph(projectFilter || undefined);
      setData(g);
    } catch (e) {
      setErr(String(e));
    }
  }

  if (!open) return null;

  return (
    <div
      className="fixed inset-0 z-30 flex flex-col bg-canvas"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      {/* Toolbar */}
      <header
        data-tauri-drag-region
        className="flex h-10 shrink-0 items-center justify-between border-b border-hairline bg-surface/80 px-3 backdrop-blur"
      >
        <div className="flex items-center gap-2 text-[13px] text-ink">
          <span className="font-medium">그래프</span>
          <span className="text-caption-sm text-stone">
            {data?.nodes.length ?? 0} 노드 · {data?.edges.length ?? 0} 링크
          </span>
          {loading && (
            <span className="text-caption-sm text-stone">로딩…</span>
          )}
        </div>

        <div className="flex items-center gap-2">
          <div className="inline-flex items-center gap-1 rounded-sm border border-hairline bg-surface-elevated px-2">
            <Search size={11} className="text-stone" />
            <input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="이름으로 찾기"
              className="h-6 w-[140px] bg-transparent text-[12px] outline-none placeholder:text-stone"
            />
          </div>

          <div className="inline-flex items-center gap-1 rounded-sm border border-hairline bg-surface-elevated px-2 py-0.5 text-[12px]">
            <Filter size={11} className="text-stone" />
            <select
              value={projectFilter}
              onChange={(e) => setProjectFilter(e.target.value)}
              className="bg-transparent text-body outline-none"
            >
              <option value="">전체 프로젝트</option>
              {projects.map((p) => (
                <option key={p} value={p}>
                  {p}
                </option>
              ))}
            </select>
          </div>

          <label className="inline-flex items-center gap-1.5 text-caption-md text-body">
            <input
              type="checkbox"
              checked={showConfirmed}
              onChange={(e) => setShowConfirmed(e.target.checked)}
              className="accent-on-dark"
            />
            <span>
              <span className="mr-1 inline-block h-0.5 w-3 bg-on-dark-mute align-middle" />
              확정
            </span>
          </label>
          <label className="inline-flex items-center gap-1.5 text-caption-md text-body">
            <input
              type="checkbox"
              checked={showGhost}
              onChange={(e) => setShowGhost(e.target.checked)}
              className="accent-accent-blue"
            />
            <span>
              <span className="mr-1 inline-block h-0.5 w-3 border-t border-dashed border-accent-blue align-middle" />
              제안
            </span>
          </label>
          <label className="inline-flex items-center gap-1.5 text-caption-md text-body">
            <input
              type="checkbox"
              checked={showSoft}
              onChange={(e) => setShowSoft(e.target.checked)}
              className="accent-accent-yellow"
            />
            <span>
              <span className="mr-1 inline-block h-0.5 w-3 border-t border-dashed border-accent-yellow align-middle" />
              관련
            </span>
          </label>

          <div
            className="inline-flex items-center gap-1 rounded-sm border border-hairline bg-surface-elevated px-2 py-0.5 text-[12px]"
            title="관계로 선 거르기"
          >
            <GitBranch size={11} className="text-stone" />
            <select
              value={relFilter}
              onChange={(e) => setRelFilter(e.target.value)}
              className="bg-transparent text-body outline-none"
            >
              <option value="">모든 선</option>
              <option value={REL_TYPED}>
                관계 있는 선만 ({typedEdgeTotal})
              </option>
              {relationCounts.map(([name, n]) => (
                <option key={name} value={name}>
                  {relKo(name)} ({n})
                </option>
              ))}
            </select>
          </div>

          <div
            className="inline-flex items-center gap-1 rounded-sm border border-hairline bg-surface-elevated px-2 py-0.5 text-[12px]"
            title="노드 색 기준"
          >
            <Palette size={11} className="text-stone" />
            <select
              value={colorMode}
              onChange={(e) => setColorMode(e.target.value as ColorMode)}
              className="bg-transparent text-body outline-none"
            >
              {(Object.keys(COLOR_MODE_LABEL) as ColorMode[]).map((m) => (
                <option key={m} value={m}>
                  색: {COLOR_MODE_LABEL[m]}
                </option>
              ))}
            </select>
          </div>

          <button
            onClick={() => setInsightsOpen((v) => !v)}
            title="인사이트 패널"
            className={cn(
              "inline-flex h-7 items-center gap-1 rounded-sm border px-2 text-[12px] transition-colors",
              insightsOpen
                ? "border-accent-yellow bg-accent-yellow-soft text-accent-yellow"
                : "border-hairline bg-surface-elevated text-body hover:text-on-dark",
            )}
          >
            <Lightbulb size={11} />
            인사이트
          </button>

          <button
            onClick={onClose}
            className="grid h-7 w-7 place-items-center rounded-sm text-mute transition-colors hover:bg-surface-elevated hover:text-on-dark"
            title="닫기 (Esc)"
          >
            <X size={14} />
          </button>
        </div>
      </header>

      {/* Canvas */}
      <div ref={containerRef} className="relative flex-1 overflow-hidden">
        {err && (
          <div className="absolute left-4 top-4 z-10 max-w-[420px] rounded-md border border-hairline bg-surface-elevated p-2 font-mono text-[11px] leading-[1.5] text-accent-red">
            {err}
          </div>
        )}
        {data && data.nodes.length === 0 && !loading && (
          <div className="grid h-full place-items-center text-[13px] text-stone">
            표시할 문서가 없어요. 프로젝트에 .md 파일을 추가해 보세요.
          </div>
        )}
        {data && data.nodes.length > 0 && (
          <ForceGraph2D<GNode, GLink>
            ref={graphRef}
            graphData={viewData}
            width={dims.w}
            height={dims.h}
            backgroundColor="transparent"
            // 클러스터끼리 자연스럽게 떨어지게 — 기존 80 ticks 에서는
            // 평형 도달 전 시뮬레이션이 멈춰서 뭉친 모양이 됐다. 너무
            // 길게 잡으면 사용자 피드백상 폭주해서 적당히 살짝만 늘림.
            cooldownTicks={210}
            d3AlphaDecay={0.028}
            d3VelocityDecay={0.42}
            nodeRelSize={4.5}
            nodeCanvasObject={nodeCanvasObject}
            nodePointerAreaPaint={(node, color, ctx) => {
              const n = node as GNode;
              // 보이는 크기보다 조금 넉넉하게 — 작은 점도 집기 쉽게.
              const r = Math.max(6, nodeRadius(n) + 2);
              ctx.fillStyle = color;
              ctx.beginPath();
              ctx.arc(
                (n.x ?? 0) as number,
                (n.y ?? 0) as number,
                r,
                0,
                Math.PI * 2,
              );
              ctx.fill();
            }}
            linkCanvasObject={linkCanvasObject}
            linkPointerAreaPaint={(link, color, ctx) => {
              const l = link as GLink;
              const s = l.source as GNode;
              const t = l.target as GNode;
              ctx.strokeStyle = color;
              ctx.lineWidth = 6;
              ctx.beginPath();
              ctx.moveTo((s?.x ?? 0) as number, (s?.y ?? 0) as number);
              ctx.lineTo((t?.x ?? 0) as number, (t?.y ?? 0) as number);
              ctx.stroke();
            }}
            onNodeHover={(n) => setHovered((n as GNode | null) ?? null)}
            onLinkHover={(l) => setHoveredLink((l as GLink | null) ?? null)}
            onNodeClick={(n) => {
              const node = n as GNode;
              selectDomain(node.project, node.domain);
              onClose();
            }}
            onLinkClick={(l, ev) => {
              const link = l as GLink;
              const x = (ev as MouseEvent).clientX;
              const y = (ev as MouseEvent).clientY;
              setPopup({ x, y, link });
            }}
          />
        )}

        {hovered && (
          <div className="pointer-events-none absolute bottom-3 right-3 rounded-md border border-hairline bg-surface px-3 py-2 text-caption-md">
            <div className="text-mute">{hovered.project}</div>
            <div className="text-[13px] font-medium text-ink">
              {prettyLabel(hovered.label)}
            </div>
            <div className="mt-0.5 font-mono text-caption-sm text-stone">
              {hovered.domain}
            </div>
            <div className="text-caption-sm text-stone">
              {typeKo(hovered.doc_type)}
              {hovered.status ? ` · ${statusKo(hovered.status)}` : ""} · 연결{" "}
              {hovered.degree} · {(hovered.bytes / 1024).toFixed(1)} KB
            </div>
            {communities.get(hovered.community) && (
              <div className="mt-0.5 flex items-center gap-1.5 text-caption-sm text-stone">
                <span
                  className="inline-block h-2 w-2 rounded-full"
                  style={{ backgroundColor: communityColor(hovered.community) }}
                />
                주제군:{" "}
                {prettyLabel(communities.get(hovered.community)!.top.label)}
              </div>
            )}
          </div>
        )}

        {!hovered && hoveredLink && (
          <LinkTooltip link={hoveredLink} relKo={relKo} />
        )}

        {data && data.nodes.length > 0 && (
          <GraphLegend
            colorMode={colorMode}
            typeCounts={typeCounts.map(([name, n]) => ({
              name,
              label: typeKo(name),
              count: n,
              color: typeColor(name, knownTypes.has(name), cssToken),
            }))}
            typedEdges={typedEdgeTotal}
            communities={
              colorMode === "project"
                ? Array.from(
                    viewData.nodes.reduce(
                      (m, n) => m.set(n.project, (m.get(n.project) ?? 0) + 1),
                      new Map<string, number>(),
                    ),
                  )
                    .sort((a, b) => b[1] - a[1])
                    .slice(0, 12)
                    .map(([p, count], id) => ({
                      id,
                      name: p,
                      count,
                      color: projectColor(p),
                    }))
                : Array.from(communities.entries())
                    .sort((a, b) => b[1].count - a[1].count)
                    .slice(0, 12)
                    .map(([id, c]) => ({
                      id,
                      name: prettyLabel(c.top.label),
                      count: c.count,
                      color: communityColor(id),
                    }))
            }
          />
        )}

        {insightsOpen && data && (
          <InsightsPanel
            insights={data.insights}
            nodes={data.nodes}
            onFocusNode={(id) => {
              // Highlight on hover using the existing hover state so the
              // user gets visual feedback without zoom logic.
              const node = data.nodes.find((n) => n.id === id);
              if (node) setHovered(node as GNode);
              // Zoom to the node if the force-graph instance is ready.
              const inst = graphRef.current;
              if (inst && node) {
                // The positions come from the simulation; we read them
                // off the node object itself since force-graph mutates
                // it in place.
                const x = (node as unknown as GNode).x;
                const y = (node as unknown as GNode).y;
                if (typeof x === "number" && typeof y === "number") {
                  inst.centerAt(x, y, 600);
                  inst.zoom(2.5, 600);
                }
              }
            }}
          />
        )}

        {popup &&
          (popup.link.kind === "ghost" ? (
            <GhostPopup
              x={popup.x}
              y={popup.y}
              link={popup.link}
              relKo={relKo}
              onAccept={() => acceptGhost(popup.link)}
              onReject={() => rejectGhost(popup.link)}
              onClose={() => setPopup(null)}
            />
          ) : (
            <ConfirmedPopup
              x={popup.x}
              y={popup.y}
              link={popup.link}
              relKo={relKo}
              onOpen={(node) => {
                selectDomain(node.project, node.domain);
                setPopup(null);
                onClose();
              }}
              onClose={() => setPopup(null)}
            />
          ))}
      </div>
    </div>
  );
}

function GhostPopup({
  x,
  y,
  link,
  relKo,
  onAccept,
  onReject,
  onClose,
}: {
  x: number;
  y: number;
  link: GLink;
  relKo: (name: string) => string;
  onAccept: () => void;
  onReject: () => void;
  onClose: () => void;
}) {
  // 그래프 edge 에는 제안자가 없어서 ghost store 에서 찾아온다.
  const [proposedBy, setProposedBy] = useState<string | null>(null);
  useEffect(() => {
    if (!link.ghost_project || !link.ghost_id) return;
    let alive = true;
    ipc
      .ghostList(link.ghost_project)
      .then((st) => {
        if (!alive) return;
        const g = st.links.find((l) => l.id === link.ghost_id);
        setProposedBy(g?.proposed_by ?? null);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [link.ghost_project, link.ghost_id]);

  // Trim popup label to just the filename portion for readability.
  const src =
    typeof link.source === "string"
      ? link.source
      : (link.source as GNode).domain;
  const tgt =
    typeof link.target === "string"
      ? link.target
      : (link.target as GNode).domain;

  return (
    <div
      className="fixed z-40 w-[300px] rounded-md border border-hairline-strong bg-surface-card p-3"
      style={{
        left: Math.min(x, window.innerWidth - 320),
        top: Math.min(y + 4, window.innerHeight - 160),
      }}
    >
      <div className="mb-2 flex items-center justify-between">
        <span className="text-[11px] uppercase tracking-[0.4px] text-on-dark-mute">
          {proposedBy && proposedBy !== "scan"
            ? `에이전트 제안 (${proposedBy})`
            : "AI 제안 링크"}
        </span>
        <button
          onClick={onClose}
          className="grid h-5 w-5 place-items-center rounded-sm text-stone hover:text-on-dark"
        >
          <X size={10} />
        </button>
      </div>
      <div className="mb-1 font-mono text-[12px] text-ink">
        {src}
        <span className="mx-1 text-stone">→</span>
        {tgt}
      </div>
      {link.relation && (
        <div className="mb-1.5 flex items-center gap-1.5">
          <RelationBadge>{relKo(link.relation)}</RelationBadge>
        </div>
      )}
      {link.reason && (
        <div className="mb-2 text-caption-sm leading-[1.5] text-body">
          {link.reason}
        </div>
      )}
      <div className="mb-3 text-caption-sm leading-[1.5] text-stone">
        받아들이면 {src} 의 ## 관련 에{" "}
        <span className="font-mono text-mute">
          {link.relation && link.relation !== "relates_to"
            ? `${relKo(link.relation)}:: [[…]]`
            : "[[…]]"}
        </span>{" "}
        로 적어요.
      </div>
      <div className="flex items-center justify-end gap-1.5">
        <button
          onClick={onReject}
          className="inline-flex h-7 items-center rounded-sm border border-hairline bg-surface-elevated px-2 text-[12px] text-body hover:border-hairline-strong hover:text-on-dark"
        >
          거절
        </button>
        <button
          onClick={onAccept}
          className={cn(
            "inline-flex h-7 items-center rounded-sm px-2 text-[12px] font-medium",
            "bg-primary text-on-primary hover:bg-primary-pressed",
          )}
        >
          받아들이기
        </button>
      </div>
    </div>
  );
}

function ConfirmedPopup({
  x,
  y,
  link,
  relKo,
  onOpen,
  onClose,
}: {
  x: number;
  y: number;
  link: GLink;
  relKo: (name: string) => string;
  onOpen: (node: GNode) => void;
  onClose: () => void;
}) {
  // For confirmed edges both endpoints are always full node objects because
  // the force-graph replaces the string ids with references before drawing.
  const src = link.source as GNode;
  const tgt = link.target as GNode;
  return (
    <div
      className="fixed z-40 w-[320px] rounded-md border border-hairline-strong bg-surface-card p-3"
      style={{
        left: Math.min(x, window.innerWidth - 340),
        top: Math.min(y + 4, window.innerHeight - 180),
      }}
    >
      <div className="mb-2 flex items-center justify-between">
        <span className="flex items-center gap-1.5 text-[11px] uppercase tracking-[0.4px] text-on-dark-mute">
          확정 링크
          {link.relation && <RelationBadge>{relKo(link.relation)}</RelationBadge>}
        </span>
        <button
          onClick={onClose}
          className="grid h-5 w-5 place-items-center rounded-sm text-stone hover:text-on-dark"
        >
          <X size={10} />
        </button>
      </div>
      <div className="mb-3 font-mono text-[12px] leading-[1.6] text-ink">
        <span className="text-mute">{src.project}</span>
        <span className="text-stone">/</span>
        {src.domain}
        <span className="mx-1 text-stone">→</span>
        <span className="text-mute">{tgt.project}</span>
        <span className="text-stone">/</span>
        {tgt.domain}
      </div>
      <div className="mb-3 text-caption-sm leading-[1.5] text-body">
        {link.relation ? (
          <>
            <span className="text-mute">{src.domain}</span> 의{" "}
            <span className="text-on-dark">{relKo(link.relation)}</span> 이(가){" "}
            <span className="text-mute">{tgt.domain}</span> 라고 적혀 있어요{" "}
            <span className="font-mono text-stone">
              ({relKo(link.relation)}:: [[{tgt.label.replace(/\.md$/i, "")}]])
            </span>
            .
          </>
        ) : (
          <>
            <span className="text-mute">{src.domain}</span> 의 본문에{" "}
            <span className="font-mono text-on-dark">[[{tgt.label.replace(/\.md$/i, "")}]]</span>{" "}
            참조가 있어서 이어진 선이에요.
          </>
        )}
      </div>
      <div className="flex items-center justify-end gap-1.5">
        <button
          onClick={() => onOpen(tgt)}
          className="inline-flex h-7 items-center rounded-sm border border-hairline bg-surface-elevated px-2 text-[12px] text-body hover:border-hairline-strong hover:text-on-dark"
        >
          {tgt.label} 열기
        </button>
        <button
          onClick={() => onOpen(src)}
          className="inline-flex h-7 items-center rounded-sm bg-primary px-2 text-[12px] font-medium text-on-primary hover:bg-primary-pressed"
        >
          {src.label} 열기
        </button>
      </div>
    </div>
  );
}

function InsightsPanel({
  insights,
  nodes,
  onFocusNode,
}: {
  insights: GraphInsights;
  nodes: Array<{ id: string; label: string; project: string }>;
  onFocusNode: (id: string) => void;
}) {
  const rawLabelOf = (id: string) =>
    nodes.find((n) => n.id === id)?.label ?? id;
  const labelOf = (id: string) => {
    const raw = rawLabelOf(id);
    const stem = raw.replace(/\.md$/i, "").replace(/^daily\//, "");
    if (/^\d{4}-\d{2}-\d{2}$/.test(stem)) return stem;
    const words = stem.replace(/[-_]+/g, " ").split(/\s+/).filter(Boolean);
    return words
      .map((w) =>
        /^[a-z]{1,4}$/.test(w)
          ? w.toUpperCase()
          : w.charAt(0).toUpperCase() + w.slice(1),
      )
      .join(" ");
  };
  const projectOf = (id: string) =>
    nodes.find((n) => n.id === id)?.project ?? "";

  return (
    <div className="absolute right-3 top-3 z-20 flex max-h-[calc(100%-24px)] w-[320px] flex-col overflow-hidden rounded-md border border-hairline-strong bg-surface-card/95 backdrop-blur">
      <header className="flex items-center gap-2 border-b border-hairline px-3 py-2 text-[12px] font-medium text-ink">
        <Lightbulb size={12} className="text-accent-yellow" />
        그래프 인사이트
      </header>
      <div className="flex-1 min-h-0 overflow-auto px-3 py-2">
        <InsightSection
          title="영향 표시 없는 결정"
          subtitle="유효한 결정인데 무엇에 영향을 주는지 안 적혔어요 — 본문에 영향:: [[문서]]"
          empty="모든 결정에 영향이 적혀 있어요."
          items={(insights.decisions_without_affects ?? []).map((id) => ({ id }))}
          labelOf={labelOf}
          projectOf={projectOf}
          onFocus={onFocusNode}
        />
        <InsightSection
          title="원인 없는 버그"
          subtitle="열린 버그인데 원인이 안 적혔어요 — 본문에 원인:: [[문서]]"
          empty="모든 버그에 원인이 적혀 있어요."
          items={(insights.bugs_without_cause ?? []).map((id) => ({ id }))}
          labelOf={labelOf}
          projectOf={projectOf}
          onFocus={onFocusNode}
        />
        <InsightSection
          title="Hub"
          subtitle="가장 많이 연결된 문서들 — 프로젝트의 중심 개념"
          empty="아직 허브가 될 만큼 연결이 없어요."
          items={insights.hubs.map((h) => ({
            id: h.id,
            right: `${h.degree} 연결`,
          }))}
          labelOf={labelOf}
          projectOf={projectOf}
          onFocus={onFocusNode}
        />
        <InsightSection
          title="Bridge"
          subtitle="이 문서가 사라지면 그래프가 쪼개져요 — 연결의 급소"
          empty="브릿지 노드가 없어요 (좋은 신호)."
          items={insights.bridges.map((id) => ({ id }))}
          labelOf={labelOf}
          projectOf={projectOf}
          onFocus={onFocusNode}
        />
        <InsightSection
          title="Surprising"
          subtitle="서로 다른 주제 군집을 이어주는 유일한 다리"
          empty="community 간 유일 엣지가 없어요."
          items={insights.surprising.map((s) => ({
            id: s.source,
            right: labelOf(s.target),
          }))}
          labelOf={labelOf}
          projectOf={projectOf}
          onFocus={onFocusNode}
        />
        <InsightSection
          title="Isolated"
          subtitle="아무 엣지도 없는 외톨이 문서 — 삭제하거나 연결해보세요"
          empty="고립된 문서가 없어요."
          items={insights.isolated.map((id) => ({ id }))}
          labelOf={labelOf}
          projectOf={projectOf}
          onFocus={onFocusNode}
        />
        <InsightSection
          title="Sparse"
          subtitle="1~2개짜리 작은 community — 더 깊게 파볼 만한 영역"
          empty="모든 community 가 적당한 크기예요."
          items={insights.sparse_communities.flatMap((c) =>
            c.members.map((id) => ({
              id,
              right: `C${c.id}`,
            })),
          )}
          labelOf={labelOf}
          projectOf={projectOf}
          onFocus={onFocusNode}
        />
      </div>
    </div>
  );
}

function InsightSection({
  title,
  subtitle,
  empty,
  items,
  labelOf,
  projectOf,
  onFocus,
}: {
  title: string;
  subtitle: string;
  empty: string;
  items: Array<{ id: string; right?: string }>;
  labelOf: (id: string) => string;
  projectOf: (id: string) => string;
  onFocus: (id: string) => void;
}) {
  return (
    <section className="mb-4 last:mb-0">
      <div className="mb-1 flex items-center gap-1.5">
        <span className="text-[10px] uppercase tracking-[0.6px] text-mute">
          {title}
        </span>
        <span className="text-caption-sm text-stone">{items.length}</span>
      </div>
      <p className="mb-1.5 text-caption-sm leading-[1.5] text-stone">
        {subtitle}
      </p>
      {items.length === 0 ? (
        <div className="text-caption-sm italic text-stone">{empty}</div>
      ) : (
        <ul className="flex flex-col gap-0.5">
          {items.slice(0, 8).map((it, i) => (
            <li key={`${it.id}-${i}`}>
              <button
                onClick={() => onFocus(it.id)}
                className="flex w-full items-center gap-1.5 rounded-sm px-1.5 py-1 text-left transition-colors hover:bg-surface-elevated"
              >
                <span className="text-caption-sm text-mute">
                  {projectOf(it.id)}
                </span>
                <span className="text-stone">/</span>
                <span className="flex-1 truncate font-mono text-[11px] text-body">
                  {labelOf(it.id)}
                </span>
                {it.right && (
                  <span className="text-caption-sm text-stone">{it.right}</span>
                )}
              </button>
            </li>
          ))}
          {items.length > 8 && (
            <li className="px-1.5 text-caption-sm text-stone">
              +{items.length - 8} 더
            </li>
          )}
        </ul>
      )}
    </section>
  );
}

function GraphLegend({
  colorMode,
  typeCounts,
  typedEdges,
  communities,
}: {
  colorMode: ColorMode;
  typeCounts: Array<{ name: string; label: string; count: number; color: string }>;
  typedEdges: number;
  communities: Array<{ id: number; name: string; count: number; color: string }>;
}) {
  return (
    <div className="pointer-events-none absolute bottom-3 left-3 max-h-[calc(100%-24px)] select-none overflow-hidden rounded-md border border-hairline bg-surface/90 px-3 py-2 backdrop-blur">
      <div className="mb-1.5 text-[10px] uppercase tracking-[0.6px] text-mute">
        범례
      </div>

      <div className="mb-2">
        <div className="mb-1 text-caption-sm text-stone">
          노드 —{" "}
          {colorMode === "type"
            ? "문서 종류"
            : colorMode === "project"
              ? "프로젝트"
              : "주제군"}
        </div>
        <div className="flex flex-col gap-0.5 text-caption-md text-body">
          {colorMode === "type" ? (
            typeCounts.map((t) => (
              <div key={t.name} className="flex items-center gap-1.5">
                <span
                  className="inline-block h-2 w-2 rounded-full"
                  style={{ backgroundColor: t.color }}
                />
                <span>{t.label}</span>
                <span className="font-mono text-caption-sm text-stone">
                  {t.count}
                </span>
              </div>
            ))
          ) : (
            // 주제군 이름 = 그 안에서 연결이 가장 많은 문서, 프로젝트 모드는
            // 프로젝트 이름. 큰 순 12개.
            communities.map((c) => (
              <div key={c.id} className="flex items-center gap-1.5">
                <span
                  className="inline-block h-2 w-2 shrink-0 rounded-full"
                  style={{ backgroundColor: c.color }}
                />
                <span className="max-w-[180px] truncate">{c.name}</span>
                <span className="font-mono text-caption-sm text-stone">
                  {c.count}
                </span>
              </div>
            ))
          )}
          <div className="flex items-center gap-1.5">
            <span className="inline-block h-2 w-2 rounded-full bg-on-dark-mute opacity-30" />
            <span>흐림: 대체됨 · 옅음: 완료</span>
          </div>
          <div className="flex items-center gap-1.5">
            <span className="inline-block h-2 w-2 rounded-full bg-on-dark-mute" />
            <span>크기: 연결 수에 비례</span>
          </div>
        </div>
      </div>

      <div>
        <div className="mb-1 text-caption-sm text-stone">링크</div>
        <div className="flex flex-col gap-0.5 text-caption-md text-body">
          <div className="flex items-center gap-1.5">
            <span className="inline-block h-[2px] w-5 bg-ink" />
            <span>
              관계 — 화살표 + 이름 (영향·원인…)
              <span className="ml-1 font-mono text-caption-sm text-stone">
                {typedEdges}
              </span>
            </span>
          </div>
          <div className="flex items-center gap-1.5">
            <span className="inline-block h-0.5 w-5 bg-on-dark-mute" />
            <span>확정 — 실제 wikilink</span>
          </div>
          <div className="flex items-center gap-1.5">
            <span className="inline-block h-0 w-5 border-t border-dashed border-accent-blue" />
            <span>제안 — AI가 찾은 후보</span>
          </div>
          <div className="flex items-center gap-1.5">
            <span className="inline-block h-0 w-5 border-t border-dashed border-accent-yellow" />
            <span>관련 — 내용 기반 유사도</span>
          </div>
        </div>
      </div>
    </div>
  );
}

function LinkTooltip({
  link,
  relKo,
}: {
  link: GLink;
  relKo: (name: string) => string;
}) {
  const s = link.source as GNode;
  const t = link.target as GNode;
  const kind =
    link.kind === "ghost" ? "제안" : link.kind === "soft" ? "내용 유사" : "확정";
  return (
    <div className="pointer-events-none absolute bottom-3 right-3 max-w-[360px] rounded-md border border-hairline bg-surface px-3 py-2 text-caption-md">
      <div className="flex items-center gap-1.5 text-mute">
        {kind}
        {link.relation && <RelationBadge>{relKo(link.relation)}</RelationBadge>}
      </div>
      <div className="mt-0.5 font-mono text-[12px] text-ink">
        <span className="truncate">{s?.domain ?? String(link.source)}</span>
        <span className="mx-1 text-stone">→</span>
        <span className="truncate">{t?.domain ?? String(link.target)}</span>
      </div>
      {link.reason && (
        <div className="mt-0.5 line-clamp-2 text-caption-sm text-stone">
          {link.reason}
        </div>
      )}
    </div>
  );
}

/** 관계 이름 뱃지 — 중립 톤(chrome 에 accent 금지). */
function RelationBadge({ children }: { children: React.ReactNode }) {
  return (
    <span className="inline-flex h-[18px] items-center rounded-xs border border-hairline-strong bg-surface-elevated px-1.5 text-[10px] normal-case tracking-normal text-on-dark">
      {children}
    </span>
  );
}

// Consumed only for typing — suppress unused warning via re-export alias.
export type { GhostSuggestion as _Unused };

/** 캔버스는 CSS 변수를 직접 못 읽으니 :root 의 토큰 값을 가져온다.
 *  라이트 모드로 바뀌면 다음 렌더에서 자연히 새 값. */
function cssToken(name: string): string {
  const v = getComputedStyle(document.documentElement)
    .getPropertyValue(`--color-${name}`)
    .trim();
  return v || "#888";
}

/** `#rrggbb` / `rgb(a)` 토큰에 알파를 입힌다 (링크·헤일로용). */
function withAlpha(color: string, alpha: number): string {
  const hex = color.match(/^#([0-9a-f]{6})$/i);
  if (hex) {
    const n = parseInt(hex[1], 16);
    return `rgba(${(n >> 16) & 255},${(n >> 8) & 255},${n & 255},${alpha})`;
  }
  const rgb = color.match(/^rgba?\(([^)]+)\)$/);
  if (rgb) {
    const [r, g, b] = rgb[1].split(",").map((x) => x.trim());
    return `rgba(${r},${g},${b},${alpha})`;
  }
  return color;
}
