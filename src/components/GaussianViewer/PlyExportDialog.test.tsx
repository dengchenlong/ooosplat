// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { LanguageProvider } from "../../i18n";
import { PlyExportDialog, type PlyExportDialogState } from "./PlyExportDialog";

describe("model export status (no second format selector)", () => {
  let root: Root, container: HTMLDivElement;
  beforeEach(() => {
    Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
    localStorage.setItem("ooo-splat-language", "zh-CN");
    container = document.createElement("div"); document.body.append(container); root = createRoot(container);
  });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); localStorage.clear(); });
  const props = () => ({ onClose: vi.fn(), onRetry: vi.fn(), onReveal: vi.fn(), onCancel: vi.fn(), onConfirmAdjustment: vi.fn() });
  const render = async (state: PlyExportDialogState, progress: number | null = 0, callbacks = props()) => {
    await act(async () => root.render(<LanguageProvider><PlyExportDialog state={state} progress={progress} format="spz" {...callbacks} /></LanguageProvider>)); return callbacks;
  };
  const button = (text: string) => Array.from(container.querySelectorAll("button")).find(b => b.textContent === text)!;
  it("shows selected format immediately with cancellable saving and live processing progress", async () => {
    const callbacks = await render({ phase: "preparing" });
    expect(container.textContent).toContain("正在保存编辑并准备导出"); expect(container.textContent).toContain("SPZ");
    expect(container.querySelector("[role=radiogroup]")).toBeNull();
    await act(async () => button("取消").click()); expect(callbacks.onCancel).toHaveBeenCalledOnce();
    await render({ phase: "encoding" }, 47.9);
    expect(container.textContent).toContain("47%"); expect(container.querySelector("progress")?.value).toBe(47.9);
  });
  it("keeps compressed encoding indeterminate and prevents cancellation during atomic publishing", async () => {
    await render({ phase: "encoding" }, null);
    expect(container.querySelector("progress")?.hasAttribute("value")).toBe(false);
    await render({ phase: "publishing" }); expect(button("取消").disabled).toBe(true);
    await render({ phase: "cancelling" }); expect(button("取消").disabled).toBe(true);
  });
  it("shows actual size and output path with folder and close actions", async () => {
    const callbacks = await render({ phase: "completed", path: "E:\\Exports\\edit.spz", fileSize: 1048576 });
    expect(container.textContent).toContain("模型导出完成"); expect(container.textContent).toContain("1 MiB");
    await act(async () => button("打开文件夹").click()); expect(callbacks.onReveal).toHaveBeenCalledWith("E:\\Exports\\edit.spz");
    await act(async () => button("关闭").click()); expect(callbacks.onClose).toHaveBeenCalledOnce();
  });
  it("reports errors with retry and explains SPZ range protection", async () => {
    const callbacks = await render({ phase: "error", error: "disk full" });
    expect(container.textContent).toContain("模型导出失败: disk full");
    await act(async () => button("重试导出").click()); expect(callbacks.onRetry).toHaveBeenCalledOnce();
    await render({ phase: "error", error: "Error: SPZ_POSITION_RANGE" }); expect(container.textContent).toContain("不会自动缩放或删点");
  });
  it("ignores Escape while active, traps focus and allows closing a terminal result", async () => {
    const callbacks = await render({ phase: "encoding" }, 5);
    const dialog = container.querySelector<HTMLElement>("[role=dialog]")!;
    await act(async () => dialog.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }))); expect(callbacks.onClose).not.toHaveBeenCalled();
    const cancel = button("取消"); cancel.focus();
    await act(async () => cancel.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true }))); expect(document.activeElement).toBe(cancel);
    await render({ phase: "completed", path: "edit.spz" }, 100, callbacks);
    await act(async () => dialog.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }))); expect(callbacks.onClose).toHaveBeenCalledOnce();
  });
  const confirmation = { phase: "awaitingConfirmation" as const, exportId: "export-1", confirmationId: 9, adjustment: { center: [10000, 0, 0] as [number, number, number], scale: 0.5 } };
  it("uses the approved warning copy, focuses Cancel, pauses progress and confirms the exact request", async () => {
    const callbacks = await render(confirmation, 45);
    expect(container.textContent).toContain("是否自动调整模型坐标");
    expect(container.textContent).toContain("当前模型坐标超出 SPZ 可表示的范围。");
    expect(container.textContent).toContain("必要时再等比例缩小模型，仅影响本次导出，不修改原项目。");
    expect(container.textContent).toContain("手动清理远处的漂浮点、裁切或调整模型");
    expect(container.querySelector("progress")).toBeNull(); expect(container.querySelector(".spin")).toBeNull();
    expect(document.activeElement).toBe(button("取消"));
    expect(container.querySelectorAll("[role=dialog]")).toHaveLength(1);
    expect(callbacks.onConfirmAdjustment).not.toHaveBeenCalled();
    await act(async () => button("自动调整并导出").click());
    expect(callbacks.onConfirmAdjustment).toHaveBeenCalledExactlyOnceWith("export-1", 9);
  });
  it("cancels a waiting confirmation with Escape and keeps keyboard focus in the same modal", async () => {
    const callbacks = await render(confirmation);
    const cancel = button("取消"), confirm = button("自动调整并导出");
    confirm.focus();
    await act(async () => confirm.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true })));
    expect(document.activeElement).toBe(cancel);
    await act(async () => cancel.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
    expect(callbacks.onCancel).toHaveBeenCalledOnce(); expect(callbacks.onConfirmAdjustment).not.toHaveBeenCalled(); expect(callbacks.onClose).not.toHaveBeenCalled();
  });
  it("shows approved adjustment result labels and safe-size failure guidance", async () => {
    await render({ phase: "completed", path: "edit.spz", coordinateAdjustment: confirmation.adjustment });
    expect(container.textContent).toContain("坐标调整"); expect(container.textContent).toContain("居中参考点");
    expect(container.textContent).toContain("c = [10000, 0, 0]"); expect(container.textContent).toContain("缩放比例: s = 0.5");
    await render({ phase: "error", error: "Error: SPZ_SCALE_RANGE" });
    expect(container.textContent).toContain("自动调整后，高斯点尺寸超出 SPZ 可表示的范围，已停止导出。");
    expect(container.textContent).toContain("请手动调整模型，或改用标准 PLY、压缩 PLY、SOG。");
  });
  it("uses the approved corresponding English confirmation copy", async () => {
    localStorage.setItem("ooo-splat-language", "en");
    await render(confirmation);
    expect(container.textContent).toContain("Automatically adjust model coordinates?");
    expect(container.textContent).toContain("This affects only this export and does not modify the original project.");
    expect(container.textContent).toContain("To preserve the original coordinates and dimensions");
    expect(button("Cancel")).toBe(document.activeElement); expect(button("Adjust and export")).toBeDefined();
  });
});
