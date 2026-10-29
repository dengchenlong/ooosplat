// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { LanguageProvider } from "../i18n";
import type { EngineKind, EngineStatus } from "../types/pipeline";
import { EngineStatusPopover, enginePanelLayout } from "./EngineStatusPopover";

const engine = (kind: EngineKind, canStart = true): EngineStatus => ({
  kind, path: `E:\\engines\\${kind}.exe`, exists: true, canStart,
  version: null, cpuOnly: null, acceleration: null,
  detail: canStart ? "引擎可启动" : "帮助命令退出码：-1073741515\nMissing runtime dependency",
});
const allEngines = (ready = true) => (['ffmpeg', 'ffprobe', 'colmap', 'brush'] as const).map(kind => engine(kind, ready));
let container: HTMLDivElement;
let root: Root;
const trigger = () => container.querySelector<HTMLButtonElement>('.engine-summary')!;
const panel = () => container.querySelector<HTMLElement>('.engine-diagnostics-panel');
const advance = async (ms: number) => { await act(async () => vi.advanceTimersByTime(ms)); };
const hover = async (element: HTMLElement = trigger()) => {
  await act(async () => element.dispatchEvent(new MouseEvent('mouseover', { bubbles: true })));
};
const leave = async (element: HTMLElement = trigger()) => {
  await act(async () => element.dispatchEvent(new MouseEvent('mouseout', { bubbles: true })));
};
const render = async (engines = allEngines(), locale = 'zh-CN') => {
  localStorage.setItem('ooo-splat-language', locale);
  await act(async () => root.render(<LanguageProvider key={locale}><EngineStatusPopover engines={engines} /><button id="outside">outside</button></LanguageProvider>));
};

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  vi.useFakeTimers();
  container = document.createElement('div'); document.body.append(container); root = createRoot(container);
});
afterEach(async () => {
  await act(async () => root.unmount());
  container.remove(); localStorage.clear(); vi.restoreAllMocks(); vi.unstubAllGlobals(); vi.useRealTimers();
});

describe('engine status hover panel', () => {
  it.each([
    ['zh-CN', '引擎就绪', '4 个引擎异常', '正在检查引擎'],
    ['en', 'Engines ready', '4 engine issues', 'Checking engines'],
  ])('uses approved concise status text without engine names (%s)', async (locale, ready, issues, checking) => {
    await render(allEngines(), locale); expect(trigger().textContent).toBe(ready);
    expect(trigger().querySelector('svg')).toBeNull();
    expect(trigger().querySelector('.status-light')?.classList.contains('warning')).toBe(false);
    await render(allEngines(false), locale); expect(trigger().textContent).toBe(issues);
    expect(trigger().querySelector('.status-light')?.classList.contains('warning')).toBe(true);
    await render([], locale); expect(trigger().textContent).toBe(checking);
    expect(trigger().querySelector('.status-light')?.classList.contains('checking')).toBe(true);
    expect(trigger().textContent).not.toMatch(/ffmpeg|ffprobe|colmap|brush/i);
    await act(async () => trigger().focus()); expect(panel()?.textContent).toBe(checking);
    expect(trigger().querySelector('svg')).toBeNull();
  });

  it('waits 150ms to open and cancels a brief hover', async () => {
    await render(); await hover(); await advance(149); expect(panel()).toBeNull();
    await leave(); await advance(300); expect(panel()).toBeNull();
    await hover(); await advance(150); expect(panel()).not.toBeNull();
    expect(trigger().getAttribute('aria-controls')).toBe(panel()?.id);
    expect(panel()?.getAttribute('aria-labelledby')).toBe(trigger().id);
  });

  it('allows entry into the scrollable panel and closes 200ms after leaving', async () => {
    await render(); await hover(); await advance(150);
    await leave(); await advance(199); expect(panel()).not.toBeNull();
    await hover(panel()!); await advance(500); expect(panel()).not.toBeNull();
    await act(async () => panel()!.dispatchEvent(new Event('scroll', { bubbles: true })));
    expect(panel()).not.toBeNull();
    await leave(panel()!); await advance(199); expect(panel()).not.toBeNull();
    await advance(1); expect(panel()).toBeNull();
  });

  it('opens on focus, allows focus in the panel and closes on focus leaving the component', async () => {
    await render(); await act(async () => trigger().focus()); expect(panel()).not.toBeNull();
    await act(async () => panel()!.focus()); expect(panel()).not.toBeNull();
    await act(async () => container.querySelector<HTMLButtonElement>('#outside')!.focus());
    expect(panel()).toBeNull(); expect(trigger().getAttribute('aria-expanded')).toBe('false');
  });

  it('dismisses with Escape without reopening or losing keyboard focus', async () => {
    await render(); await hover(); await advance(150);
    await act(async () => panel()!.focus());
    await act(async () => document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })));
    await advance(500);
    expect(panel()).toBeNull(); expect(document.activeElement).toBe(trigger());
    await leave(); await hover(); await advance(150); expect(panel()).not.toBeNull();
  });

  it('cancels a pending opening on Escape and closes on outside pointer-down', async () => {
    await render(); await hover();
    await act(async () => document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })));
    await advance(300); expect(panel()).toBeNull();
    await leave(); await hover(); await advance(150);
    await act(async () => panel()!.dispatchEvent(new Event('pointerdown', { bubbles: true })));
    expect(panel()).not.toBeNull();
    await act(async () => container.querySelector('#outside')!.dispatchEvent(new Event('pointerdown', { bubbles: true })));
    await advance(300); expect(panel()).toBeNull();
  });

  it('allows a fresh hover after Escape removes the panel without a parent mouse-leave event', async () => {
    await render(); await hover(); await advance(150); await hover(panel()!);
    await act(async () => document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })));
    await advance(300); expect(panel()).toBeNull();
    // Unmounting a hovered absolute child need not dispatch mouseleave to its parent.
    await hover(); await advance(150); expect(panel()).not.toBeNull();
  });

  it('preserves full normalized local/UNC paths, error output and the existing readiness rule', async () => {
    const local = String.raw`\\?\E:\Projects\中文 项目` + `\\${'long-path'.repeat(30)}\\ffmpeg.exe`;
    const unc = String.raw`\\?\UNC\server\share\ffprobe.exe`;
    const input = allEngines(); input[0].path = local; input[1].path = unc;
    input[1].exists = false; input[1].canStart = false; input[1].detail = 'first line\n' + 'diagnostic output\n'.repeat(500);
    // canStart is the existing app-wide readiness contract, not a new GPU check.
    input[2].exists = false;
    await render(input); expect(trigger().textContent).toBe('1 个引擎异常');
    await act(async () => trigger().focus());
    expect(Array.from(panel()!.querySelectorAll('section strong')).map(node => node.textContent)).toEqual(['FFmpeg', 'FFprobe', 'COLMAP', 'Brush']);
    const paths = panel()!.querySelectorAll('.engine-item-path dd');
    expect(paths[0].textContent).toBe(local.slice(4));
    expect(paths[1].textContent).toBe(String.raw`\\server\share\ffprobe.exe`);
    expect(panel()!.querySelectorAll('.engine-item-detail')[1].textContent).toBe(input[1].detail);
    expect(panel()!.querySelectorAll('.engine-item-flags')[1].textContent).toBe('文件存在否可启动否');
    await render(allEngines(), 'en'); await act(async () => trigger().focus());
    expect(panel()?.textContent).toContain('Engine can start');
    expect(panel()?.textContent).toContain('File existsYesCan startYes');
  });

  it('updates data while open and translates failures without discarding technical evidence', async () => {
    await render(allEngines(), 'en'); await act(async () => trigger().focus());
    await render([engine('ffprobe', false)], 'en');
    expect(trigger().textContent).toBe('1 engine issues');
    expect(panel()?.textContent).toContain('Engine check exit code: -1073741515\nMissing runtime dependency');
  });

  it('cleans up pending timers and document listeners when unmounted', async () => {
    await render(); const remove = vi.spyOn(document, 'removeEventListener');
    const scheduled = vi.spyOn(globalThis, 'setTimeout');
    const cleared = vi.spyOn(globalThis, 'clearTimeout');
    await hover();
    const pending = scheduled.mock.results[scheduled.mock.calls.findIndex(call => call[1] === 150)].value;
    await act(async () => root.render(null));
    expect(cleared).toHaveBeenCalledWith(pending); await advance(500); expect(panel()).toBeNull();
    expect(remove.mock.calls.map(call => call[0])).toEqual(expect.arrayContaining(['pointerdown', 'keydown', 'visibilitychange']));
  });

  it('clamps the live panel when resized and observes the scaled interface only while open', async () => {
    const disconnect = vi.fn(); const observe = vi.fn();
    vi.stubGlobal('ResizeObserver', class { observe = observe; disconnect = disconnect; });
    await render(); const measure = vi.spyOn(trigger(), 'getBoundingClientRect').mockReturnValue({ right: 1000, bottom: 55 } as DOMRect);
    await act(async () => trigger().focus()); expect(panel()!.style.width).toBe('480px');
    expect(observe).toHaveBeenCalledWith(trigger());
    measure.mockReturnValue({ right: 360, bottom: 55 } as DOMRect);
    const width = vi.spyOn(window, 'innerWidth', 'get').mockReturnValue(375);
    await act(async () => window.dispatchEvent(new Event('resize')));
    expect(panel()!.style.width).toBe('351px');
    await act(async () => document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' })));
    expect(disconnect).toHaveBeenCalled(); width.mockRestore(); vi.unstubAllGlobals();
  });
});

describe('scaled engine panel geometry', () => {
  it.each([0.8, 1, 1.4])('keeps the panel on screen at scale %s', scale => {
    for (const viewport of [{ width: 1100, height: 680 }, { width: 375, height: 300 }]) {
      for (const anchor of [{ right: viewport.width - 28 * scale, bottom: 48 * scale }, { right: 90, bottom: 55 }]) {
        const layout = enginePanelLayout(anchor, viewport, scale);
        const width = Number(layout.width), height = Number(layout.maxHeight), right = Number(layout.right);
        const left = anchor.right - (right + width) * scale;
        expect(width).toBeLessThanOrEqual(480); expect(height).toBeLessThanOrEqual(520);
        expect(left).toBeGreaterThanOrEqual(12 - 1e-8);
        expect(left + width * scale).toBeLessThanOrEqual(viewport.width - 12 + 1e-8);
        expect(anchor.bottom + (8 + height) * scale).toBeLessThanOrEqual(viewport.height - 12 + 1e-8);
      }
    }
  });
});
