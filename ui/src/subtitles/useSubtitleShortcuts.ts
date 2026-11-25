import { useEffect, useState } from 'react';
import { api } from '../api/any2text';
import { chordEquals, formatChordForDisplay, isAllowedAny2TextChord, type ShortcutChord } from '../uiChrome';
import contract from '../shortcuts/any2text-shortcuts.generated.json';

type BindingMap = Record<string, ShortcutChord | null>;

export function useSubtitleBindings(allowed: boolean, endpoint?: string) {
  const [bindings, setBindings] = useState<BindingMap>({});
  useEffect(() => {
    if (!allowed) return;
    let live = true;
    let generation = 0;
    const refresh = async () => {
      const request = ++generation;
      try {
        const value = await api.shortcutBindings() as { schemaVersion?: unknown; bindings?: Record<string, unknown> };
        if (!live || request !== generation || value.schemaVersion !== 1 || !value.bindings
          || typeof value.bindings !== 'object' || Array.isArray(value.bindings)) return;
        const accepted: BindingMap = {};
        for (const { id } of contract.actions) {
          if (!Object.hasOwn(value.bindings, id)) continue;
          const raw = value.bindings[id];
          if (raw === null) { accepted[id] = null; continue; }
          const chord = raw as ShortcutChord;
          if (!chord || typeof chord.code !== 'string'
            || Object.entries(chord).some(([key, field]) => key !== 'code' && (!['ctrl', 'alt', 'shift', 'meta'].includes(key) || typeof field !== 'boolean'))
            || !isAllowedAny2TextChord(chord)) return;
          accepted[id] = chord;
        }
        const effective = contract.actions.map(action => Object.hasOwn(accepted, action.id) ? accepted[action.id] : action.defaultChord);
        if (effective.some((chord, i) => chord && effective.slice(i + 1).some(other => other && chordEquals(chord, other)))) return;
        setBindings(accepted);
      } catch { /* Last validated snapshot stays in effect. */ }
    };
    void refresh();
    window.addEventListener('focus', refresh);
    return () => { live = false; window.removeEventListener('focus', refresh); };
  }, [allowed, endpoint]);
  return bindings;
}

export function getChord(bindings: BindingMap, action: string): ShortcutChord | null | undefined {
  const id = 'any2text.' + action;
  return Object.hasOwn(bindings, id) ? bindings[id] : contract.actions.find(row => row.id === id)?.defaultChord;
}

export function shortcutProps(bindings: BindingMap, action: string) {
  const chord = getChord(bindings, action);
  if (!chord) return {};
  const code = chord.code.replace(/^(Key|Digit)/, '');
  return { shortcutLabel: formatChordForDisplay(chord, 'zh-CN', false),
    'aria-keyshortcuts': [chord.ctrl && 'Control', chord.alt && 'Alt', chord.shift && 'Shift', chord.meta && 'Meta', code].filter(Boolean).join('+') };
}
