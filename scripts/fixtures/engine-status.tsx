// Browser fixture using the real application/components, without native IPC.
import { createRoot } from 'react-dom/client';
import { App } from '../../src/app/App';
import { LanguageProvider } from '../../src/i18n';
import { useAppStore } from '../../src/stores/appStore';
import type { EngineStatus } from '../../src/types/pipeline';
import '../../src/styles.css';

const params = new URLSearchParams(location.search);
localStorage.setItem('ooo-splat-language', params.get('locale') === 'en' ? 'en' : 'zh-CN');
localStorage.setItem('ooo-splat-ui-scale', params.get('scale') || '100');

Object.assign(window, {
  setEngineStatusFixture(state: 'ready' | 'issues' | 'checking') {
    const kinds = ['ffmpeg', 'ffprobe', 'colmap', 'brush'] as const;
    const engines: EngineStatus[] = state === 'checking' ? [] : kinds.map(kind => ({
      kind,
      path: kind === 'ffprobe'
        ? String.raw`\\?\UNC\server\share\引擎目录\ffprobe.exe`
        : String.raw`\\?\E:\Project\OOOSplat\中文 长路径 ${'engine-runtime-'.repeat(4)}` + `\\${kind}\\${kind}.exe`,
      exists: state !== 'issues' || kind !== 'ffprobe',
      canStart: state === 'ready', version: null, cpuOnly: null, acceleration: null,
      detail: state === 'issues'
        ? '帮助命令退出码：-1073741515\nMissing runtime dependency\n' + 'Technical diagnostic output retained.\n'.repeat(3)
        : kind === 'colmap'
          ? 'COLMAP 4.x CLI 已启用 NVIDIA GeForce RTX 3060 Ti；BA: Caspar GPU 0: execution probe passed'
          : kind === 'brush'
            ? 'OOOBrush 无界面 CLI 可启动；GPU 训练将在任务中验证'
            : '引擎可启动',
    }));
    useAppStore.setState({ engines });
  },
});

createRoot(document.getElementById('root')!).render(<LanguageProvider><App /></LanguageProvider>);
