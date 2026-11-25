import { useEffect, useState } from 'react';
import type { Correction, ReviewGroup, TimeRange } from '../api/any2text';
import { Btn, NativeDialog, UiRangeSlider, chordEquals, chordFromKeyboardEvent, isComposingEvent } from '../uiChrome';
import { confirm } from './subtitleDraft';
import { getChord, shortcutProps, type useSubtitleBindings } from './useSubtitleShortcuts';

export function precise(ms: number) {
  return new Date(ms).toISOString().slice(11, 23);
}

export function BoundaryReviewPanel({ draft, initialGroups, close, apply, play, stop, seekPlayback, positionMs, bindings }: {
  draft: Correction;
  initialGroups: ReviewGroup[];
  close(): void;
  apply(next: Correction): void;
  play(range: TimeRange, loop: boolean, failed: (error: string) => void): void;
  seekPlayback(ms: number): void;
  positionMs: number;
  stop(): void;
  bindings: ReturnType<typeof useSubtitleBindings>;
}) {
  const [groups, setGroups] = useState(initialGroups);
  const [index, setIndex] = useState(0);
  const [error, setError] = useState('');
  const group = groups[index];
  const playGroup = (loop: boolean) => { setError(''); play(group, loop, setError); };
  const finish = () => {
    const next = confirm(draft, groups);
    if (!next) {
      setIndex(Math.max(0, groups.findIndex(item => !confirm(draft, [item]))));
      setError('请先确认每处分界位置。');
      return;
    }
    apply(next);
    close();
  };
  useEffect(() => () => stop(), [stop]);
  useEffect(() => {
    const key = (event: KeyboardEvent) => {
      const chord = chordFromKeyboardEvent(event);
      const binding = getChord(bindings, 'confirmReview');
      if (!chord || !binding || !chordEquals(chord, binding) || event.defaultPrevented
        || event.repeat || isComposingEvent(event) || (event.target as Element | null)?.closest('input')) return;
      event.preventDefault();
      finish();
    };
    window.addEventListener('keydown', key);
    return () => window.removeEventListener('keydown', key);
  });
  return <NativeDialog title={groups.length === 1 ? '复核字幕时间' : '复核全部字幕时间'} onClose={close} className="subtitle-dialog">
    <div className="row">
      <Btn variant="ghost" disabled={!index} onClick={() => { stop(); setIndex(index - 1); }}>上一组</Btn>
      <span>{index + 1} / {groups.length}</span>
      <Btn variant="ghost" disabled={index + 1 === groups.length} onClick={() => { stop(); setIndex(index + 1); }}>下一组</Btn>
      <time>{precise(group.startMs)} – {precise(group.endMs)}</time>
    </div>
    <div className="row">
      <Btn variant="ghost" onClick={() => playGroup(false)}>试听</Btn>
      <Btn variant="ghost" onClick={() => playGroup(true)}>循环试听</Btn>
      <Btn variant="ghost" onClick={stop}>停止</Btn>
    </div>
    <UiRangeSlider aria-label="局部试听位置" min={group.startMs} max={group.endMs}
      value={Math.min(group.endMs, Math.max(group.startMs, positionMs))}
      onChange={seekPlayback} />
    {group.boundaries.map((boundary, i) => {
      const min = (group.boundaries[i - 1]?.suggestedMs ?? group.startMs) + 1;
      const max = (group.boundaries[i + 1]?.suggestedMs ?? group.endMs) - 1;
      const update = (suggestedMs: number, ready: boolean) => {
        setError('');
        setGroups(previous => previous.map((item, n) => n === index ? {
          ...item, boundaries: item.boundaries.map((point, j) => j === i ? { suggestedMs, ready } : point),
        } : item));
      };
      return <section key={i} className="subtitle-boundary">
        <p>{draft.entries.find(entry => entry.id === group.memberIds[i])?.text}</p>
        <div className="row"><time>{precise(boundary.suggestedMs)}</time>
          <Btn variant="ghost" disabled={min > max || boundary.ready}
            onClick={() => update(boundary.suggestedMs, true)}>{boundary.ready ? '已认可' : '认可此分界'}</Btn>
        </div>
        <UiRangeSlider aria-label={`分界 ${i + 1}`} min={min} max={Math.max(min, max)}
          disabled={min > max} value={boundary.suggestedMs} onChange={value => update(value, false)} />
        <p>{draft.entries.find(entry => entry.id === group.memberIds[i + 1])?.text}</p>
      </section>;
    })}
    {error && <p role="alert">{error}</p>}
    <div className="row"><Btn variant="ghost" onClick={close}>取消</Btn>
      <Btn {...shortcutProps(bindings, 'confirmReview')} onClick={finish}>确认时间</Btn>
    </div>
  </NativeDialog>;
}
