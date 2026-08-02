import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./styles/globals.css";
import "./styles/redesign-tokens-v2.css";
import "./styles/redesign-components.css";
import "@fontsource/inter-tight/400.css";
import "@fontsource/inter-tight/500.css";
import "@fontsource/inter-tight/600.css";
import "@fontsource/jetbrains-mono/400.css";
import "@fontsource/jetbrains-mono/500.css";
import "@fontsource/jetbrains-mono/600.css";
import "./i18n";

// Suppress the webview's native context menu everywhere the app doesn't
// provide one of its own. Only four places do (asset rows, gallery cards,
// the asset-list container, the tag panel header), so right-clicking the
// sidebar / header / issue list / stats / preview / any empty space used to
// pop WebView2's "Reload / Print" menu — release builds included — which
// reads as an unfinished application. Components with their own
// `onContextMenu` call `preventDefault` and open their menu while the event
// bubbles through React's root container, well before this window-level
// listener runs, so they are unaffected.
//
// Text fields are exempt: their native menu carries copy / paste / spelling,
// which the app does not reimplement.
//
// Deliberately not exempted in dev. A behaviour that exists only in dev is a
// behaviour dev can never test — this codebase already carries that trap in
// the other direction (`tauri dev` applies no CSP, so CSP regressions pass
// dev unnoticed), and the right-click "Inspect Element" entry is not worth
// buying it a second time. The inspector is reachable via F12 / Cmd+Opt+I
// instead — see `toggle_devtools` in `src-tauri/src/lib.rs`.
const NATIVE_MENU_SELECTOR = 'input, textarea, [contenteditable]:not([contenteditable="false"])';

window.addEventListener("contextmenu", (event) => {
  const target = event.target;
  if (target instanceof Element && target.closest(NATIVE_MENU_SELECTOR)) return;
  event.preventDefault();
});

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
);
