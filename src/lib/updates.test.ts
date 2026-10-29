// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
const mocks = vi.hoisted(() => ({ invoke: vi.fn(), writeText: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({ writeText: mocks.writeText }));
import { checkAppUpdate, copyUpdateText } from "./updates";

describe("notification-only native calls", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    delete (window as typeof window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
  });
  afterEach(() => { delete (window as typeof window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__; });

  it("does not perform an update request in a browser", async () => {
    await expect(checkAppUpdate()).resolves.toBeNull();
    expect(mocks.invoke).not.toHaveBeenCalled();
  });

  it("shares in-flight requests and passes only a force flag", async () => {
    (window as typeof window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {};
    let resolve!: (value: unknown) => void;
    mocks.invoke.mockReturnValueOnce(new Promise(done => { resolve = done; }));
    const first = checkAppUpdate();
    const second = checkAppUpdate();
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
    expect(mocks.invoke).toHaveBeenCalledWith("check_app_update", { force: false });
    resolve({ status: "upToDate" });
    await Promise.all([first, second]);
    mocks.invoke.mockResolvedValueOnce({ status: "upToDate" });
    await checkAppUpdate(true);
    expect(mocks.invoke).toHaveBeenLastCalledWith("check_app_update", { force: true });
  });

  it("writes only the requested text and propagates copy failures", async () => {
    (window as typeof window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {};
    mocks.writeText.mockResolvedValueOnce(undefined);
    await copyUpdateText("https://github.com/ooolabdev/ooosplat/releases/tag/0.6.1");
    expect(mocks.writeText).toHaveBeenCalledWith("https://github.com/ooolabdev/ooosplat/releases/tag/0.6.1");
    expect(mocks.invoke).not.toHaveBeenCalled();
    mocks.writeText.mockRejectedValueOnce(new Error("clipboard unavailable"));
    await expect(copyUpdateText("1234")).rejects.toThrow("clipboard unavailable");
  });
});
