// @vitest-environment jsdom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { LanguageProvider } from '../i18n';
import { MediaSourceStatus } from './MediaSourceStatus';
import { TaskConfiguration } from './TaskConfiguration';
import type { ComponentProps } from 'react';
let container: HTMLDivElement, root: Root;
const detection = { sourceIdentity: 'source', confirmedErp: false, requiresChoice: true, supportedErp: true, unsupported: false, reason: '', previewJpeg: [] };
beforeEach(() => { Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true }); localStorage.setItem('ooo-splat-language', 'zh-CN'); container = document.createElement('div'); document.body.append(container); root = createRoot(container); });
afterEach(async () => { await act(async () => root.unmount()); container.remove(); });
async function render(props: ComponentProps<typeof MediaSourceStatus>) { await act(async () => root.render(<LanguageProvider key={localStorage.getItem('ooo-splat-language')}><MediaSourceStatus {...props} /></LanguageProvider>)); }

it('shows loading instead of an old projection result and offers retry only for analysis errors', async () => {
  await render({ inputType: 'video', checking: true, projection: 'equirectangular', detection });
  expect(container.textContent).toBe('正在检测视频类型…');
  expect(container.querySelector('.spin')).not.toBeNull();
  const retry = vi.fn();
  await render({ inputType: 'video', error: '文件不存在', onRetry: retry });
  expect(container.textContent).toContain('素材检测失败');
  expect(container.textContent).not.toContain('普通视频');
  await act(async () => container.querySelector<HTMLButtonElement>('button')!.click());
  expect(retry).toHaveBeenCalledOnce();
});

it('requires an explicit inline choice, keeps it editable, and never renders a preview', async () => {
  const change = vi.fn();
  await render({ inputType: 'video', projection: 'auto', detection, onProjectionChange: change });
  const select = container.querySelector<HTMLSelectElement>('select')!;
  expect(select.value).toBe('');
  expect(select.getAttribute('aria-label')).toBe('请选择视频类型');
  expect(container.querySelector('.media-source-copy > p')?.textContent).toBe('无法判断视频类型。当前不支持双鱼眼、双目、VR180、裁切全景和厂商私有原片。');
  expect(select.querySelector('option[value=equirectangular]')?.textContent).toBe('全景视频');
  expect(container.querySelector('.media-source-type-icon')).toBeNull();
  await act(async () => { select.value = 'equirectangular'; select.dispatchEvent(new Event('change', { bubbles: true })); });
  expect(change).toHaveBeenCalledWith('equirectangular');
  await render({ inputType: 'video', projection: 'equirectangular', detection, onProjectionChange: change });
  expect(container.textContent).toContain('已选择完整全景视频');
  expect(container.textContent).not.toContain('已识别完整 ERP 元数据');
  expect(container.querySelector('select')).not.toBeNull();
  expect(container.querySelector('img,details')).toBeNull();
});

it('shows the metadata explanation only for confirmed ERP and disallows unsupported panorama', async () => {
  await render({ inputType: 'video', projection: 'equirectangular', detection: { ...detection, confirmedErp: true, requiresChoice: false } });
  expect(container.textContent).toContain('已识别完整 ERP 元数据，将自动按全景视频生成');
  expect(container.querySelector('select')).toBeNull();
  await render({ inputType: 'video', projection: 'auto', detection: { ...detection, supportedErp: false, unsupported: true } });
  expect(container.querySelector<HTMLOptionElement>('option[value=equirectangular]')!.disabled).toBe(true);
  expect(container.textContent).toContain('不支持此素材');
});

it.each([
  ['equirectangular', '.lucide-globe'],
  ['perspective', '.lucide-tv-minimal-play'],
] as const)('shows a left-side type icon for %s, including readonly history', async (projection, icon) => {
  for (const historical of [false, true]) {
    await render({ inputType: 'video', projection, historical });
    const typeIcon = container.querySelector(`.has-type-icon .media-source-type-icon${icon}`);
    expect(typeIcon).not.toBeNull();
    expect(typeIcon?.getAttribute('aria-hidden')).toBe('true');
  }
  await render({ inputType: 'images' });
  expect(container.querySelector('.media-source-type-icon')).toBeNull();
});

it.each([true, false])('uses the same readonly saved-type explanation with completed=%s', async completed => {
  const props = { sourcePath: 'E:/old.mp4', projectsRoot: 'E:/projects', inputType: 'video' as const, quality: 'fast' as const, plannerEnabled: false, completed, acceleration: null };
  await act(async () => root.render(<LanguageProvider><TaskConfiguration {...props} videoProjection="equirectangular" /></LanguageProvider>));
  expect(container.querySelector('.source-input-card.readonly')?.textContent).toContain('本任务按全景视频处理');
  expect(container.querySelector('select')).toBeNull();
  expect(container.querySelector('.source-input-card .media-path svg')).toBeNull();
  expect(container.querySelector('.source-input-card .media-path strong')?.textContent).toBe('old.mp4');
  await act(async () => root.render(<LanguageProvider><TaskConfiguration {...props} videoProjection="perspective" /></LanguageProvider>));
  expect(container.querySelector('.media-source-status')?.textContent).toContain('本任务按普通视频处理');
});

it('does not guess a historical type from dimensions and distinguishes image sequences', async () => {
  await render({ inputType: 'video', historical: true, projection: null });
  expect(container.textContent).toContain('未记录视频类型');
  await render({ inputType: 'images', historical: true });
  expect(container.textContent).toContain('本任务按普通图片处理');
  await render({ inputType: 'images' });
  expect(container.textContent).toContain('将按普通图片处理');
});

it('renders the corresponding English copy', async () => {
  localStorage.setItem('ooo-splat-language', 'en');
  await render({ inputType: 'video', projection: 'perspective' });
  expect(container.textContent).toContain('This video will use ordinary-video processing.');
  await render({ inputType: 'video', projection: 'auto', detection });
  expect(container.querySelector('.media-source-copy > p')?.textContent).toContain('Unable to determine the video type.');
  expect(container.querySelector('option[value=equirectangular]')?.textContent).toBe('Panorama video');
});

it('uses only approved image choices, differentiates confirmed and manual ERP, and blocks unsupported ERP', async () => {
  const change=vi.fn();
  await render({inputType:'images',projection:'auto',detection,onProjectionChange:change});
  const select=container.querySelector<HTMLSelectElement>('select')!;
  expect(select.value).toBe('');expect(select.getAttribute('aria-label')).toBe('请选择图片类型');
  expect(container.textContent).toContain('无法判断图片类型。请选择普通图片或全景图片。');
  expect([...select.options].map(o=>o.textContent)).toEqual(['请选择','普通图片','全景图片']);
  await act(async()=>{select.value='equirectangular';select.dispatchEvent(new Event('change',{bubbles:true}));});
  expect(change).toHaveBeenCalledWith('equirectangular');
  await render({inputType:'images',projection:'equirectangular',detection});
  expect(container.textContent).toContain('已选择全景图片，将按全景图片生成');expect(container.querySelector('select')).not.toBeNull();
  await render({inputType:'images',projection:'equirectangular',detection:{...detection,confirmedErp:true,requiresChoice:false}});
  expect(container.textContent).toContain('已识别完整 ERP 元数据，将自动按全景图片生成');expect(container.querySelector('select')).toBeNull();
  await render({inputType:'images',projection:'auto',detection:{...detection,unsupported:true,supportedErp:false}});
  expect(container.querySelector<HTMLOptionElement>('option[value=equirectangular]')!.disabled).toBe(true);expect(container.textContent).toContain('不支持此文件夹的全景格式。');
});

it('displays saved panorama image type in readonly history and approved English copy',async()=>{
  await render({inputType:'images',projection:'equirectangular',historical:true});expect(container.textContent).toContain('本任务按全景图片处理');expect(container.querySelector('select')).toBeNull();
  localStorage.setItem('ooo-splat-language','en');
  await render({inputType:'images',projection:'equirectangular',detection});expect(container.textContent).toContain('Panorama images selected. These images will use panorama generation.');
});
