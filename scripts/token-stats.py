#!/usr/bin/env python3
"""
오늘 하루치 Claude Code 세션을 훑어서 단비 ON / OFF 토큰 사용량을 비교한다.

분류 규칙:
  - 세션 transcript 안에 `mcp__danbi__*` tool_use 가 한 번이라도 있으면 ON
  - 없으면 OFF

집계: input / cache_read / cache_creation / output / 턴 수 / 추정 USD

결과는 한 줄 표 + Danbi vault 의 stats/token-usage/<오늘>.md 에 append.
"""
from __future__ import annotations
import json, os, sys, glob, datetime, pathlib

CC_PROJECTS = pathlib.Path.home() / ".claude" / "projects"
VAULT = pathlib.Path.home() / "Danbi_Vault"
STATS_DIR = VAULT / "Projects" / "단비" / "stats" / "token-usage"

# Opus 4.7 가격 (per 1M tokens, 2026-05 기준 추정 — Sonnet 등 다른 모델은 따로 계산해도 OK)
PRICE = {
    "claude-opus-4-7":   {"in": 15.0, "out": 75.0, "cache_read": 1.5, "cache_write": 18.75},
    "claude-sonnet-4-6": {"in":  3.0, "out": 15.0, "cache_read": 0.3, "cache_write":  3.75},
    "claude-haiku-4-5":  {"in":  1.0, "out":  5.0, "cache_read": 0.1, "cache_write":  1.25},
}
DEFAULT = PRICE["claude-opus-4-7"]


def cost(model: str, usage: dict) -> float:
    p = PRICE.get(model, DEFAULT)
    return (
        usage.get("input_tokens", 0)               * p["in"] / 1e6
        + usage.get("output_tokens", 0)            * p["out"] / 1e6
        + usage.get("cache_read_input_tokens", 0)  * p["cache_read"] / 1e6
        + usage.get("cache_creation_input_tokens", 0) * p["cache_write"] / 1e6
    )


def session_stats(path: pathlib.Path, today: datetime.date):
    """Return (touched_today, used_danbi, totals) — totals only counts today's lines."""
    touched = False
    used_danbi = False
    totals = {
        "input": 0, "cache_read": 0, "cache_creation": 0, "output": 0,
        "turns": 0, "cost": 0.0, "model": None,
        "first_ts": None, "last_ts": None,
    }
    try:
        with path.open() as f:
            for line in f:
                try:
                    obj = json.loads(line)
                except json.JSONDecodeError:
                    continue
                ts = obj.get("timestamp")
                if not ts:
                    continue
                try:
                    d = datetime.datetime.fromisoformat(ts.replace("Z", "+00:00")).astimezone().date()
                except ValueError:
                    continue
                if d != today:
                    continue
                touched = True
                # iso timestamp for wall-clock window
                try:
                    epoch = datetime.datetime.fromisoformat(ts.replace("Z", "+00:00")).timestamp()
                    if totals["first_ts"] is None or epoch < totals["first_ts"]:
                        totals["first_ts"] = epoch
                    if totals["last_ts"] is None or epoch > totals["last_ts"]:
                        totals["last_ts"] = epoch
                except ValueError:
                    pass
                msg = obj.get("message") or {}
                # tool_use scan for mcp__danbi__
                content = msg.get("content")
                if isinstance(content, list):
                    for c in content:
                        if isinstance(c, dict) and c.get("type") == "tool_use":
                            if str(c.get("name", "")).startswith("mcp__danbi__"):
                                used_danbi = True
                u = msg.get("usage")
                if u:
                    model = msg.get("model") or totals["model"]
                    totals["model"] = model
                    totals["input"]          += u.get("input_tokens", 0)
                    totals["cache_read"]     += u.get("cache_read_input_tokens", 0)
                    totals["cache_creation"] += u.get("cache_creation_input_tokens", 0)
                    totals["output"]         += u.get("output_tokens", 0)
                    totals["turns"]          += 1
                    totals["cost"]           += cost(model or "", u)
    except OSError:
        pass
    return touched, used_danbi, totals


def fmt_int(n: int) -> str:
    return f"{n:,}"


def fmt_dur(seconds: float) -> str:
    s = int(round(seconds))
    h, rem = divmod(s, 3600)
    m, s = divmod(rem, 60)
    if h:
        return f"{h}h{m:02d}m"
    if m:
        return f"{m}m{s:02d}s"
    return f"{s}s"


def main():
    today = datetime.date.today()
    if len(sys.argv) > 1:
        today = datetime.date.fromisoformat(sys.argv[1])

    on = {"sessions": 0, "input": 0, "cache_read": 0, "cache_creation": 0, "output": 0, "turns": 0, "cost": 0.0, "wall": 0.0}
    off = dict(on)
    rows = []

    for jsonl in CC_PROJECTS.glob("*/*.jsonl"):
        touched, used_danbi, t = session_stats(jsonl, today)
        if not touched:
            continue
        bucket = on if used_danbi else off
        bucket["sessions"]       += 1
        bucket["input"]          += t["input"]
        bucket["cache_read"]     += t["cache_read"]
        bucket["cache_creation"] += t["cache_creation"]
        bucket["output"]         += t["output"]
        bucket["turns"]          += t["turns"]
        bucket["cost"]           += t["cost"]
        wall = 0.0
        if t["first_ts"] is not None and t["last_ts"] is not None:
            wall = max(0.0, t["last_ts"] - t["first_ts"])
        bucket["wall"]           += wall
        rows.append((
            "ON" if used_danbi else "OFF",
            jsonl.parent.name.replace("-Users-hazbola-", ""),
            jsonl.stem[:8],
            t["turns"], t["input"], t["cache_read"], t["output"], t["cost"], wall,
        ))

    # Console output
    print(f"# Token usage — {today}")
    print()
    print(f"{'':4} {'sessions':>8} {'turns':>6} {'input':>11} {'cache_read':>13} {'output':>9} {'wall':>7} {'cost':>8}")
    for label, b in (("ON", on), ("OFF", off)):
        print(f"{label:>4} {b['sessions']:>8} {b['turns']:>6} {fmt_int(b['input']):>11} {fmt_int(b['cache_read']):>13} {fmt_int(b['output']):>9} {fmt_dur(b['wall']):>7} ${b['cost']:>7.2f}")
    print()
    print("## sessions")
    for r in sorted(rows, key=lambda x: -x[3]):
        label, proj, sid, turns, inp, cr, out, c, w = r
        print(f"  [{label:3}] {turns:3} turns · in={fmt_int(inp):>10} · cache_read={fmt_int(cr):>10} · out={fmt_int(out):>7} · wall={fmt_dur(w):>6} · ${c:.2f}  {proj}/{sid}")

    # Vault append
    STATS_DIR.mkdir(parents=True, exist_ok=True)
    md = STATS_DIR / f"{today.isoformat()}.md"
    now = datetime.datetime.now().strftime("%H:%M")
    snapshot = []
    snapshot.append(f"\n## snapshot {now}\n")
    snapshot.append("| 모드 | 세션 | 턴 | input | cache_read | cache_creation | output | wall | 추정 비용 |")
    snapshot.append("|---|---:|---:|---:|---:|---:|---:|---:|---:|")
    for label, b in (("ON 🟢", on), ("OFF ⚪", off)):
        snapshot.append(
            f"| {label} | {b['sessions']} | {b['turns']} | {fmt_int(b['input'])} | "
            f"{fmt_int(b['cache_read'])} | {fmt_int(b['cache_creation'])} | "
            f"{fmt_int(b['output'])} | {fmt_dur(b['wall'])} | ${b['cost']:.2f} |"
        )
    if on["turns"] and off["turns"]:
        # turn 당 평균 (신규) input + 턴당 wall clock 비교
        avg_on  = on["input"]  / on["turns"]
        avg_off = off["input"] / off["turns"]
        wall_per_turn_on  = on["wall"]  / on["turns"]  if on["turns"]  else 0
        wall_per_turn_off = off["wall"] / off["turns"] if off["turns"] else 0
        snapshot.append("")
        snapshot.append(f"- ON 턴당 평균 input: {avg_on:,.0f} tok · 턴당 wall: {fmt_dur(wall_per_turn_on)}")
        snapshot.append(f"- OFF 턴당 평균 input: {avg_off:,.0f} tok · 턴당 wall: {fmt_dur(wall_per_turn_off)}")
        if avg_off > 0:
            ratio = avg_on / avg_off
            snapshot.append(f"- ON / OFF 토큰 비율: **{ratio:.2f}×** (1.0 미만이면 단비가 토큰을 절약)")
        if wall_per_turn_off > 0:
            wratio = wall_per_turn_on / wall_per_turn_off
            snapshot.append(f"- ON / OFF 시간 비율: **{wratio:.2f}×** (1.0 미만이면 단비가 시간을 절약)")

    snapshot.append("")
    if not md.exists():
        md.write_text(f"# Token usage — {today}\n\n_단비 ON 은 그 세션에서 `mcp__danbi__*` tool 을 1번이라도 호출한 경우, OFF 는 그 외._\n")
    with md.open("a") as f:
        f.write("\n".join(snapshot))
    print(f"\n→ snapshot appended: {md}")


if __name__ == "__main__":
    main()
