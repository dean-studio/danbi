import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { CaptureApp } from "./capture/CaptureApp";
import { PopoverApp } from "./popover/PopoverApp";
import { ErrorBoundary } from "./components/ErrorBoundary";
import "./index.css";

const hash =
  typeof window !== "undefined" ? window.location.hash : "";
const isCapture = hash === "#capture";
const isPopover = hash === "#popover";

if ((isCapture || isPopover) && typeof document !== "undefined") {
  document.documentElement.dataset.window = isPopover ? "popover" : "capture";
  // index.html sets an opaque background inline to prevent a startup flash in
  // the main window. For the transparent capture/popover windows we must undo
  // those higher-specificity styles before the first paint.
  document.documentElement.style.background = "transparent";
  document.body.style.background = "transparent";
  const rootEl = document.getElementById("root");
  if (rootEl) rootEl.style.background = "transparent";
}

const scope = isCapture ? "capture" : isPopover ? "popover" : "main";
const Root = isCapture ? <CaptureApp /> : isPopover ? <PopoverApp /> : <App />;

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <ErrorBoundary scope={scope}>{Root}</ErrorBoundary>
  </React.StrictMode>,
);
