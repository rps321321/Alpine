import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { previewDesktopClient, tauriDesktopClient } from "./desktop";
import "./styles.css";

const isTauri = "__TAURI_INTERNALS__" in window;

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App desktop={isTauri ? tauriDesktopClient : previewDesktopClient} />
  </StrictMode>,
);
