import { describe, expect, it } from "vitest";
import { previewAssetUrl } from "./previewAssetUrl";

describe("previewAssetUrl", () => {
  it("creates distinct URLs for repeated preview sessions", () => {
    const source = "http://asset.localhost/C%3A/Projects/final.ply";
    const first = previewAssetUrl(source, "previewSession", "first");
    const second = previewAssetUrl(source, "previewSession", "second");
    expect(first).not.toBe(second);
    expect(first).toContain("previewSession=first");
    expect(second).toContain("previewSession=second");
  });

  it("preserves the session while replacing the retry revision", () => {
    const source = "http://asset.localhost/final.ply?previewSession=session-a&retry=0";
    const retried = previewAssetUrl(source, "retry", "1");
    expect(retried).toContain("previewSession=session-a");
    expect(retried).toContain("retry=1");
    expect(retried).not.toContain("retry=0");
  });

  it("resolves an imported model identity without treating it as an absolute URL", () => {
    const resolved = previewAssetUrl("ooosplat-import-session.ply", "retry", "1");
    const url = new URL(resolved);
    expect(url.pathname).toMatch(/\/ooosplat-import-session\.ply$/);
    expect(url.searchParams.get("retry")).toBe("1");
  });
});
