import { useMemo } from "react";
import type { GhostLink, WikiLink } from "@/lib/ipc";
import { inverseRelation, relationLabel } from "@/lib/ontology";
import { useProjectColor } from "@/lib/projectColor";
import { cn } from "@/lib/utils";

/**
 * 문서 인스펙터 링크 탭 위쪽의 작은 연결 지도.
 *
 * 가운데가 이 문서, 둘레가 한 단계 이웃(나가는 · 들어오는 · ghost 제안).
 * 이웃은 프로젝트별로 모아 배치하고, 바깥 호(arc)로 어느 프로젝트에
 * 이어져 있는지 보여 준다. 색은 그래프 뷰와 같은 프로젝트 색.
 * 선: 뜻 있는 관계(영향·원인…) 는 굵고 밝게, 일반 [[링크]] 는 옅게,
 * ghost 제안은 점선. 화살표 방향 = 링크를 적은 쪽 → 가리키는 쪽.
 */

export type LinkNeighbor = {
  key: string;
  project: string;
  domain: string;
  /** 이 문서가 거는 링크 — 관계 이름(없으면 ""). */
  out?: string;
  /** 이 문서를 거는 링크 — 이 문서 입장의 관계 이름(역방향, 없으면 ""). */
  in?: string;
  /** 연결은 없고 ghost 제안만 있음. */
  ghost?: string;
};

const W = 288;
const H = 216;
const CX = W / 2;
const CY = 106;
const MAX_NODES = 18;
// 글자 뒤에 바탕색 테두리 — 선 위에 놓여도 읽히게.
const HALO = { stroke: "var(--color-canvas)", strokeWidth: 3, paintOrder: "stroke", strokeLinejoin: "round" } as const;

function isTyped(rel?: string): boolean {
  return !!rel && rel !== "relates_to";
}

function shortName(domain: string, max: number): string {
  const base = domain.split("/").pop()!.replace(/\.md$/, "");
  return base.length > max ? `${base.slice(0, max - 1)}…` : base;
}

/** 나가는 · 들어오는 · ghost 를 문서 하나당 한 이웃으로 합친다. */
export function buildNeighbors(
  project: string,
  domain: string,
  outgoing: WikiLink[],
  incoming: WikiLink[],
  ghost: GhostLink[] | null,
): LinkNeighbor[] {
  const self = `${project}/${domain}`;
  const map = new Map<string, LinkNeighbor>();
  const get = (p: string, d: string) => {
    const key = `${p}/${d}`;
    let n = map.get(key);
    if (!n) {
      n = { key, project: p, domain: d };
      map.set(key, n);
    }
    return n;
  };
  for (const l of outgoing) {
    if (`${l.project}/${l.domain}` === self) continue;
    const n = get(l.project, l.domain);
    if (!isTyped(n.out)) n.out = l.relation ?? "";
  }
  for (const l of incoming) {
    if (`${l.project}/${l.domain}` === self) continue;
    const n = get(l.project, l.domain);
    const rel = l.relation ? inverseRelation(l.relation) : "";
    if (!isTyped(n.in)) n.in = rel;
  }
  for (const g of ghost ?? []) {
    const key = `${project}/${g.target_domain}`;
    if (key === self || map.has(key)) continue;
    get(project, g.target_domain).ghost = g.relation ?? "";
  }
  return Array.from(map.values());
}

type Placed = LinkNeighbor & { x: number; y: number; angle: number; color: string };

export function LinkMap({
  project,
  domain,
  neighbors,
  hot,
  onHot,
  onOpenDoc,
}: {
  project: string;
  domain: string;
  neighbors: LinkNeighbor[];
  /** 강조 중인 이웃 key 또는 "project:<이름>". */
  hot: string | null;
  onHot: (key: string | null) => void;
  onOpenDoc: (project: string, domain: string) => void;
}) {
  const projectColor = useProjectColor();

  // 프로젝트별 묶음 — 이 문서의 프로젝트 먼저, 나머지는 많은 순.
  const groups = useMemo(() => {
    const m = new Map<string, LinkNeighbor[]>();
    for (const n of neighbors) {
      const arr = m.get(n.project) ?? [];
      arr.push(n);
      m.set(n.project, arr);
    }
    for (const arr of m.values()) arr.sort((a, b) => a.domain.localeCompare(b.domain));
    return Array.from(m.entries()).sort((a, b) => {
      if (a[0] === project) return -1;
      if (b[0] === project) return 1;
      return b[1].length - a[1].length || a[0].localeCompare(b[0]);
    });
  }, [neighbors, project]);

  const { placed, arcs, hidden, radius } = useMemo(() => {
    // 많으면 MAX_NODES 개만 — 프로젝트마다 비율대로, 적어도 하나씩은 남겨
    // 어느 프로젝트와 이어졌는지는 빠지지 않게.
    const all = groups.reduce((n, [, arr]) => n + arr.length, 0);
    const quota = groups.map(([, arr]) =>
      all <= MAX_NODES ? arr.length : Math.max(1, Math.floor((arr.length * MAX_NODES) / all)),
    );
    let left = MAX_NODES - quota.reduce((a, b) => a + b, 0);
    for (let i = 0; left > 0 && i < groups.length; i++) {
      const extra = Math.min(left, groups[i][1].length - quota[i]);
      quota[i] += extra;
      left -= extra;
    }
    const kept = groups.map(([p, arr], i) => [p, arr.slice(0, quota[i])] as const);
    const shown = kept.flatMap(([, arr]) => arr);

    const r = shown.length <= 4 ? 62 : shown.length <= 10 ? 72 : 80;
    // 묶음 사이에 빈 칸을 0.8칸 둬서 프로젝트 경계가 보이게.
    const gap = kept.length > 1 ? 0.8 : 0;
    const slots = shown.length + gap * kept.length;
    const step = slots > 0 ? (Math.PI * 2) / slots : 0;
    let a = -Math.PI / 2 - (shown.length === 1 ? 0 : step / 2) + (gap * step) / 2;
    if (shown.length === 1) a = 0; // 하나면 오른쪽에
    const out: Placed[] = [];
    const arcList: { project: string; a0: number; a1: number; color: string }[] = [];
    for (const [p, arr] of kept) {
      const color = projectColor(p);
      const a0 = a;
      for (const n of arr) {
        a += step / 2;
        out.push({ ...n, angle: a, x: CX + Math.cos(a) * r, y: CY + Math.sin(a) * r, color });
        a += step / 2;
      }
      arcList.push({ project: p, a0, a1: a, color });
      a += gap * step;
    }
    return { placed: out, arcs: arcList, hidden: all - shown.length, radius: r };
  }, [groups, projectColor]);

  const showLabels = placed.length <= 10;
  const selfColor = projectColor(project);
  const hotProject = hot?.startsWith("project:") ? hot.slice(8) : null;
  const isDim = (n: LinkNeighbor) =>
    hot !== null && (hotProject ? n.project !== hotProject : n.key !== hot);
  const hotNode = !hotProject && hot ? placed.find((n) => n.key === hot) : undefined;

  const totals = groups.map(([p, arr]) => ({ project: p, count: arr.length, color: projectColor(p) }));
  const total = neighbors.length;

  return (
    <div className="flex flex-col gap-2.5">
      <div className="overflow-hidden rounded-lg border border-hairline bg-canvas">
        <svg
          viewBox={`0 0 ${W} ${H}`}
          width="100%"
          className="block select-none"
          onMouseLeave={() => onHot(null)}
        >
          {/* 궤도 */}
          <circle
            cx={CX}
            cy={CY}
            r={radius}
            fill="none"
            style={{ stroke: "var(--color-hairline)" }}
            strokeDasharray={total === 0 ? "3 4" : undefined}
          />

          {/* 프로젝트 호 — 묶음이 둘 이상일 때만 */}
          {arcs.length > 1 &&
            arcs.map((g) => {
              const R = radius + 17;
              const pad = 0.06;
              const a0 = g.a0 + pad;
              const a1 = Math.max(a0 + 0.01, g.a1 - pad);
              const large = a1 - a0 > Math.PI ? 1 : 0;
              const d = `M ${CX + Math.cos(a0) * R} ${CY + Math.sin(a0) * R} A ${R} ${R} 0 ${large} 1 ${CX + Math.cos(a1) * R} ${CY + Math.sin(a1) * R}`;
              const dim = hotProject ? hotProject !== g.project : !!hotNode && hotNode.project !== g.project;
              return (
                <path
                  key={g.project}
                  d={d}
                  fill="none"
                  stroke={g.color}
                  strokeWidth={3}
                  strokeLinecap="round"
                  opacity={dim ? 0.15 : 0.55}
                  className="cursor-default transition-opacity"
                  onMouseEnter={() => onHot(`project:${g.project}`)}
                >
                  <title>{g.project}</title>
                </path>
              );
            })}

          {/* 선 */}
          {placed.map((n) => {
            const dx = n.x - CX;
            const dy = n.y - CY;
            const len = Math.hypot(dx, dy) || 1;
            const ux = dx / len;
            const uy = dy / len;
            const x0 = CX + ux * 14;
            const y0 = CY + uy * 14;
            const x1 = n.x - ux * 9;
            const y1 = n.y - uy * 9;
            const typed = isTyped(n.out) || isTyped(n.in);
            const dim = isDim(n);
            const stroke = n.ghost !== undefined && n.out === undefined && n.in === undefined
              ? "var(--color-stone)"
              : typed
                ? "var(--color-ink)"
                : "var(--color-mute)";
            const arrow = (tx: number, ty: number, dirx: number, diry: number) => {
              const s = 5;
              const px = -diry;
              const py = dirx;
              return `M ${tx} ${ty} L ${tx - dirx * s + px * s * 0.6} ${ty - diry * s + py * s * 0.6} L ${tx - dirx * s - px * s * 0.6} ${ty - diry * s - py * s * 0.6} Z`;
            };
            return (
              <g
                key={`e-${n.key}`}
                opacity={dim ? 0.12 : typed ? 0.85 : 0.5}
                className="transition-opacity"
                style={{ color: stroke }}
              >
                <line
                  x1={x0}
                  y1={y0}
                  x2={x1}
                  y2={y1}
                  stroke="currentColor"
                  strokeWidth={typed ? 1.6 : 1}
                  strokeDasharray={n.out === undefined && n.in === undefined ? "3 3" : undefined}
                />
                {n.out !== undefined && <path d={arrow(x1, y1, ux, uy)} fill="currentColor" />}
                {n.in !== undefined && <path d={arrow(x0, y0, -ux, -uy)} fill="currentColor" />}
              </g>
            );
          })}

          {/* 가운데 — 이 문서 */}
          <circle cx={CX} cy={CY} r={11} fill={selfColor} />
          <circle
            cx={CX}
            cy={CY}
            r={14}
            fill="none"
            strokeWidth={1.2}
            style={{ stroke: "var(--color-ink)" }}
            opacity={0.35}
          />
          <text
            x={CX}
            y={CY + 29}
            textAnchor="middle"
            fontSize={10.5}
            fontWeight={600}
            style={{ fill: "var(--color-ink)", ...HALO }}
          >
            {shortName(domain, 14)}
          </text>

          {/* 이웃 */}
          {placed.map((n) => {
            const dim = isDim(n);
            const onlyGhost = n.out === undefined && n.in === undefined;
            const c = Math.cos(n.angle);
            const anchor = c > 0.35 ? "start" : c < -0.35 ? "end" : "middle";
            const lx = anchor === "start" ? n.x + 10 : anchor === "end" ? n.x - 10 : n.x;
            const ly = anchor === "middle" ? n.y + (Math.sin(n.angle) > 0 ? 18 : -11) : n.y + 3.5;
            return (
              <g
                key={`n-${n.key}`}
                className="cursor-pointer transition-opacity"
                opacity={dim ? 0.25 : 1}
                onMouseEnter={() => onHot(n.key)}
                onClick={() => onOpenDoc(n.project, n.domain)}
              >
                <circle cx={n.x} cy={n.y} r={13} fill="transparent" />
                <circle
                  cx={n.x}
                  cy={n.y}
                  r={hot === n.key ? 7.5 : 6.5}
                  fill={onlyGhost ? "none" : n.color}
                  stroke={n.color}
                  strokeWidth={onlyGhost ? 1.4 : 0}
                  strokeDasharray={onlyGhost ? "2 2" : undefined}
                />
                {(showLabels || hot === n.key) && (
                  <text
                    x={lx}
                    y={ly}
                    textAnchor={anchor}
                    fontSize={9.5}
                    style={{ fill: hot === n.key ? "var(--color-ink)" : "var(--color-mute)", ...HALO }}
                  >
                    {shortName(n.domain, 6)}
                  </text>
                )}
              </g>
            );
          })}

          {hidden > 0 && (
            <text x={W - 10} y={H - 10} textAnchor="end" fontSize={10} style={{ fill: "var(--color-stone)" }}>
              +{hidden}개 더
            </text>
          )}
        </svg>

        {/* 마우스를 올린 이웃 — 전체 이름과 관계 */}
        <div className="flex h-7 min-w-0 items-center gap-1.5 border-t border-hairline px-2.5 text-[11px]">
          {hotNode ? (
            <>
              <span className="h-2 w-2 shrink-0 rounded-full" style={{ background: hotNode.color }} />
              {hotNode.project !== project && (
                <span className="shrink-0 text-mute">{hotNode.project} /</span>
              )}
              <span className="min-w-0 flex-1 truncate font-mono text-body">{hotNode.domain}</span>
              <span className="shrink-0 text-stone">{neighborBadge(hotNode)}</span>
            </>
          ) : (
            <span className="truncate text-stone">
              {total === 0 ? "아직 이어진 문서가 없어요" : "점에 올리면 이름, 누르면 그 문서로"}
            </span>
          )}
        </div>
      </div>

      {/* 어느 프로젝트에 이어졌나 */}
      {total > 0 && (
        <div className="flex flex-col gap-1.5">
          <div className="flex h-1.5 overflow-hidden rounded-full bg-surface-elevated">
            {totals.map((t) => (
              <div
                key={t.project}
                className="h-full transition-opacity"
                style={{
                  width: `${(t.count / total) * 100}%`,
                  background: t.color,
                  opacity: hotProject && hotProject !== t.project ? 0.25 : 1,
                }}
              />
            ))}
          </div>
          <div className="flex flex-wrap gap-x-2.5 gap-y-1" onMouseLeave={() => onHot(null)}>
            {totals.map((t) => (
              <button
                key={t.project}
                type="button"
                onMouseEnter={() => onHot(`project:${t.project}`)}
                className={cn(
                  "inline-flex items-center gap-1 text-[11px] transition-colors",
                  hotProject === t.project ? "text-ink" : "text-mute hover:text-on-dark",
                )}
              >
                <span className="h-2 w-2 rounded-full" style={{ background: t.color }} />
                {t.project === project ? `${t.project} (이 프로젝트)` : t.project}
                <span className="text-stone">{t.count}</span>
              </button>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

/** "나가는 · 영향" 같은 짧은 설명. */
function neighborBadge(n: LinkNeighbor): string {
  const parts: string[] = [];
  if (n.out !== undefined && n.in !== undefined) parts.push("양방향");
  else if (n.out !== undefined) parts.push("나가는");
  else if (n.in !== undefined) parts.push("들어오는");
  else parts.push("제안");
  const rel = [n.out, n.in, n.ghost].find(isTyped);
  if (rel) parts.push(relationLabel(rel));
  return parts.join(" · ");
}
