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
import { installAppMenu } from "./lib/appMenu";

// Suppress the webview's native context menu everywhere the app does not provide
// one of its own. Text fields are exempt, since their native menu carries copy,
// paste and spelling. Dev builds are not exempt.
const NATIVE_MENU_SELECTOR = 'input, textarea, [contenteditable]:not([contenteditable="false"])';

window.addEventListener("contextmenu", (event) => {
  const target = event.target;
  if (target instanceof Element && target.closest(NATIVE_MENU_SELECTOR)) return;
  event.preventDefault();
});

// macOS menu bar (no-op elsewhere). Installed outside React: the menu is
// app-global, and a React-effect install would double-fire under strict mode's
// mount/unmount cycle.
installAppMenu();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
);
