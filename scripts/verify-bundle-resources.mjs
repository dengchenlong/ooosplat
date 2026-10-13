import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const workspace = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const configDirectory = path.join(workspace, "src-tauri");

function readJson(relative) {
  return JSON.parse(fs.readFileSync(path.join(workspace, relative), "utf8"));
}

const common = readJson("src-tauri/tauri.conf.json");
const viewerSource = "../.cache/html-viewer/runtime.js";
if (common.bundle?.resources?.[viewerSource] !== "html-viewer/runtime.js") throw new Error("The shared offline HTML viewer resource must be bundled on every platform.");
if (!fs.existsSync(path.resolve(configDirectory, viewerSource))) throw new Error("Offline HTML viewer not built. Run npm run build:html-viewer first.");

const expected = new Map([
  ["tauri.windows.conf.json", ["../engines/colmap/", "engines/colmap/"]],
  ["tauri.linux.conf.json", ["../engines/linux/colmap/", "engines/linux/colmap/"]],
  ["tauri.macos.conf.json", ["../engines/macos/arm64/colmap/", "engines/macos/arm64/colmap/"]],
]);
for (const [configName, [source, destination]] of expected) {
  const resources = readJson(`src-tauri/${configName}`).bundle?.resources;
  if (!resources || Array.isArray(resources) || resources[source] !== destination) throw new Error(`${configName} must bundle the complete locked COLMAP runtime directory.`);
}

const runtimeLock = readJson("engines/colmap-runtime.json");
if (runtimeLock.offlineVocabulary?.path !== "lib/colmap/vocab_tree_faiss_flickr100K_words256K.bin"
  || !/^[a-f0-9]{64}$/i.test(runtimeLock.offlineVocabulary?.sha256 ?? "")) throw new Error("The offline COLMAP vocabulary must remain hash-locked and bundled.");

// Loop detection is deliberately enabled for optimized video in every tier.
// Rust tests enforce the saved optimization toggle, explicit CLI flags, and
// unchanged exhaustive/Bridge matching. No resource download is introduced.
const optimization = readJson("config/pipeline-optimization.json");
for (const quality of ["fast", "balanced", "high"]) {
  if (optimization.qualities?.[quality]?.automaticOptimizationOn?.loopDetection !== true) {
    throw new Error(`${quality} must enable optimized-video loop detection by default.`);
  }
}

console.log("Verified complete COLMAP runtimes, retained offline vocabulary, and loop-detection defaults for all three tiers.");
