import { useCallback, useEffect, useMemo, useState } from "react";
import { X } from "lucide-react";
import {
  ipc,
  type DocChangeEntry,
  type DocDiff,
  type DomainNode,
  type GhostLink,
  type WikiLink,
} from "@/lib/ipc";
import { inverseRelation, relationLabel } from "@/lib/ontology";
import { cn } from "@/lib/utils";
import { useApp } from "@/state/store";

/**
 * 문서 화면 우측 인스펙터 (⌘I) — 변경 · 링크 · 정보 세 탭.
 *
 * 변경: 이 문서의 git 이력(docChangeHistory) + 커밋별 unified diff +
 *       문서 단위 복원(docRestore: "at" = 그 버전으로, "before" = 그 변경
 *       직전으로). vault 전체 reset 이 아니라 이 파일만 되돌린다.
 * 링크: linkIndex 의 나가는/들어오는 링크 + 이 문서가 source 인 ghost
 *       제안(승인/무시).
 * 정보: 크기 · 수정 시각 · 경로 · kind.
 *
 * 하단 Backlinks 스트립을 대체한다. 탭은 localStorage 에 기억.
 */

type Tab = "changes" | "links" | "info";
const TAB_KEY = "danbi:inspector-tab";

function readTab(): Tab {
  try {
    const v = localStorage.getItem(TAB_KEY);
    if (v === "changes" || v === "links" || v === "info") return v;
  } catch {
    /* ignore */
  }
  return "changes";
}

export function DocInspector({
  project,
  domain,
  refreshKey,
  onOpenDoc,
  onRestored,
  onClose,
}: {
  project: string;
  domain: string;
  refreshKey: number;
  onOpenDoc: (project: string, domain: string) => void;
  /** 복원이 성공해 파일이 바뀐 뒤 — 에디터 재로드용. */
  onRestored?: () => void;
  onClose: () => void;
}) {
  const [tab, setTab] = useState<Tab>(readTab);
  useEffect(() => {
    try {
      localStorage.setItem(TAB_KEY, tab);
    } catch {
      /* ignore */
    }
  }, [tab]);

  const TABS: { key: Tab; label: string }[] = [
    { key: "changes", label: "변경" },
    { key: "links", label: "링크" },
    { key: "info", label: "정보" },
  ];

  return (
    <aside className="flex h-full w-[320px] shrink-0 flex-col border-l border-hairline bg-surface">
      <div className="flex h-10 shrink-0 items-center justify-between border-b border-hairline px-3">
        <div className="flex items-center gap-1">
          {TABS.map((t) => (
            <button
              key={t.key}
              type="button"
              onClick={() => setTab(t.key)}
              className={cn(
                "inline-flex h-6 items-center rounded-full px-2.5 text-[12px] transition-colors",
                tab === t.key
                  ? "bg-surface-elevated text-ink"
                  : "text-mute hover:text-on-dark",
              )}
            >
              {t.label}
            </button>
          ))}
        </div>
        <button
          type="button"
          onClick={onClose}
          title="인스펙터 닫기 (⌘I)"
          className="grid h-6 w-6 place-items-center rounded-sm text-mute transition-colors hover:bg-surface-elevated hover:text-on-dark"
        >
          <X size={13} />
        </button>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto p-4">
        {tab === "changes" && (
          <ChangesTab
            project={project}
            domain={domain}
            refreshKey={refreshKey}
            onRestored={onRestored}
          />
        )}
        {tab === "links" && (
          <LinksTab project={project} domain={domain} onOpenDoc={onOpenDoc} />
        )}
        {tab === "info" && <InfoTab project={project} domain={domain} />}
      </div>
    </aside>
  );
}

// ---------- 변경 ----------------------------------------------------------

function ChangesTab({
  project,
  domain,
  refreshKey,
  onRestored,
}: {
  project: string;
  domain: string;
  refreshKey: number;
  onRestored?: () => void;
}) {
  const [entries, setEntries] = useState<DocChangeEntry[] | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [diffs, setDiffs] = useState<Record<string, DocDiff | "loading" | string>>(
    {},
  );
  const [openDiff, setOpenDiff] = useState<string | null>(null);
  // 인라인 확인 — Tauri webview 의 window.confirm 은 믿지 않는다.
  const [confirm, setConfirm] = useState<{
    commit: string;
    mode: "before" | "at";
  } | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(() => {
    let alive = true;
    ipc
      .docChangeHistory(project, domain, 40)
      .then((rs) => {
        if (alive) setEntries(rs);
      })
      .catch((e) => {
        if (alive) {
          setEntries([]);
          setErr(String(e));
        }
      });
    return () => {
      alive = false;
    };
  }, [project, domain]);

  useEffect(() => {
    setEntries(null);
    setErr(null);
    setDiffs({});
    setOpenDiff(null);
    setConfirm(null);
    return load();
    // refreshKey: 외부(MCP) 쓰기·저장 직후 다시 받는다.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [load, refreshKey]);

  async function toggleDiff(commit: string) {
    if (openDiff === commit) {
      setOpenDiff(null);
      return;
    }
    setOpenDiff(commit);
    if (diffs[commit]) return;
    setDiffs((d) => ({ ...d, [commit]: "loading" }));
    try {
      const res = await ipc.docDiff(project, domain, commit);
      setDiffs((d) => ({ ...d, [commit]: res }));
    } catch (e) {
      setDiffs((d) => ({ ...d, [commit]: `오류: ${String(e).slice(0, 160)}` }));
    }
  }

  async function runRestore() {
    if (!confirm) return;
    setBusy(true);
    setErr(null);
    try {
      await ipc.docRestore(project, domain, confirm.commit, confirm.mode);
      setConfirm(null);
      setDiffs({});
      setOpenDiff(null);
      onRestored?.();
      load();
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  }

  if (entries === null) return <Skeleton />;
  if (entries.length === 0) {
    return (
      <Empty>
        변경 기록이 없어요. 저장하거나 에이전트가 쓰면 여기에 쌓여요.
      </Empty>
    );
  }

  return (
    <div className="flex flex-col gap-1">
      {err && <div className="text-caption-sm text-accent-red">{err}</div>}
      {entries.map((c, idx) => {
        const diff = diffs[c.commit];
        const isOpen = openDiff === c.commit;
        const confirming = confirm?.commit === c.commit;
        return (
          <div
            key={c.commit}
            className="rounded-md border border-hairline bg-surface-elevated px-2.5 py-2"
          >
            <div className="flex items-center gap-2">
              <span className="font-mono text-[11px] text-stone">
                {formatTs(c.ts)}
              </span>
              <span
                className={cn(
                  "rounded-xs px-1.5 py-px text-[10px] font-medium uppercase leading-none tracking-[0.2px]",
                  changeOpTone(c.op),
                )}
              >
                {changeOpLabel(c.op, c.mode)}
              </span>
              {idx === 0 && (
                <span className="text-[10px] text-mute">현재</span>
              )}
            </div>
            {c.target && (
              <div className="mt-1 truncate text-[12px] text-body" title={c.target}>
                {c.target}
              </div>
            )}
            <div className="mt-1.5 flex flex-wrap items-center gap-1">
              <SmallBtn active={isOpen} onClick={() => toggleDiff(c.commit)}>
                diff
              </SmallBtn>
              {idx > 0 && (
                <SmallBtn
                  onClick={() => setConfirm({ commit: c.commit, mode: "at" })}
                >
                  이 버전으로
                </SmallBtn>
              )}
              <SmallBtn
                onClick={() => setConfirm({ commit: c.commit, mode: "before" })}
              >
                이 변경 되돌리기
              </SmallBtn>
            </div>
            {confirming && confirm && (
              <div className="mt-2 rounded-md border border-hairline bg-surface p-2">
                <div className="text-[12px] text-on-dark">
                  {confirm.mode === "at"
                    ? "이 문서를 이 버전의 내용으로 되돌릴까요?"
                    : "이 변경만 취소하고 직전 상태로 되돌릴까요?"}
                </div>
                <div className="mt-0.5 text-caption-sm text-mute">
                  이 파일만 바뀌고 새 커밋으로 남아요. 다른 문서는 그대로.
                </div>
                <div className="mt-1.5 flex items-center gap-1.5">
                  <button
                    type="button"
                    disabled={busy}
                    onClick={runRestore}
                    className="inline-flex h-6 items-center rounded-sm bg-primary px-2 text-[11px] font-medium text-on-primary hover:bg-primary-pressed disabled:opacity-50"
                  >
                    {busy ? "되돌리는 중…" : "되돌리기"}
                  </button>
                  <button
                    type="button"
                    disabled={busy}
                    onClick={() => setConfirm(null)}
                    className="inline-flex h-6 items-center rounded-sm px-2 text-[11px] text-mute hover:text-on-dark"
                  >
                    취소
                  </button>
                </div>
              </div>
            )}
            {isOpen && (
              <div className="mt-2">
                {diff === "loading" || diff === undefined ? (
                  <div className="text-caption-sm text-mute">diff 읽는 중…</div>
                ) : typeof diff === "string" ? (
                  <div className="text-caption-sm text-accent-red">{diff}</div>
                ) : (
                  <DiffBlock diff={diff} />
                )}
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}

export function DiffBlock({ diff }: { diff: DocDiff }) {
  if (diff.binary) {
    return <div className="text-caption-sm text-mute">바이너리 변경이에요.</div>;
  }
  const lines = diff.unified.split("\n");
  if (lines.every((l) => l.trim() === "")) {
    return <div className="text-caption-sm text-mute">내용 변화가 없어요.</div>;
  }
  return (
    <div className="overflow-x-auto rounded-md border border-hairline bg-surface">
      <div className="flex items-center gap-2 border-b border-hairline px-2 py-1 text-[10px] text-mute">
        <span className="text-accent-green">+{diff.added}</span>
        <span className="text-accent-red">−{diff.removed}</span>
        {diff.base && (
          <span className="truncate font-mono">
            {diff.base.slice(0, 7)} → {diff.commit.slice(0, 7)}
          </span>
        )}
      </div>
      <pre className="whitespace-pre px-2 py-1.5 font-mono text-[11px] leading-[1.5]">
        {lines.map((l, i) => {
          // 파일 헤더(---/+++)는 +/- 로 시작하지만 변경 줄이 아니다.
          const isHeader =
            l.startsWith("@@") || l.startsWith("+++") || l.startsWith("---");
          const tone = isHeader
            ? "text-mute"
            : l.startsWith("+")
              ? "text-accent-green"
              : l.startsWith("-")
                ? "text-accent-red"
                : "text-body";
          return (
            <div key={i} className={tone}>
              {l.length === 0 ? " " : l}
            </div>
          );
        })}
      </pre>
    </div>
  );
}

// ---------- 링크 ----------------------------------------------------------

function LinksTab({
  project,
  domain,
  onOpenDoc,
}: {
  project: string;
  domain: string;
  onOpenDoc: (project: string, domain: string) => void;
}) {
  const index = useApp((s) => s.linkIndex);
  const key = `${project}/${domain}`;
  const outgoing: WikiLink[] = index?.outgoing[key] ?? [];
  const incoming: WikiLink[] = index?.incoming[key] ?? [];

  const [ghost, setGhost] = useState<GhostLink[] | null>(null);
  const [ghostErr, setGhostErr] = useState<string | null>(null);
  const loadGhost = useCallback(() => {
    let alive = true;
    ipc
      .ghostList(project)
      .then((st) => {
        if (!alive) return;
        setGhost(
          st.links.filter(
            (g) => g.status === "pending" && g.source_domain === domain,
          ),
        );
      })
      .catch(() => {
        if (alive) setGhost([]);
      });
    return () => {
      alive = false;
    };
  }, [project, domain]);
  useEffect(() => {
    setGhost(null);
    return loadGhost();
  }, [loadGhost]);

  async function decide(id: string, accept: boolean) {
    setGhostErr(null);
    try {
      if (accept) await ipc.ghostAccept(project, id);
      else await ipc.ghostReject(project, id);
      loadGhost();
    } catch (e) {
      setGhostErr(String(e));
    }
  }

  const nothing =
    outgoing.length === 0 && incoming.length === 0 && (ghost?.length ?? 0) === 0;

  return (
    <div className="flex flex-col gap-4">
      <Section label={`나가는 ${outgoing.length}`}>
        {outgoing.length === 0 ? (
          <Empty>나가는 링크가 없어요. 본문에 [[문서명]] 을 적으면 생겨요.</Empty>
        ) : (
          outgoing.map((l, i) => (
            <LinkRow
              key={`${l.project}/${l.domain}-${i}`}
              link={l}
              current={project}
              onClick={() => onOpenDoc(l.project, l.domain)}
            />
          ))
        )}
      </Section>
      <Section label={`들어오는 ${incoming.length}`}>
        {incoming.length === 0 ? (
          <Empty>이 문서를 참조하는 곳이 없어요.</Empty>
        ) : (
          incoming.map((l, i) => (
            <LinkRow
              key={`${l.project}/${l.domain}-${i}`}
              link={l}
              current={project}
              incoming
              onClick={() => onOpenDoc(l.project, l.domain)}
            />
          ))
        )}
      </Section>
      <Section label={`제안(ghost) ${ghost?.length ?? 0}`}>
        {ghost === null ? (
          <Skeleton rows={2} />
        ) : ghost.length === 0 ? (
          <Empty>
            {nothing
              ? "제안이 없어요. 그래프(⌘G)에서 스캔하면 관련 문서를 찾아요."
              : "대기 중인 제안이 없어요."}
          </Empty>
        ) : (
          ghost.map((g) => (
            <div
              key={g.id}
              className="rounded-md border border-hairline bg-surface-elevated px-2.5 py-2"
            >
              <div className="flex min-w-0 items-center gap-1.5">
                {g.relation && (
                  <RelTag>{relationLabel(g.relation)}</RelTag>
                )}
                <button
                  type="button"
                  onClick={() => onOpenDoc(project, g.target_domain)}
                  className="block min-w-0 flex-1 truncate text-left font-mono text-[12px] text-body hover:text-on-dark"
                  title={g.target_domain}
                >
                  → {g.target_domain}
                </button>
              </div>
              {g.proposed_by && g.proposed_by !== "scan" && (
                <div className="mt-0.5 text-caption-sm text-stone">
                  에이전트 제안 ({g.proposed_by})
                </div>
              )}
              {g.reason && (
                <div className="mt-0.5 line-clamp-2 text-caption-sm text-mute">
                  {g.reason}
                </div>
              )}
              {g.relation && g.relation !== "relates_to" && (
                <div className="mt-0.5 text-caption-sm text-stone">
                  승인하면{" "}
                  <span className="font-mono">
                    {relationLabel(g.relation)}:: [[…]]
                  </span>{" "}
                  로 적어요.
                </div>
              )}
              <div className="mt-1.5 flex items-center gap-1">
                <SmallBtn onClick={() => decide(g.id, true)}>승인</SmallBtn>
                <SmallBtn onClick={() => decide(g.id, false)}>무시</SmallBtn>
              </div>
            </div>
          ))
        )}
        {ghostErr && (
          <div className="text-caption-sm text-accent-red">{ghostErr}</div>
        )}
      </Section>
    </div>
  );
}

function LinkRow({
  link,
  current,
  incoming = false,
  onClick,
}: {
  link: WikiLink;
  current: string;
  /** 들어오는 링크면 relation 은 상대 문서가 적은 것 — 역방향으로 표시. */
  incoming?: boolean;
  onClick: () => void;
}) {
  // 들어오는 링크는 상대가 적은 관계의 역방향으로 읽는다 (원인 → 결과).
  const rel = link.relation
    ? incoming
      ? inverseRelation(link.relation)
      : link.relation
    : null;
  return (
    <button
      type="button"
      onClick={onClick}
      title={
        rel
          ? incoming
            ? `${link.domain} 에 '${relationLabel(link.relation!)}:: [[이 문서]]' 로 적혀 있어요`
            : `이 문서의 ${relationLabel(rel)}`
          : undefined
      }
      className="flex w-full items-center gap-1.5 rounded-md px-2 py-1.5 text-left text-[12px] text-body transition-colors hover:bg-surface-elevated hover:text-on-dark"
    >
      {rel && <RelTag>{relationLabel(rel)}</RelTag>}
      {link.project !== current && (
        <>
          <span className="shrink-0 text-mute">{link.project}</span>
          <span className="text-stone">/</span>
        </>
      )}
      <span className="truncate font-mono">{link.domain}</span>
    </button>
  );
}

// ---------- 정보 ----------------------------------------------------------

function InfoTab({ project, domain }: { project: string; domain: string }) {
  const tree = useApp((s) => s.tree);
  const index = useApp((s) => s.linkIndex);
  const node: DomainNode | null = useMemo(() => {
    const p = tree?.projects.find((x) => x.name === project);
    if (!p) return null;
    const top = p.domains.find((d) => d.name === domain);
    if (top) return top;
    for (const f of p.subfolders) {
      const hit = f.domains.find((d) => d.name === domain);
      if (hit) return hit;
      for (const g of f.subfolders ?? []) {
        const hit2 = g.domains.find((d) => d.name === domain);
        if (hit2) return hit2;
      }
    }
    return null;
  }, [tree, project, domain]);
  const key = `${project}/${domain}`;
  const out = index?.outgoing[key]?.length ?? 0;
  const inc = index?.incoming[key]?.length ?? 0;

  const rows: [string, string][] = [
    ["프로젝트", project],
    ["경로", domain],
    ["크기", node ? prettyBytes(node.bytes) : "—"],
    [
      "수정",
      node?.modified_ms ? formatTs(Math.floor(node.modified_ms / 1000)) : "—",
    ],
    ["링크", `나가는 ${out} · 들어오는 ${inc}`],
    ["kind", node?.kind ?? "—"],
  ];
  if (node?.title) rows.splice(2, 0, ["제목", node.title]);

  return (
    <div className="flex flex-col">
      {rows.map(([k, v]) => (
        <div
          key={k}
          className="flex items-start gap-3 border-b border-hairline py-2 last:border-b-0"
        >
          <span className="w-14 shrink-0 text-caption-sm text-mute">{k}</span>
          <span
            className={cn(
              "min-w-0 break-all text-[12px] text-body",
              k === "경로" && "font-mono",
            )}
          >
            {v}
          </span>
        </div>
      ))}
    </div>
  );
}

// ---------- 공용 조각 ---------------------------------------------------------

function Section({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <section>
      <div className="mb-1 text-caption-sm uppercase tracking-[0.4px] text-mute">
        {label}
      </div>
      <div className="flex flex-col gap-1">{children}</div>
    </section>
  );
}

function RelTag({ children }: { children: React.ReactNode }) {
  return (
    <span className="inline-flex h-[18px] shrink-0 items-center rounded-xs border border-hairline-strong bg-surface-elevated px-1.5 text-[10px] text-on-dark">
      {children}
    </span>
  );
}

function SmallBtn({
  children,
  onClick,
  active = false,
}: {
  children: React.ReactNode;
  onClick: () => void;
  active?: boolean;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "inline-flex h-6 items-center rounded-sm border px-2 text-[11px] transition-colors",
        active
          ? "border-hairline-strong bg-surface text-ink"
          : "border-hairline bg-surface text-mute hover:border-hairline-strong hover:text-on-dark",
      )}
    >
      {children}
    </button>
  );
}

function Empty({ children }: { children: React.ReactNode }) {
  return <div className="px-1 py-1 text-[12px] leading-[1.5] text-mute">{children}</div>;
}

function Skeleton({ rows = 3 }: { rows?: number }) {
  return (
    <div className="flex flex-col gap-1.5">
      {Array.from({ length: rows }).map((_, i) => (
        <div key={i} className="h-9 animate-pulse rounded-md bg-surface-elevated" />
      ))}
    </div>
  );
}

function formatTs(tsSec: number): string {
  const dt = new Date(tsSec * 1000);
  const mm = String(dt.getMonth() + 1).padStart(2, "0");
  const dd = String(dt.getDate()).padStart(2, "0");
  const hh = String(dt.getHours()).padStart(2, "0");
  const mi = String(dt.getMinutes()).padStart(2, "0");
  return `${mm}-${dd} ${hh}:${mi}`;
}

function prettyBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

/**
 * 변경 히스토리 row 의 op label. backend 가 분류한 키(`upsert_item`,
 * `replace_section`, `append`, `apply`, …) 를 사람이 읽을 한국어로.
 * `upsert_item` 은 mode (update/add) 까지 합쳐 분간.
 * (DocView.tsx 에서 옮겨옴 — 그쪽 사본은 삭제 예정.)
 */
export function changeOpLabel(op: string, mode?: string | null): string {
  if (op === "upsert_item") {
    if (mode === "update") return "항목 갱신";
    if (mode === "add") return "항목 추가";
    return "upsert";
  }
  if (op === "replace_section") return "섹션 교체";
  if (op === "append") return "추가";
  if (op === "insert_after") return "삽입";
  if (op === "rewrite_all") return "전체 재작성";
  if (op === "apply") return "AI 편집";
  if (op === "undo") return "되돌리기";
  if (op === "quick_capture") return "Quick Capture";
  if (op === "compound") return "Compound";
  return op;
}

/**
 * op 별로 어울리는 색 토큰 매핑. 외부 LLM 의 비파괴적 갱신은 차분한
 * blue/green, 파괴적/대규모는 yellow/red.
 */
export function changeOpTone(op: string): string {
  switch (op) {
    case "upsert_item":
      return "bg-accent-green-soft text-accent-green";
    case "replace_section":
      return "bg-accent-blue-soft text-accent-blue";
    case "append":
    case "insert_after":
      return "bg-surface-elevated text-stone";
    case "rewrite_all":
    case "undo":
      return "bg-accent-yellow-soft text-accent-yellow";
    default:
      return "bg-surface-elevated text-stone";
  }
}
