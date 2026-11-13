import { useEffect, useState } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';

export function WindowTitleBar({ onError }: { onError: (message: string) => void }) {
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    const window = getCurrentWindow();
    let active = true;
    const update = () => { void window.isMaximized().then(value => {
      if (active) setMaximized(value);
    }).catch(() => {}); };
    update();
    const listener = window.onResized(update);
    return () => { active = false; void listener.then(unlisten => unlisten()).catch(() => {}); };
  }, []);

  const perform = (command: 'minimize' | 'toggleMaximize' | 'close') => {
    void getCurrentWindow()[command]().catch(error => onError(String(error)));
  };
  const resizeLabel = maximized ? '还原窗口' : '最大化窗口';

  return <div className="window-titlebar" data-tauri-drag-region>
    <span data-tauri-drag-region>比格熊-any2text</span>
    <div className="window-controls" role="group" aria-label="窗口控制">
      <button type="button" aria-label="最小化窗口" title="最小化窗口" onClick={() => perform('minimize')}><i className="window-minimize" /></button>
      <button type="button" aria-label={resizeLabel} title={resizeLabel} onClick={() => perform('toggleMaximize')}><i className={maximized ? 'window-restore' : 'window-maximize'} /></button>
      <button type="button" className="window-close-button" aria-label="关闭窗口" title="关闭窗口" onClick={() => perform('close')}><i className="window-close" /></button>
    </div>
  </div>;
}
