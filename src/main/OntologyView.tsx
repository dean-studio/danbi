import { useEffect, useMemo, useState } from "react";
import { ChevronDown, ChevronRight, HelpCircle, Search, X } from "lucide-react";
import {
  ipc,
  type OntologyDocRow,
  type OntologyDocs,
  type OntologySchema,
} from "@/lib/ipc";
import {
  DEFAULT_TYPE_ORDER,
  relationLabel,
  splitDocId,
  statusLabel,
  typeLabel,
} from "@/lib/ontology";
import { cn } from "@/lib/utils";
import { useApp } from "@/state/store";

/** 관계 열에서 접기 전에 보여줄 개수. */
const REL_VISIBLE = 3;
const DOC_LIMIT = 500;

/**
 * 온톨로지 표 — 한 프로젝트의 문서를 종류 · 상태 · 관계로 본다.
 * GraphView 처럼 전체 화면 오버레이. 데이터는 `ontology_schema`(어휘·개수)
 * + `ontology_docs`(행, daily 제외, 최근 수정순). 필터는 서버에서 건다.
 * 행을 누르면 그래프와 같은 동작(selectDomain → 닫기)으로 문서를 연다.
 */
export function OntologyView({
  project,
  onClose,
}: {
  /** null 이면 닫힘. */
  project: string | null;
  onClose: () => void;
}) {
  const selectDomain = useApp((s) => s.selectDomain);
  const [schema, setSchema] = useState<OntologySchema | null>(null);
  const [docs, setDocs] = useState<OntologyDocs | null>(null);
  const [loading, setLoading] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [docType, setDocType] = useState("");
  const [status, setStatus] = useState("");
  const [relation, setRelation] = useState("");
  const [query, setQuery] = useState("");
  const [howToOpen, setHowToOpen] = useState(false);

  // 프로젝트가 바뀌면 필터 초기화 + schema 다시.
  useEffect(() => {
    if (!project) return;
    setDocType("");
    setStatus("");
    setRelation("");
    setQuery("");
    setSchema(null);
    let alive = true;
    ipc
      .ontologySchema(project)
      .then((s) => alive && setSchema(s))
      .catch((e) => alive && setErr(String(e)));
    return () => {
      alive = false;
    };
  }, [project]);

  useEffect(() => {
    if (!project) return;
    let alive = true;
    setLoading(true);
    setErr(null);
    ipc
      .ontologyDocs({
        project,
        docType: docType || null,
        status: status || null,
        relation: relation || null,
        limit: DOC_LIMIT,
      })
      .then((d) => alive && setDocs(d))
      .catch((e) => alive && setErr(String(e)))
      .finally(() => alive && setLoading(false));
    return () => {
      alive = false;
    };
  }, [project, docType, status, relation]);

  useEffect(() => {
    if (!project) return;
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") onClose();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [project, onClose]);

  // 종류 칩 — 기본 순서 → schema 가 더한 종류 → 어휘 밖(모르는) 값.
  const typeChips = useMemo(() => {
    if (!schema) return [] as Array<{ name: string; count: number; known: boolean }>;
    const counts = schema.counts_by_type;
    const known = new Set(schema.types.map((t) => t.name));
    const names = new Set<string>([
      ...schema.types.map((t) => t.name),
      ...Object.keys(counts),
    ]);
    // daily 는 표에서 빠지므로 칩도 뺀다 (개수만 요약에 남김).
    names.delete("daily");
    return Array.from(names)
      .map((name) => ({ name, count: counts[name] ?? 0, known: known.has(name) }))
      .filter((t) => t.count > 0 || DEFAULT_TYPE_ORDER.includes(t.name))
      .sort((a, b) => {
        const ia = DEFAULT_TYPE_ORDER.indexOf(a.name);
        const ib = DEFAULT_TYPE_ORDER.indexOf(b.name);
        if (ia >= 0 && ib >= 0) return ia - ib;
        if (ia >= 0) return -1;
        if (ib >= 0) return 1;
        return b.count - a.count;
      });
  }, [schema]);

  const totalDocs = useMemo(
    () =>
      Object.entries(schema?.counts_by_type ?? {})
        .filter(([k]) => k !== "daily")
        .reduce((a, [, n]) => a + n, 0),
    [schema],
  );

  const rows = useMemo(() => {
    const q = query.trim().toLowerCase();
    const list = docs?.docs ?? [];
    if (!q) return list;
    return list.filter((d) => d.domain.toLowerCase().includes(q));
  }, [docs, query]);

  if (!project) return null;

  function openDoc(p: string, d: string) {
    selectDomain(p, d);
    onClose();
  }

  const filtered = !!(docType || status || relation);

  return (
    <div className="fixed inset-0 z-30 flex flex-col bg-canvas">
      <header
        data-tauri-drag-region
        className="flex h-10 shrink-0 items-center justify-between border-b border-hairline bg-surface/80 px-3 backdrop-blur"
      >
        <div className="flex items-center gap-2 text-[13px] text-ink">
          <span className="font-medium">온톨로지</span>
          <span className="inline-flex h-5 items-center rounded-full border border-hairline bg-surface-elevated px-2 text-[11px] text-on-dark">
            @{project}
          </span>
          {loading && <span className="text-caption-sm text-stone">로딩…</span>}
        </div>
        <button
          onClick={onClose}
          className="grid h-7 w-7 place-items-center rounded-sm text-mute transition-colors hover:bg-surface-elevated hover:text-on-dark"
          title="닫기 (Esc)"
        >
          <X size={14} />
        </button>
      </header>

      <div className="flex-1 min-h-0 overflow-auto">
        <div className="mx-auto max-w-[1120px] px-6 py-6">
          {/* 요약 — schema 의 종류별 개수 · 선언 수 · 관계 링크 수 */}
          <section className="rounded-lg border border-hairline bg-surface p-4">
            <div className="flex flex-wrap items-baseline justify-between gap-2">
              <div className="text-[13px] font-medium text-ink">
                문서 {totalDocs}
                <span className="ml-2 text-caption-md font-normal text-mute">
                  종류 직접 적음 {schema?.declared_types ?? 0} · 관계 있는 링크{" "}
                  {schema?.typed_links ?? 0}
                </span>
              </div>
              <div className="flex items-center gap-3 text-caption-sm text-stone">
                {schema?.inherited_from && (
                  <span>어휘: {schema.inherited_from} 에서 상속</span>
                )}
                {(schema?.counts_by_type.daily ?? 0) > 0 && (
                  <span>일지 {schema?.counts_by_type.daily} (표에서 제외)</span>
                )}
                <button
                  onClick={() => setHowToOpen((v) => !v)}
                  className="inline-flex items-center gap-1 text-mute hover:text-on-dark"
                >
                  <HelpCircle size={11} /> 어떻게 적나요?
                </button>
              </div>
            </div>

            <div className="mt-3 flex flex-wrap items-center gap-1.5">
              <Pill on={docType === ""} onClick={() => setDocType("")}>
                전체 종류
              </Pill>
              {typeChips.map((t) => (
                <Pill
                  key={t.name}
                  on={docType === t.name}
                  onClick={() => setDocType(docType === t.name ? "" : t.name)}
                  title={t.known ? t.name : `${t.name} — schema 에 없는 종류`}
                >
                  <span className={t.known ? undefined : "italic"}>
                    {typeLabel(t.name, schema)}
                  </span>
                  <span className="font-mono text-[11px] text-stone">{t.count}</span>
                </Pill>
              ))}
            </div>

            {howToOpen && schema && (
              <ul className="mt-3 flex flex-col gap-1 border-t border-hairline pt-3 text-caption-md leading-[1.6] text-body">
                {schema.how_to.map((line, i) => (
                  <li key={i} className="flex gap-2">
                    <span className="text-stone">·</span>
                    <span>{line}</span>
                  </li>
                ))}
              </ul>
            )}
          </section>

          {/* 필터 줄 */}
          <div className="mt-4 flex flex-wrap items-center gap-2">
            <FilterSelect
              label="상태"
              value={status}
              onChange={setStatus}
              options={(schema?.statuses ?? []).map((s) => ({
                value: s.name,
                label: statusLabel(s.name, schema),
              }))}
            />
            <FilterSelect
              label="관계"
              value={relation}
              onChange={setRelation}
              options={(schema?.relations ?? []).map((r) => ({
                value: r.name,
                label: relationLabel(r.name, schema),
              }))}
            />
            <div className="inline-flex h-7 items-center gap-1 rounded-md border border-hairline bg-surface-elevated px-2">
              <Search size={11} className="text-stone" />
              <input
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="문서 이름으로 거르기"
                className="h-6 w-[160px] bg-transparent text-[12px] text-body outline-none placeholder:text-stone"
              />
            </div>
            {filtered && (
              <button
                onClick={() => {
                  setDocType("");
                  setStatus("");
                  setRelation("");
                }}
                className="text-caption-md text-mute hover:text-on-dark"
              >
                필터 지우기
              </button>
            )}
            <span className="ml-auto text-caption-sm text-stone">
              {docs ? `${rows.length} / ${docs.total}건` : ""}
              {docs && docs.total > docs.docs.length
                ? ` · 최근 ${docs.docs.length}건만`
                : ""}
            </span>
          </div>

          {err && (
            <div className="mt-3 rounded-md border border-hairline bg-surface-elevated px-3 py-2 text-caption-md text-accent-red">
              {err}
            </div>
          )}

          {/* 표 */}
          <div className="mt-3 overflow-hidden rounded-lg border border-hairline bg-surface">
            <div className="grid grid-cols-[minmax(0,2.2fr)_110px_80px_minmax(0,2.6fr)_92px] gap-3 border-b border-hairline px-4 py-2 text-[10px] uppercase tracking-[0.6px] text-mute">
              <span>문서</span>
              <span>종류</span>
              <span>상태</span>
              <span>관계</span>
              <span className="text-right">수정일</span>
            </div>
            {!docs && loading ? (
              <ul className="flex flex-col gap-1 p-3">
                {[0, 1, 2, 3].map((i) => (
                  <li
                    key={i}
                    className="h-9 animate-pulse rounded-md bg-surface-elevated"
                  />
                ))}
              </ul>
            ) : rows.length === 0 ? (
              <div className="px-4 py-8 text-center text-[13px] text-mute">
                {filtered || query
                  ? "조건에 맞는 문서가 없어요."
                  : "문서가 없어요."}
              </div>
            ) : (
              <ul>
                {rows.map((d) => (
                  <DocRowView
                    key={`${d.project}/${d.domain}`}
                    row={d}
                    schema={schema}
                    onOpen={() => openDoc(d.project, d.domain)}
                    onOpenRelated={(id) => {
                      const [p, dom] = splitDocId(id);
                      openDoc(p || d.project, dom);
                    }}
                  />
                ))}
              </ul>
            )}
          </div>
          <div className="mt-3 text-caption-sm text-stone">
            종류가 옅게 "폴더" 로 표시된 문서는 type 을 안 적어 폴더·파일 이름으로
            판단한 거예요. 관계는 들어오는 링크까지 이 문서 쪽에서 읽은 이름이에요
            (예: 상대가 원인:: 으로 적으면 여기선 결과).
          </div>
        </div>
      </div>
    </div>
  );
}

function DocRowView({
  row,
  schema,
  onOpen,
  onOpenRelated,
}: {
  row: OntologyDocRow;
  schema: OntologySchema | null;
  onOpen: () => void;
  onOpenRelated: (id: string) => void;
}) {
  const [expanded, setExpanded] = useState(false);
  const rels = expanded ? row.relations : row.relations.slice(0, REL_VISIBLE);
  const hidden = row.relations.length - rels.length;
  const folder = row.type_source === "folder";
  const superseded = row.status === "superseded";

  return (
    <li
      onClick={onOpen}
      className="grid cursor-pointer grid-cols-[minmax(0,2.2fr)_110px_80px_minmax(0,2.6fr)_92px] items-start gap-3 border-b border-hairline px-4 py-2 transition-colors last:border-b-0 hover:bg-surface-elevated"
    >
      <span
        className={cn(
          "truncate py-0.5 font-mono text-[12px]",
          superseded ? "text-stone line-through" : "text-ink",
        )}
        title={row.domain}
      >
        {row.domain}
      </span>
      <span className="flex items-center gap-1.5 py-0.5 text-[12px]">
        <span className={folder ? "text-mute" : "text-body"}>
          {typeLabel(row.doc_type, schema)}
        </span>
        {folder && (
          <span
            className="rounded-xs border border-hairline px-1 text-[10px] text-stone"
            title="type 을 안 적어서 폴더·파일 이름으로 판단했어요"
          >
            폴더
          </span>
        )}
      </span>
      <span className="py-0.5 text-[12px] text-body">
        {row.status ? statusLabel(row.status, schema) : (
          <span className="text-stone">—</span>
        )}
      </span>
      <span className="flex min-w-0 flex-col gap-0.5">
        {row.relations.length === 0 ? (
          <span className="py-0.5 text-[12px] text-stone">—</span>
        ) : (
          rels.map((r, i) => {
            const [p, dom] = splitDocId(r.doc);
            return (
              <button
                key={`${r.relation}-${r.doc}-${i}`}
                onClick={(e) => {
                  e.stopPropagation();
                  onOpenRelated(r.doc);
                }}
                className="flex min-w-0 items-center gap-1.5 rounded-sm text-left text-[12px] hover:text-on-dark"
                title={r.doc}
              >
                <span className="inline-flex h-[18px] shrink-0 items-center rounded-xs border border-hairline-strong bg-surface-elevated px-1.5 text-[10px] text-on-dark">
                  {relationLabel(r.relation, schema)}
                </span>
                <span className="truncate font-mono text-body">
                  {p && p !== row.project ? `${p}/` : ""}
                  {dom}
                </span>
              </button>
            );
          })
        )}
        {(hidden > 0 || expanded) && row.relations.length > REL_VISIBLE && (
          <button
            onClick={(e) => {
              e.stopPropagation();
              setExpanded((v) => !v);
            }}
            className="inline-flex items-center gap-0.5 text-left text-caption-sm text-mute hover:text-on-dark"
          >
            {expanded ? <ChevronDown size={11} /> : <ChevronRight size={11} />}
            {expanded ? "접기" : `+${hidden} 더`}
          </button>
        )}
      </span>
      <span className="py-0.5 text-right font-mono text-[11px] text-stone">
        {row.modified_ms ? fmtDate(row.modified_ms) : "—"}
      </span>
    </li>
  );
}

function Pill({
  on,
  onClick,
  title,
  children,
}: {
  on: boolean;
  onClick: () => void;
  title?: string;
  children: React.ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      title={title}
      className={cn(
        "inline-flex h-7 items-center gap-1.5 rounded-full border px-3 text-[12px] transition-colors",
        on
          ? "border-hairline-strong bg-surface-card text-on-dark"
          : "border-hairline bg-surface text-mute hover:text-on-dark",
      )}
    >
      {children}
    </button>
  );
}

function FilterSelect({
  label,
  value,
  onChange,
  options,
}: {
  label: string;
  value: string;
  onChange: (v: string) => void;
  options: Array<{ value: string; label: string }>;
}) {
  return (
    <label className="inline-flex h-7 items-center gap-1.5 rounded-md border border-hairline bg-surface-elevated px-2 text-[12px]">
      <span className="text-stone">{label}</span>
      <select
        value={value}
        onChange={(e) => onChange(e.target.value)}
        className="bg-transparent text-body outline-none"
      >
        <option value="">전체</option>
        {options.map((o) => (
          <option key={o.value} value={o.value}>
            {o.label}
          </option>
        ))}
      </select>
    </label>
  );
}

function fmtDate(ms: number): string {
  const d = new Date(ms);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}
