import { useCallback, useEffect, useMemo, useState } from "react";
import { ChevronDown, ChevronRight, Info } from "lucide-react";
import { ipc, type TimeSplit } from "@/lib/ipc";
import { useProjectColor } from "@/lib/projectColor";
import { cn } from "@/lib/utils";
import { useApp } from "@/state/store";
import { formatMinutes } from "@/main/pulseUi";
import { WarmupPanel, useWarmup, useWarmupAccents } from "@/main/Warmup";

/** 홈 오른쪽 열 — "이번 주 어디에 시간을 썼나".
 *  시간 = Claude Code 응답이 있었던 5분 칸 수 (코드 폴더 → 연결된 프로젝트),
 *  기록 = vault 커밋 수. 연결 안 된 폴더는 아래에서 바로 프로젝트에 묶는다. */

const PERIOD_KEY = "danbi:time-split-days";
const ROWS_VISIBLE = 6;

type FolderRow = { key: string; label: string; folder: string; minutes: number };

/** 폴더 하나씩 묶는다 — 형제를 부모로 합치면 서로 다른 저장소가 한 프로젝트로
 *  섞일 수 있어서. 이름이 겹치면 서로 구분될 때까지 앞쪽 경로를 더 붙인다
 *  (teams/a/roles/tester · teams/b/roles/tester). */
function folderRows(rows: TimeSplit["unmatched"]): FolderRow[] {
  const parts = rows.map((r) => r.cwd.split("/").filter(Boolean));
  const depth = rows.map(() => 1);
  for (let round = 0; round < 12; round++) {
    const labels = parts.map((p, i) => p.slice(-depth[i]).join("/"));
    const count = new Map<string, number>();
    for (const l of labels) count.set(l, (count.get(l) ?? 0) + 1);
    let changed = false;
    labels.forEach((l, i) => {
      if ((count.get(l) ?? 0) > 1 && depth[i] < parts[i].length) {
        depth[i] += 1;
        changed = true;
      }
    });
    if (!changed) break;
  }
  return rows.map((r, i) => ({
    key: r.cwd,
    folder: r.cwd,
    minutes: r.minutes,
    label: parts[i].length ? parts[i].slice(-depth[i]).join("/") : r.label,
  }));
}

export function TimeSplitCard() {
  const tree = useApp((s) => s.tree);
  const selectProject = useApp((s) => s.selectProject);
  const setCfg = useApp((s) => s.setCfg);
  const showToast = useApp((s) => s.showToast);
  const projectColor = useProjectColor();
  const [days, setDays] = useState<number>(() => {
    try {
      return localStorage.getItem(PERIOD_KEY) === "30" ? 30 : 7;
    } catch {
      return 7;
    }
  });
  const [data, setData] = useState<TimeSplit | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [all, setAll] = useState(false);
  const [showUnmatched, setShowUnmatched] = useState(false);
  // 첫 호출은 Claude Code 대화 기록 전체를 파싱해서 오래 걸린다 — 실제 진행률로 보여 준다.
  const warm = useWarmup("usage", !data && !err);
  const warmAccents = useWarmupAccents(4);

  const load = useCallback(async () => {
    try {
      setData(await ipc.timeSplit(days));
      setErr(null);
    } catch (e) {
      setErr(String(e));
    }
  }, [days]);
  useEffect(() => {
    setData(null);
    load();
    try {
      localStorage.setItem(PERIOD_KEY, String(days));
    } catch {
      /* 무시 */
    }
  }, [days, load]);

  const projects = useMemo(() => tree?.projects.map((p) => p.name) ?? [], [tree]);
  const folders = useMemo(() => folderRows(data?.unmatched ?? []), [data]);

  async function bind(folder: string, project: string) {
    try {
      setCfg(await ipc.folderProjectSet(folder, project));
      showToast("ok", `${folder.split("/").pop()} → ${project} 로 셀게요`);
      load();
    } catch (e) {
      showToast("err", String(e));
    }
  }

  const rows = data?.rows ?? [];
  const shown = all ? rows : rows.slice(0, ROWS_VISIBLE);

  return (
    <section className="rounded-lg border border-hairline bg-surface">
      <div className="flex items-center justify-between px-4 pb-1 pt-2.5">
        <span className="text-[13px] font-medium text-ink">어디에 시간을 썼나</span>
        <div className="flex items-center gap-0.5 rounded-full border border-hairline p-0.5">
          {[7, 30].map((d) => (
            <button
              key={d}
              onClick={() => setDays(d)}
              className={cn(
                "h-5 rounded-full px-2 text-[11px] transition-colors",
                days === d ? "bg-surface-card text-on-dark" : "text-mute hover:text-on-dark",
              )}
            >
              {d}일
            </button>
          ))}
        </div>
      </div>

      {err ? (
        <div className="px-4 pb-3 text-[12px] text-accent-red">{err}</div>
      ) : !data ? (
        // 600ms 안에 오면 아무것도 안 그린다(번쩍임 방지). 그 전까지는 자리만 잡아 둔다.
        warm.visible ? (
          <WarmupPanel state={warm} rows={3} accents={warmAccents} className="px-4 pb-3 pt-1" />
        ) : (
          <div className="h-[142px]" aria-hidden />
        )
      ) : rows.length === 0 ? (
        <div className={cn("px-4 pb-3 text-[12px] leading-[1.6] text-mute", warm.revealClass)}>
          최근 {days}일 기록이 없어요.
        </div>
      ) : (
        <div className={warm.revealClass}>
          <div
            className="px-4 text-[11px] text-stone"
            title="Claude Code 응답이 있었던 5분 칸을 셉니다. 에이전트 여러 개가 동시에 돌면 각각 셉니다. 기록 = 단비 vault 에 쌓인 커밋 수."
          >
            {data.by_minutes
              ? `에이전트 작업 ${formatMinutes(data.total_minutes)} · 기록 ${data.total_records}건`
              : `기록 ${data.total_records}건 기준`}
            <Info size={10} className="ml-1 inline align-[-1px]" />
          </div>
          <div className="mx-4 mt-2 flex h-2 overflow-hidden rounded-full bg-surface-elevated">
            {rows
              .filter((r) => r.share > 0.005)
              .map((r) => (
                <span
                  key={r.project}
                  style={{ width: `${r.share * 100}%`, background: projectColor(r.project) }}
                  title={`${r.project} ${Math.round(r.share * 100)}%`}
                />
              ))}
          </div>
          <ul className="mt-1.5 pb-1">
            {shown.map((r) => (
              <li key={r.project}>
                <button
                  onClick={() => selectProject(r.project)}
                  className="flex w-full items-center gap-2 px-4 py-1 text-left text-[12px] transition-colors hover:bg-surface-elevated"
                  title={`${r.project} — 세션 ${r.sessions}개 · 토큰 ${r.tokens.toLocaleString()} · MCP ${r.mcp_calls}회`}
                >
                  <span className="h-1.5 w-1.5 shrink-0 rounded-full" style={{ background: projectColor(r.project) }} />
                  <span className="min-w-0 flex-1 truncate text-body">{r.project}</span>
                  <span className="shrink-0 font-mono text-[11px] text-stone">
                    {data.by_minutes && r.minutes > 0 ? formatMinutes(r.minutes) : `기록 ${r.records}`}
                  </span>
                  <span className="w-[34px] shrink-0 text-right font-mono text-[11px] text-mute">
                    {Math.round(r.share * 100)}%
                  </span>
                </button>
              </li>
            ))}
          </ul>
          {rows.length > ROWS_VISIBLE && (
            <button
              onClick={() => setAll((v) => !v)}
              className="w-full border-t border-hairline px-4 py-1.5 text-left text-[11px] text-mute hover:text-on-dark"
            >
              {all ? "접기" : `+${rows.length - ROWS_VISIBLE}개 프로젝트`}
            </button>
          )}
        </div>
      )}

      {data && !data.tracking && (
        <div className="border-t border-hairline px-4 py-2 text-[11px] leading-[1.5] text-stone">
          Claude Code 사용량 추적이 꺼져 있어 기록 수로만 보여요 (설정 → 사용량).
        </div>
      )}
      {data && data.tracking && data.connected_folders + data.manual_folders === 0 && (
        <div className="border-t border-hairline px-4 py-2 text-[11px] leading-[1.5] text-stone">
          연결 시트에서 코드 폴더를 프로젝트에 연결하면 Claude Code 작업 시간이 프로젝트별로 잡혀요.
        </div>
      )}
      {data && folders.length > 0 && (
        <div className="border-t border-hairline">
          <button
            onClick={() => setShowUnmatched((v) => !v)}
            className="flex w-full items-center gap-1 px-4 py-1.5 text-left text-[11px] text-mute hover:text-on-dark"
            title="프로젝트에 연결되지 않은 코드 폴더에서 일한 시간"
          >
            {showUnmatched ? <ChevronDown size={11} /> : <ChevronRight size={11} />}
            연결 안 된 폴더 {formatMinutes(data.unmatched_minutes)}
          </button>
          {showUnmatched && (
            <ul className="pb-2">
              {folders.map((f) => (
                <li key={f.key} className="flex items-center gap-2 px-4 py-1 text-[12px]">
                  <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-body" title={f.folder}>
                    {f.label}
                  </span>
                  <span className="shrink-0 font-mono text-[11px] text-stone">{formatMinutes(f.minutes)}</span>
                  <label className="relative inline-flex h-5 shrink-0 items-center rounded-sm border border-hairline px-1.5 text-[10px] text-mute hover:border-hairline-strong hover:text-on-dark">
                    <select
                      value=""
                      onChange={(e) => e.target.value && bind(f.folder, e.target.value)}
                      className="absolute inset-0 cursor-pointer opacity-0"
                      aria-label={`${f.label} 를 프로젝트에 묶기`}
                    >
                      <option value="">프로젝트에 묶기</option>
                      {projects.map((p) => (
                        <option key={p} value={p}>
                          {p}
                        </option>
                      ))}
                    </select>
                    묶기
                  </label>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
    </section>
  );
}
