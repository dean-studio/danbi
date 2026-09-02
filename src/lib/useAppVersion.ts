import { useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";

/**
 * 앱의 실제 번들 버전을 런타임에 읽어온다. `tauri.conf.json` 의 `version`
 * 이 단일 진실의 원천 — About 다이얼로그·설정 정보 패널에서 이 훅을 써서
 * 버전 문자열을 하드코딩하지 않는다 (그동안 0.3.0 으로 박혀 실제 릴리즈와
 * 어긋나던 문제 해결).
 */
export function useAppVersion(): string | null {
  const [version, setVersion] = useState<string | null>(null);
  useEffect(() => {
    let alive = true;
    getVersion()
      .then((v) => {
        if (alive) setVersion(v);
      })
      .catch(() => {
        /* dev 웹 프리뷰 등 Tauri 컨텍스트 밖이면 조용히 무시 */
      });
    return () => {
      alive = false;
    };
  }, []);
  return version;
}
