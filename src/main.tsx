import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { PopoverApp } from "./popover/PopoverApp";
import { ErrorBoundary } from "./components/ErrorBoundary";
import "./index.css";

const hash =
  typeof window !== "undefined" ? window.location.hash : "";
const isPopover = hash === "#popover";

if (isPopover && typeof document !== "undefined") {
  document.documentElement.dataset.window = "popover";
  // index.html sets an opaque background inline to prevent a startup flash in
  // the main window. For the transparent popover window we must undo
  // those higher-specificity styles before the first paint.
  document.documentElement.style.background = "transparent";
  document.body.style.background = "transparent";
  const rootEl = document.getElementById("root");
  if (rootEl) rootEl.style.background = "transparent";
}

const scope = isPopover ? "popover" : "main";
const Root = isPopover ? <PopoverApp /> : <App />;

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <ErrorBoundary scope={scope}>{Root}</ErrorBoundary>
  </React.StrictMode>,
);
