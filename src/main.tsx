import { createRoot } from "react-dom/client";
import { App } from "./app/App";
import { LanguageProvider } from "./i18n";
import "./styles.css";

if ("__TAURI_INTERNALS__" in window) {
  void import("./lib/mcpExportExecutor").then(async ({ startMcpExportExecutor }) => {
    const dispose = await startMcpExportExecutor();
    import.meta.hot?.dispose(() => { void dispose(); });
  }).catch(error => console.warn("MCP export executor unavailable", error));
}

createRoot(document.getElementById("root")!).render(<LanguageProvider><App /></LanguageProvider>);
