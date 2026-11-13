import { useEffect, useLayoutEffect, useState } from 'react';
import { api, type Capabilities } from './api/any2text';
import themes from './bigbear-theme.json';

type ThemeId = string;
export const themeIds = themes.map(theme => theme.id);
export const themeMeta = Object.fromEntries(themes.map(theme => [theme.id, theme]));
export const isThemeId = (value: unknown): value is ThemeId => typeof value === 'string' && themeIds.includes(value);

function saved(key: string) {
  try { return localStorage.getItem(key); } catch { return null; }
}

export function useAppearance(connection: Capabilities | undefined) {
  const [appearance, setAppearance] = useState(() => {
    const preference = saved('any2text.appearance');
    return isThemeId(preference) ? preference : 'follow';
  });
  const [hostTheme, setHostTheme] = useState(() => {
    const previous = saved('any2text.theme');
    return isThemeId(previous) ? previous : 'dark';
  });
  const theme = appearance === 'follow' ? hostTheme : appearance;

  useLayoutEffect(() => {
    document.documentElement.dataset.theme = theme;
    try {
      localStorage.setItem('any2text.appearance', appearance);
      localStorage.setItem('any2text.theme', theme);
    } catch { /* The current window can still change appearance. */ }
  }, [appearance, theme]);

  useEffect(() => {
    if (!connection || appearance !== 'follow') return;
    let active = true;
    let pending = false;
    const refresh = async () => {
      if (pending) return;
      pending = true;
      try {
        const next = await api.appearance();
        if (active && isThemeId(next)) setHostTheme(next);
      } catch { /* Keep the last successful theme through a reconnect. */ }
      finally { pending = false; }
    };
    void refresh();
    const timer = window.setInterval(() => void refresh(), 2000);
    return () => { active = false; window.clearInterval(timer); };
  }, [appearance, connection]);

  return { appearance, setAppearance };
}
