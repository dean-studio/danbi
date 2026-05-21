import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import ForceGraph2D, {
  type ForceGraphMethods,
  type LinkObject,
  type NodeObject,
} from "react-force-graph-2d";
import { Filter, Lightbulb, Palette, Search, X } from "lucide-react";
import {
  ipc,
  type GhostSuggestion,
  type GraphData,
  type GraphInsights,
} from "@/lib/ipc";
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
  color?: string;
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
};

type ColorMode = "project" | "community";

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
  const [showSoft, setShowSoft] = useState(true);
  const [colorMode, setColorMode] = useState<ColorMode>("community");
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
    setLoading(true);
    setErr(null);
    ipc
      .buildGraph(projectFilter || undefined)
      .then(setData)
      .catch((e) => setErr(String(e)))
      .finally(() => setLoading(false));
  }, [open, projectFilter]);

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
      charge.strength(singleProject ? -600 : -380);
    }

    const link = inst.d3Force("link");
    if (link && typeof link.distance === "function") {
      // Longer target edge length = more breathing room between connected
      // nodes. Paired with a weaker link strength (below) so the
      // repulsion wins out on densely-connected clusters.
      link.distance(singleProject ? 180 : 140);
    }
    if (link && typeof link.strength === "function") {
      // Default link strength (1/min-degree) pulls hard on hub-like
      // nodes, which squishes their neighborhoods together. Clamping to a
      // gentle constant lets the charge repulsion breathe.
      link.strength(0.12);
    }

    // Gentle pull toward origin so the hub-centric layout stays framed.
    // The hub itself is already pinned (fx/fy), so this mostly affects
    // peripheral nodes and long ghost-link chains.
    const center = inst.d3Force("center");
    if (center && typeof center.strength === "function") {
      center.strength(singleProject ? 0.04 : 0.02);
    }
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
  // shuffle colors around. The palette is taken from the design tokens.
  const projectColor = useCallback((project: string): string => {
    const palette = ["#57c1ff", "#59d499", "#ffc533", "#ff6161"];
    let h = 0;
    for (const ch of project) h = (h * 31 + ch.charCodeAt(0)) | 0;
    return palette[Math.abs(h) % palette.length];
  }, []);

  // Louvain community color palette — 20 distinct hues. -1 (isolated)
  // renders muted so the eye skips over them.
  const communityColor = useCallback((id: number): string => {
    if (id < 0) return "#6a6b6c";
    const palette = [
      "#57c1ff", "#59d499", "#ffc533", "#ff6161", "#b47cff",
      "#7fffd4", "#ff8c42", "#5eead4", "#fb7185", "#a78bfa",
      "#fbbf24", "#34d399", "#60a5fa", "#f472b6", "#f87171",
      "#4ade80", "#38bdf8", "#fcd34d", "#c084fc", "#facc15",
    ];
    return palette[id % palette.length];
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
    const nodes: GNode[] = data.nodes
      .filter((n) => visibleIds.has(n.id))
      .map((n) => {
        const isHub = n.id === hubId;
        return {
          ...n,
          color:
            colorMode === "community"
              ? communityColor(n.community)
              : projectColor(n.project),
          isHub,
          fx: isHub ? 0 : undefined,
          fy: isHub ? 0 : undefined,
        };
      });

    // 2) edge 필터링: kind 토글 + score 임계 + 양 끝 노드가 모두 보일 때만.
    const links: GLink[] = data.edges
      .filter((e) => {
        if (e.kind === "ghost" && !showGhost) return false;
        if (e.kind === "soft" && !showSoft) return false;
        if (e.kind === "confirmed" && !showConfirmed) return false;
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
      }));
    return { nodes, links };
  }, [
    data,
    projectFilter,
    showConfirmed,
    showGhost,
    showSoft,
    colorMode,
    projectColor,
    communityColor,
    minDegree,
    minScore,
  ]);

  const projects = useMemo(() => {
    if (!data) return [] as string[];
    const s = new Set<string>();
    for (const n of data.nodes) s.add(n.project);
    return Array.from(s).sort();
  }, [data]);

  const q = query.trim().toLowerCase();

  // Edge color / dash pattern per kind:
  //   confirmed → bright hairline (ground truth)
  //   ghost     → accent blue, dashed (AI proposal, waiting for user)
  //   soft      → accent yellow, thin dashed (implicit relevance hint)
  function linkColor(l: GLink): string {
    if (l.kind === "ghost") return "#57c1ff";
    if (l.kind === "soft") return "rgba(255,197,51,0.55)";
    return "rgba(255,255,255,0.55)";
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
      const baseR = Math.max(3, 3 + Math.sqrt(Math.max(1, node.bytes)) / 40);
      const r = node.isHub ? Math.max(baseR * 2.2, baseR + 8) : baseR;
      const matches =
        q.length > 0 &&
        (node.label.toLowerCase().includes(q) ||
          node.project.toLowerCase().includes(q));

      const x = (node.x ?? 0) as number;
      const y = (node.y ?? 0) as number;

      // Soft radial halo behind the hub. Pure-canvas gradient so the
      // effect scales with the camera without hitting layout.
      if (node.isHub) {
        const haloR = r * 2.4;
        const grad = ctx.createRadialGradient(x, y, r * 0.9, x, y, haloR);
        grad.addColorStop(0, "rgba(255,255,255,0.18)");
        grad.addColorStop(1, "rgba(255,255,255,0)");
        ctx.beginPath();
        ctx.arc(x, y, haloR, 0, Math.PI * 2);
        ctx.fillStyle = grad;
        ctx.fill();
      }

      ctx.beginPath();
      ctx.arc(x, y, r, 0, Math.PI * 2);
      ctx.fillStyle = node.color ?? "#888";
      ctx.fill();

      if (node.isHub) {
        // Double ring: thicker inner stroke in the node color's white
        // highlight, thin outer ring for separation from the halo.
        ctx.strokeStyle = "rgba(255,255,255,0.95)";
        ctx.lineWidth = 2 / scale;
        ctx.stroke();
      } else if (matches || hovered?.id === node.id) {
        ctx.strokeStyle = "#fff";
        ctx.lineWidth = 1.5 / scale;
        ctx.stroke();
      }

      const showLabel =
        !crowded ||
        scale > 1.4 ||
        matches ||
        hovered?.id === node.id ||
        node.isHub;
      if (showLabel) {
        const fontPx = node.isHub
          ? Math.max(13, 14 / scale)
          : Math.max(10, 11 / scale);
        ctx.font = `${node.isHub ? "600 " : ""}${fontPx}px Inter, sans-serif`;
        ctx.fillStyle =
          matches || hovered?.id === node.id || node.isHub
            ? "#f4f4f6"
            : "#cdcdcd";
        ctx.textAlign = "left";
        ctx.textBaseline = "middle";
        ctx.fillText(prettyLabel(node.label), x + r + 4 / scale, y);
      }
    },
    [hovered, q, crowded],
  );

  const linkCanvasObject = useCallback(
    (link: GLink, ctx: CanvasRenderingContext2D) => {
      const src = link.source as GNode;
      const tgt = link.target as GNode;
      const sx = (src?.x ?? 0) as number;
      const sy = (src?.y ?? 0) as number;
      const tx = (tgt?.x ?? 0) as number;
      const ty = (tgt?.y ?? 0) as number;

      ctx.beginPath();
      ctx.moveTo(sx, sy);
      ctx.lineTo(tx, ty);
      ctx.strokeStyle = linkColor(link);
      // Thicken by score: a relevance 1.5 soft edge reads thinner than a
      // 3.0 confirmed wiki link. Confirmed stays bold so ground truth
      // still dominates visually.
      const base =
        link.kind === "ghost" ? 2.2 : link.kind === "soft" ? 1.1 : 2.0;
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
    },
    [],
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

          <button
            onClick={() =>
              setColorMode((m) => (m === "project" ? "community" : "project"))
            }
            title={`노드 색: ${colorMode === "project" ? "프로젝트별" : "커뮤니티별"} (클릭해서 전환)`}
            className="inline-flex h-7 items-center gap-1 rounded-sm border border-hairline bg-surface-elevated px-2 text-[12px] text-body hover:text-on-dark"
          >
            <Palette size={11} />
            {colorMode === "project" ? "프로젝트" : "커뮤니티"}
          </button>

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
              const baseR = Math.max(
                6,
                6 + Math.sqrt(Math.max(1, n.bytes)) / 40,
              );
              // Match the visual radius bump for hubs so the hit area
              // matches what the user sees.
              const r = n.isHub ? Math.max(baseR * 2.2, baseR + 8) : baseR;
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
          <div className="pointer-events-none absolute bottom-3 right-3 rounded-md border border-hairline bg-surface px-3 py-2 text-caption-md shadow-lg shadow-black/40">
            <div className="text-mute">{hovered.project}</div>
            <div className="text-[13px] font-medium text-ink">
              {prettyLabel(hovered.label)}
            </div>
            <div className="mt-0.5 font-mono text-caption-sm text-stone">
              {hovered.domain}
            </div>
            <div className="text-caption-sm text-stone">
              {(hovered.bytes / 1024).toFixed(1)} KB
            </div>
          </div>
        )}

        {data && data.nodes.length > 0 && (
          <GraphLegend colorMode={colorMode} />
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
              onAccept={() => acceptGhost(popup.link)}
              onReject={() => rejectGhost(popup.link)}
              onClose={() => setPopup(null)}
            />
          ) : (
            <ConfirmedPopup
              x={popup.x}
              y={popup.y}
              link={popup.link}
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
  onAccept,
  onReject,
  onClose,
}: {
  x: number;
  y: number;
  link: GLink;
  onAccept: () => void;
  onReject: () => void;
  onClose: () => void;
}) {
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
      className="fixed z-40 w-[300px] rounded-md border border-hairline bg-surface p-3 shadow-2xl shadow-black/50"
      style={{
        left: Math.min(x, window.innerWidth - 320),
        top: Math.min(y + 4, window.innerHeight - 160),
      }}
    >
      <div className="mb-2 flex items-center justify-between">
        <span className="text-[11px] uppercase tracking-[0.4px] text-accent-blue">
          AI 제안 링크
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
      {link.reason && (
        <div className="mb-3 text-caption-sm leading-[1.5] text-body">
          {link.reason}
        </div>
      )}
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
  onOpen,
  onClose,
}: {
  x: number;
  y: number;
  link: GLink;
  onOpen: (node: GNode) => void;
  onClose: () => void;
}) {
  // For confirmed edges both endpoints are always full node objects because
  // the force-graph replaces the string ids with references before drawing.
  const src = link.source as GNode;
  const tgt = link.target as GNode;
  return (
    <div
      className="fixed z-40 w-[320px] rounded-md border border-hairline bg-surface p-3 shadow-2xl shadow-black/50"
      style={{
        left: Math.min(x, window.innerWidth - 340),
        top: Math.min(y + 4, window.innerHeight - 180),
      }}
    >
      <div className="mb-2 flex items-center justify-between">
        <span className="text-[11px] uppercase tracking-[0.4px] text-on-dark-mute">
          확정 링크
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
        <span className="text-mute">{src.domain}</span> 의 본문에{" "}
        <span className="font-mono text-on-dark">[[{tgt.label.replace(/\.md$/i, "")}]]</span>{" "}
        참조가 있어서 이어진 선이에요.
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
    <div className="absolute right-3 top-3 z-20 flex max-h-[calc(100%-24px)] w-[320px] flex-col overflow-hidden rounded-md border border-hairline bg-surface/95 shadow-xl shadow-black/40 backdrop-blur">
      <header className="flex items-center gap-2 border-b border-hairline px-3 py-2 text-[12px] font-medium text-ink">
        <Lightbulb size={12} className="text-accent-yellow" />
        그래프 인사이트
      </header>
      <div className="flex-1 min-h-0 overflow-auto px-3 py-2">
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

function GraphLegend({ colorMode }: { colorMode: ColorMode }) {
  return (
    <div className="pointer-events-none absolute bottom-3 left-3 select-none rounded-md border border-hairline bg-surface/90 px-3 py-2 shadow-lg shadow-black/40 backdrop-blur">
      <div className="mb-1.5 text-[10px] uppercase tracking-[0.6px] text-mute">
        범례
      </div>

      <div className="mb-2">
        <div className="mb-1 text-caption-sm text-stone">노드</div>
        <div className="flex flex-col gap-0.5 text-caption-md text-body">
          <div className="flex items-center gap-1.5">
            <span
              className="inline-block h-2 w-2 rounded-full"
              style={{ background: "#57c1ff" }}
            />
            <span>
              {colorMode === "community"
                ? "커뮤니티 (같은 색 = 같은 주제군)"
                : "프로젝트별 색상"}
            </span>
          </div>
          <div className="flex items-center gap-1.5">
            <span className="inline-block h-2 w-2 rounded-full bg-on-dark-mute" />
            <span>크기: 문서 분량에 비례</span>
          </div>
        </div>
      </div>

      <div>
        <div className="mb-1 text-caption-sm text-stone">링크</div>
        <div className="flex flex-col gap-0.5 text-caption-md text-body">
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

// Consumed only for typing — suppress unused warning via re-export alias.
export type { GhostSuggestion as _Unused };
