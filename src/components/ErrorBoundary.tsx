import React from "react";

/**
 * 앱 전역 최후 방어막.
 *
 * 지금까지 앱에는 ErrorBoundary 가 없어서, 렌더 도중 어디서든 throw 가
 * 한 번 나면 React 가 트리 전체를 unmount → index.html 의 검은 배경
 * (`#07080a`) 만 남았다. "로고 → 검은 화면" 증상의 정체가 이것이다.
 *
 * 이 컴포넌트는:
 *   1. 카드 하나가 터져도 앱 전체가 죽지 않게 하고
 *   2. 삼켜지던 에러를 읽을 수 있는 패널로 띄워 (사용자가 스샷/복사로
 *      보내줄 수 있게) 실제 원인 특정을 가능하게 한다.
 *
 * 의도적으로 어떤 IPC·스토어·테마 유틸에도 의존하지 않는다 — 그것들
 * 자체가 throw 원인일 수 있으므로 순수 인라인 스타일로만 그린다.
 */
type Props = {
  children: React.ReactNode;
  /** 어느 윈도우/영역에서 터졌는지 라벨 (예: "main", "popover"). */
  scope?: string;
  /** 커스텀 fallback. 지정하면 전체화면 에러 패널 대신 이걸 그린다.
   *  Home 의 개별 카드처럼 "이 조각만 죽고 나머지는 살아야" 하는
   *  곳에 작은 카드형 안내를 넣는 용도. */
  fallback?: (error: Error, reset: () => void) => React.ReactNode;
};

type State = {
  error: Error | null;
  info: React.ErrorInfo | null;
};

export class ErrorBoundary extends React.Component<Props, State> {
  state: State = { error: null, info: null };

  static getDerivedStateFromError(error: Error): Partial<State> {
    return { error };
  }

  componentDidCatch(error: Error, info: React.ErrorInfo) {
    // 콘솔에도 남겨 devtools / 로그 파일에서 확인 가능하게.
    console.error(
      `[danbi] ErrorBoundary(${this.props.scope ?? "app"}) caught:`,
      error,
      info,
    );
    this.setState({ info });
  }

  private reset = () => {
    this.setState({ error: null, info: null });
  };

  private reload = () => {
    if (typeof window !== "undefined") window.location.reload();
  };

  private copy = () => {
    const { error, info } = this.state;
    const text = [
      `scope: ${this.props.scope ?? "app"}`,
      `message: ${error?.message ?? "(none)"}`,
      "",
      "stack:",
      error?.stack ?? "(no stack)",
      "",
      "component stack:",
      info?.componentStack ?? "(none)",
    ].join("\n");
    if (typeof navigator !== "undefined" && navigator.clipboard) {
      navigator.clipboard.writeText(text).catch(() => {});
    }
  };

  render() {
    const { error, info } = this.state;
    if (!error) return this.props.children;

    if (this.props.fallback) return this.props.fallback(error, this.reset);

    return (
      <div
        style={{
          position: "fixed",
          inset: 0,
          overflow: "auto",
          background: "#07080a",
          color: "#f4f4f6",
          fontFamily:
            'Inter, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif',
          fontFeatureSettings: '"calt", "kern", "liga", "ss03"',
          padding: "40px 32px",
          WebkitUserSelect: "text",
          userSelect: "text",
        }}
      >
        <div style={{ margin: "0 auto", maxWidth: 720 }}>
          <div
            style={{
              display: "flex",
              alignItems: "center",
              gap: 10,
              marginBottom: 14,
            }}
          >
            <span style={{ fontSize: 22 }}>💧</span>
            <h1
              style={{
                margin: 0,
                fontSize: 18,
                fontWeight: 600,
                letterSpacing: "0.2px",
              }}
            >
              단비에서 화면을 그리다 문제가 생겼어요
            </h1>
          </div>

          <p
            style={{
              margin: "0 0 20px",
              fontSize: 13,
              lineHeight: 1.6,
              color: "#9b9ea6",
            }}
          >
            아래 오류 내용을 복사해서 알려주시면 원인을 빠르게 잡을 수
            있어요. 데이터는 안전합니다 — vault 파일은 그대로예요.
          </p>

          <div
            style={{
              display: "flex",
              gap: 8,
              marginBottom: 20,
              flexWrap: "wrap",
            }}
          >
            <button
              type="button"
              onClick={this.reload}
              style={btnPrimary}
            >
              앱 다시 불러오기
            </button>
            <button type="button" onClick={this.reset} style={btnGhost}>
              이 화면 닫고 계속
            </button>
            <button type="button" onClick={this.copy} style={btnGhost}>
              오류 복사
            </button>
          </div>

          <pre style={preBox}>
            {error.message}
            {"\n\n"}
            {error.stack}
            {info?.componentStack ? "\n\n--- component stack ---" : ""}
            {info?.componentStack ?? ""}
          </pre>
        </div>
      </div>
    );
  }
}

const btnPrimary: React.CSSProperties = {
  height: 34,
  padding: "0 16px",
  borderRadius: 8,
  border: "none",
  background: "#ffffff",
  color: "#07080a",
  fontSize: 13,
  fontWeight: 600,
  cursor: "pointer",
};

const btnGhost: React.CSSProperties = {
  height: 34,
  padding: "0 14px",
  borderRadius: 8,
  border: "1px solid #242728",
  background: "#131516",
  color: "#e6e7ea",
  fontSize: 13,
  cursor: "pointer",
};

const preBox: React.CSSProperties = {
  margin: 0,
  padding: 16,
  borderRadius: 10,
  border: "1px solid #242728",
  background: "#0d0f10",
  color: "#c7c9cf",
  fontFamily:
    'ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, monospace',
  fontSize: 12,
  lineHeight: 1.55,
  whiteSpace: "pre-wrap",
  wordBreak: "break-word",
};
