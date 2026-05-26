import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { useApp } from "@/state/store";

const LAST_CHECK_KEY = "danbi.lastUpdateCheck";
const CHECK_INTERVAL_MS = 24 * 60 * 60 * 1000;

let pendingUpdate: Update | null = null;

/** Run an update check against the configured GitHub Releases endpoint.
 *  When `force=false`, throttle to once per 24h based on a localStorage
 *  timestamp so the background launch check doesn't hammer GitHub on
 *  every relaunch during heavy debugging. Manual checks (Settings →
 *  업데이트 확인) pass force=true. */
export async function runUpdateCheck(force: boolean): Promise<void> {
  if (!force) {
    const last = Number(localStorage.getItem(LAST_CHECK_KEY));
    if (Number.isFinite(last) && Date.now() - last < CHECK_INTERVAL_MS) {
      return;
    }
  }
  try {
    const update = await check();
    localStorage.setItem(LAST_CHECK_KEY, String(Date.now()));
    if (update) {
      pendingUpdate = update;
      useApp.getState().setUpdateInfo({
        status: "available",
        version: update.version,
        currentVersion: update.currentVersion,
        notes: update.body ?? null,
      });
    } else if (force) {
      // 수동 체크일 때만 "최신 버전입니다" 신호를 띄울 수 있게 ready
      // 상태로 잠깐 표기 후 비움. 자동 체크는 silent.
      useApp.getState().setUpdateInfo(null);
    }
  } catch (e) {
    console.error("[danbi] update check failed", e);
    if (force) {
      useApp.getState().setUpdateInfo({
        status: "error",
        version: null,
        message: String(e),
      });
    }
  }
}

/** Download + install the previously-found update, then relaunch. The
 *  Update object is held in a module-scope ref because the plugin's
 *  Update handle isn't serializable into zustand. */
export async function applyPendingUpdate(): Promise<void> {
  const upd = pendingUpdate;
  if (!upd) return;
  const { setUpdateInfo } = useApp.getState();
  setUpdateInfo({ status: "downloading", version: upd.version, progress: 0 });
  try {
    let downloaded = 0;
    let total = 0;
    await upd.downloadAndInstall((event) => {
      if (event.event === "Started") {
        total = event.data.contentLength ?? 0;
      } else if (event.event === "Progress") {
        downloaded += event.data.chunkLength;
        const progress = total > 0 ? Math.min(1, downloaded / total) : 0;
        setUpdateInfo({
          status: "downloading",
          version: upd.version,
          progress,
        });
      } else if (event.event === "Finished") {
        setUpdateInfo({ status: "ready", version: upd.version });
      }
    });
    await relaunch();
  } catch (e) {
    console.error("[danbi] update install failed", e);
    setUpdateInfo({
      status: "error",
      version: upd.version,
      message: String(e),
    });
  }
}

export function dismissUpdatePill(): void {
  useApp.getState().setUpdateInfo(null);
}
