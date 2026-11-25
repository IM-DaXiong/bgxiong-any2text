import { useEffect, useState } from 'react';
import { WindowControls } from './uiChrome';
import { getCurrentWindow } from '@tauri-apps/api/window';

export function WindowTitleBar({ onError, onClose }: { onError: (message: string) => void; onClose: () => void }) {
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

  const perform = (command: 'minimize' | 'toggleMaximize') => {
    void getCurrentWindow()[command]().catch(error => onError(String(error)));
  };
  return <div className="window-titlebar" data-tauri-drag-region>
    <span data-tauri-drag-region>比格熊-any2text</span>
    <div className="window-controls" role="group" aria-label="窗口控制">
      <WindowControls max={maximized} onMinimize={() => perform('minimize')}
        onMaximize={() => perform('toggleMaximize')} onClose={onClose} />
    </div>
  </div>;
}
