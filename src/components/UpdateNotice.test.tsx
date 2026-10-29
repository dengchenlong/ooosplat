// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { LanguageProvider } from "../i18n";
import type { UpdateRelease } from "../types/updates";
import { UpdateNoticeDialog } from "./UpdateNotice";
const mocks = vi.hoisted(() => ({ copy: vi.fn() }));
vi.mock("../lib/updates", () => ({ copyUpdateText: mocks.copy }));
const release: UpdateRelease = {
  version: "0.6.1", publishedAt: "2026-10-11T08:00:00Z",
  notes: { "zh-CN": ["更新提醒", "<script>unsafe()</script>"], en: ["New version notifications"] },
  downloads: { github: "https://github.com/ooolabdev/ooosplat/releases/tag/0.6.1", quark: "https://pan.quark.cn/s/abc123", quarkCode: "1234" },
};

describe("UpdateNoticeDialog", () => {
  let root: Root;
  let container: HTMLDivElement;
  const onClose = vi.fn();
  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    localStorage.setItem("ooo-splat-language", "zh-CN");
    mocks.copy.mockReset().mockResolvedValue(undefined);
    onClose.mockClear();
    container = document.createElement("div"); document.body.appendChild(container); root = createRoot(container);
  });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); localStorage.clear(); });
  const render = async (value = release) => { await act(async () => root.render(<LanguageProvider><UpdateNoticeDialog currentVersion="0.6.0" release={value} onClose={onClose} /></LanguageProvider>)); };
  const button = (label: string) => Array.from(container.querySelectorAll<HTMLButtonElement>("button")).find(item => item.textContent === label)!;

  it("renders plain-text notes and copies each exact link or access code with honest feedback", async () => {
    await render();
    expect(container.textContent).toContain("0.6.0");
    expect(container.textContent).toContain("<script>unsafe()</script>");
    expect(container.querySelector("script, a")).toBeNull();
    await act(async () => button("复制 GitHub 链接").click());
    expect(mocks.copy).toHaveBeenLastCalledWith(release.downloads.github);
    expect(container.querySelector('[role="status"]')?.textContent).toBe("已复制");
    await act(async () => button("复制提取码").click());
    expect(mocks.copy).toHaveBeenLastCalledWith("1234");
    mocks.copy.mockRejectedValueOnce(new Error("denied"));
    await act(async () => button("复制夸克链接").click());
    expect(mocks.copy).toHaveBeenLastCalledWith(release.downloads.quark);
    expect(container.querySelector('[role="status"]')?.textContent).toContain("无法复制");
  });

  it("uses English and hides unconfigured Quark controls", async () => {
    localStorage.setItem("ooo-splat-language", "en");
    await render({ ...release, downloads: { ...release.downloads, quark: null, quarkCode: null } });
    expect(container.textContent).toContain("New version notifications");
    expect(container.textContent).toContain("Copy GitHub link");
    expect(container.textContent).not.toContain("Copy Quark link");
    expect(container.textContent).not.toContain("Access code");
    await act(async () => button("Ignore").click());
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("contains keyboard focus and treats Escape and close as dismissal", async () => {
    await render();
    const dialog = container.querySelector<HTMLElement>('[role="dialog"]')!;
    const buttons = dialog.querySelectorAll<HTMLButtonElement>("button");
    expect(document.activeElement).toBe(dialog);
    await act(async () => dialog.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", shiftKey: true, bubbles: true })));
    expect(document.activeElement).toBe(buttons[buttons.length - 1]);
    await act(async () => buttons[buttons.length - 1].dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true })));
    expect(document.activeElement).toBe(buttons[0]);
    await act(async () => window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" })));
    await act(async () => buttons[0].click());
    expect(onClose).toHaveBeenCalledTimes(2);
  });
});
