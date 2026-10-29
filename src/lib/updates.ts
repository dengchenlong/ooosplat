import { invoke } from "@tauri-apps/api/core";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import type { UpdateCheckResult } from "../types/updates";

let inFlight: Promise<UpdateCheckResult> | null = null;

export async function checkAppUpdate(force = false): Promise<UpdateCheckResult | null> {
  if (!("__TAURI_INTERNALS__" in window)) return null;
  // StrictMode and repeated clicks must not create duplicate requests.
  if (!inFlight) {
    inFlight = invoke<UpdateCheckResult>("check_app_update", { force }).finally(() => { inFlight = null; });
  }
  return inFlight;
}

export async function copyUpdateText(text: string): Promise<void> {
  if ("__TAURI_INTERNALS__" in window) await writeText(text);
  else await navigator.clipboard.writeText(text);
}
