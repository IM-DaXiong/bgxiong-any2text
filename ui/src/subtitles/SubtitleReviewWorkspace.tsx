import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import type { Correction, Document, DocumentRevision, ReviewGroup, TimeRange } from '../api/any2text';
import { Btn, NativeDialog, SingleLineTextField, UiChromeSelect, UiChromeTextarea, standaloneRichPlainHost,
  createOsFullscreenController, chordEquals, chordFromKeyboardEvent, isComposingEvent, type RichPlainEditorHandle } from '../uiChrome';
import { BoundaryReviewPanel, precise } from './BoundaryReviewPanel';
import { entryRange, merge, split, suggest } from './subtitleDraft';
import { getChord, shortcutProps, type useSubtitleBindings } from './useSubtitleShortcuts';

const PAGE_SIZE = 80;

export function SubtitleReviewWorkspace({ draft, doc, rawDoc, activeId, revealId, select, change, undo, canUndo, dirty, busy,
  revisions, checkout, save, organize, exportPage, media, play, stop, seekPlayback, positionMs, bindings, reportError }: {
  draft: Correction; doc: Document; rawDoc?: Document; activeId: string; revealId: string;
  select(id: string, time?: number): void;
  change(draft: Correction, activeId?: string, coalesce?: boolean): void;
  undo(): void; canUndo: boolean; dirty: boolean; busy: boolean;
  revisions: DocumentRevision[]; checkout(revision: string): void; save(): void;
  organize(): void; exportPage(): void; media: ReactNode;
  play(range: TimeRange, loop: boolean, failed: (error: string) => void): void; stop(): void;
  seekPlayback(ms: number): void; positionMs: number;
  bindings: ReturnType<typeof useSubtitleBindings>;
  reportError(reason: unknown): void;
}) {
  const [query, setQuery] = useState('');
  const [page, setPage] = useState(0);
  const [review, setReview] = useState<ReviewGroup[]>();
  const [preview, setPreview] = useState<Correction>();
  const editor = useRef<RichPlainEditorHandle>(null);
  const composing = useRef(false);
  const selection = useRef<{ id: string; draft: Correction; start: number; end: number }>(null);
  const nextFocus = useRef<{ id: string; offset?: number }>(null);
  const handledReveal = useRef('');
  const rowElements = useRef(new Map<string, HTMLButtonElement>());
  const fullscreen = useMemo(() => createOsFullscreenController(getCurrentWindow, () => /Windows/i.test(navigator.userAgent)), []);
  const active = draft.entries.find(item => item.id === activeId) ?? draft.entries[0];
  const index = active ? draft.entries.indexOf(active) : -1;
  const visible = useMemo(() => draft.entries.filter(entry => entry.text.toLocaleLowerCase().includes(query.toLocaleLowerCase())), [draft.entries, query]);
  const filteredIndex = visible.findIndex(item => item.id === active?.id);
  const currentPage = Math.min(page, Math.max(0, Math.ceil(visible.length / PAGE_SIZE) - 1));
  const rows = visible.slice(currentPage * PAGE_SIZE, (currentPage + 1) * PAGE_SIZE);
  useEffect(() => {
    if (!revealId || revealId === handledReveal.current) return;
    handledReveal.current = revealId;
    const filtered = visible.findIndex(entry => entry.id === revealId);
    if (filtered >= 0) setPage(Math.floor(filtered / PAGE_SIZE));
    else {
      setQuery('');
      setPage(Math.max(0, Math.floor(draft.entries.findIndex(entry => entry.id === revealId) / PAGE_SIZE)));
    }
  }, [revealId, visible, draft.entries]);
  const sourceOrder = useMemo(() => new Map(rawDoc?.entries.map((entry, i) => [entry.id, i])), [rawDoc]);
  const rememberSelection = () => {
    if (active && editor.current) selection.current = { id: active.id, draft, ...editor.current.getSelection() };
  };
  const locate = (position: number, focusRow = false) => {
    const entry = visible[position];
    if (!entry) return false;
    if (focusRow) nextFocus.current = { id: entry.id };
    setPage(Math.floor(position / PAGE_SIZE));
    select(entry.id, entryRange(draft, entry)?.startMs);
    if (focusRow) requestAnimationFrame(() => {
      const row = rowElements.current.get(entry.id);
      row?.focus({ preventScroll: true });
      row?.scrollIntoView({ block: 'nearest' });
    });
    return true;
  };
  useEffect(() => {
    const target = nextFocus.current;
    if (!target || target.id !== active?.id || target.offset === undefined) return;
    nextFocus.current = null;
    editor.current?.focus();
    editor.current?.setSelection(target.offset, target.offset);
    const position = visible.findIndex(entry => entry.id === target.id);
    if (position >= 0) setPage(Math.floor(position / PAGE_SIZE));
  }, [draft, active?.id, visible]);
  useEffect(() => {
    const position = visible.findIndex(entry => entry.id === active?.id);
    if (position >= 0 && nextFocus.current?.offset === undefined && position < visible.length) {
      const row = rowElements.current.get(active!.id);
      if (row) row.scrollIntoView({ block: 'nearest' });
    }
  }, [active?.id, visible]);
  const getSelection = () => {
    if (!active || !editor.current) return;
    if (editor.current.getRootElement()?.contains(document.activeElement)) return editor.current.getSelection();
    const snapshot = selection.current;
    if (snapshot?.id === active.id && snapshot.draft === draft) return snapshot;
  };
  const openReview = (all: boolean) => {
    const groups = draft.reviewGroups.filter(group => all || group.id === active?.reviewGroupId);
    if (!groups.length) return false;
    stop(); setReview(groups); return true;
  };
  const closeReview = () => { stop(); setReview(undefined); };
  const editAtSelection = (lineBreak: boolean) => {
    const range = getSelection();
    if (!active || !range || !editor.current || composing.current || busy) return false;
    if (lineBreak) {
      editor.current.focus();
      editor.current.replaceSelection('\n', range);
      return true;
    }
    if (range.start !== range.end) return false;
    const next = split(draft, active.id, range.start);
    if (!next) return false;
    const id = next.entries[index + 1].id;
    nextFocus.current = { id, offset: 0 };
    change(next, id);
    return true;
  };
  const mergeAt = (left: number) => {
    if (busy) return false;
    const next = merge(draft, left, sourceOrder);
    if (!next) return false;
    const id = next.entries[left].id;
    nextFocus.current = { id, offset: draft.entries[left].text.length };
    change(next, id);
    return true;
  };
  const actions = {
    previousSegment: () => locate(filteredIndex - 1),
    nextSegment: () => locate(filteredIndex < 0 ? visible.length : filteredIndex + 1),
    insertLineBreak: () => editAtSelection(true),
    splitAtCursor: () => editAtSelection(false),
    mergePrevious: () => mergeAt(index - 1),
    mergeNext: () => mergeAt(index),
    suggestBreaks: () => {
      if (busy) return false;
      const next = suggest(draft, sourceOrder);
      if (next === draft) return false;
      stop(); setPreview(next); return true;
    },
    undoEdit: () => { if (!canUndo || busy) return false; undo(); return true; },
    reviewCurrent: () => openReview(false),
    reviewAll: () => openReview(true),
  };
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (review || preview || document.querySelector('dialog[open]') || composing.current
        || event.defaultPrevented || event.repeat || isComposingEvent(event)) return;
      const target = event.target instanceof Element ? event.target : null;
      if (event.code === 'F11') {
        event.preventDefault();
        void fullscreen('toggle').catch(reportError);
        return;
      }
      if (event.code === 'Escape' && !target?.closest('input, select, [contenteditable]')) {
        void fullscreen('exit').catch(reportError);
        return;
      }
      if (target?.closest('input, select, textarea, audio, video')) return;
      const plain = !event.ctrlKey && !event.altKey && !event.shiftKey && !event.metaKey;
      let handled = false;
      const focusedRow = target?.closest('[data-subtitle-row]');
      if (focusedRow && plain) {
        if (event.code === 'ArrowUp') handled = locate(filteredIndex - 1, true);
        if (event.code === 'ArrowDown') handled = locate(filteredIndex + 1, true);
        if (event.code === 'Enter') handled = locate(visible.findIndex(entry => entry.id === focusedRow.getAttribute('data-subtitle-row')), true);
      }
      const chord = chordFromKeyboardEvent(event);
      if (!handled && chord) {
        for (const [id, run] of Object.entries(actions)) {
          const binding = getChord(bindings, id);
          if (binding && chordEquals(binding, chord)) { handled = run(); break; }
        }
      }
      if (handled) event.preventDefault();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });
  const button = (id: keyof typeof actions, label: string, disabled = false) =>
    <Btn variant="ghost" {...shortcutProps(bindings, id)} disabled={disabled}
      onPointerDown={rememberSelection} onClick={() => actions[id]()}>{label}</Btn>;

  return <div className="subtitle-workspace" onCompositionStart={() => { composing.current = true; }}
    onCompositionEnd={() => { composing.current = false; }}>
    <section className="panel subtitle-pane subtitle-list-pane">
      <h2>定位与字幕</h2>
      {media}
      <SingleLineTextField label="搜索字幕" type="search" hideLabel value={query}
        onValue={value => { setQuery(value); setPage(0); }} placeholder="搜索字幕" />
      <div className="row">
        {button('previousSegment', '上一段', filteredIndex <= 0)}
        {button('nextSegment', '下一段', filteredIndex < 0 || filteredIndex + 1 >= visible.length)}
      </div>
      <div className="subtitle-rows" role="group" aria-label="字幕列表">
        {!visible.length && <p>{draft.entries.length ? '没有匹配的字幕。' : '当前没有可显示的字幕。'}</p>}
        {rows.map(entry => <div key={entry.id} data-subtitle-row={entry.id}>
          <Btn ref={node => { if (node) rowElements.current.set(entry.id, node); else rowElements.current.delete(entry.id); }}
            variant={entry.id === active?.id ? 'accent' : 'ghost'} aria-pressed={entry.id === active?.id}
            tabIndex={entry.id === active?.id || (filteredIndex < 0 || Math.floor(filteredIndex / PAGE_SIZE) !== currentPage) && entry === rows[0] ? 0 : -1}
            style={{ display: 'grid', gridTemplateColumns: 'auto minmax(0,1fr)', textAlign: 'start', gap: 10, width: '100%', whiteSpace: 'normal' }}
            onClick={() => select(entry.id, entryRange(draft, entry)?.startMs)}>
            <time>{entry.startMs !== undefined ? precise(entry.startMs) : '待复核'}</time><span>{entry.text}</span>
          </Btn>
        </div>)}
      </div>
      {visible.length > PAGE_SIZE && <div className="row">
        <Btn variant="ghost" disabled={!currentPage} onClick={() => setPage(currentPage - 1)}>上一页</Btn>
        <span>{currentPage + 1} / {Math.ceil(visible.length / PAGE_SIZE)}</span>
        <Btn variant="ghost" disabled={(currentPage + 1) * PAGE_SIZE >= visible.length} onClick={() => setPage(currentPage + 1)}>下一页</Btn>
      </div>}
    </section>
    <section className="panel subtitle-pane subtitle-edit-pane">
      <div className="row spread"><h2>校订</h2><span>{dirty ? '未保存' : '已保存'}{draft.reviewGroups.length ? ' · 时间待复核' : ''}</span></div>
      {revisions.length > 1 && <UiChromeSelect label="文稿版本" value={doc.revision}
        disabled={busy} onChange={event => checkout(event.target.value)}>
        {revisions.map((row, i) => <option key={row.id} value={row.id}>{row.kind === 'original' ? '识别原文' : '校订'} · 版本 {revisions.length - i}</option>)}
      </UiChromeSelect>}
      <div className="row">
        <div className="row" role="group" aria-label="字幕编辑">
          {button('insertLineBreak', '插入换行', !active || busy)}
          {button('splitAtCursor', '在光标处拆段', !active || busy)}
          {button('suggestBreaks', '自动整理', !active || busy)}
          {button('undoEdit', '撤销', !canUndo || busy)}
        </div>
        <div className="row" role="group" aria-label="合并">
          {button('mergePrevious', '与上一段合并', index < 1 || busy)}
          {button('mergeNext', '与下一段合并', index < 0 || index + 1 >= draft.entries.length || busy)}
        </div>
        <div className="row" role="group" aria-label="时间复核">
          {button('reviewCurrent', '复核当前段', !active?.reviewGroupId)}
          {button('reviewAll', '复核全部', !draft.reviewGroups.length)}
        </div>
      </div>
      {active && <>
        <UiChromeTextarea contentHeight key={active.id} ref={editor} richHost={standaloneRichPlainHost}
          textareaExtraClassName="subtitle-text" shellClassName="subtitle-editor-shell"
          textareaProps={{ value: active.text, 'aria-label': '当前字幕', onBlur: rememberSelection,
            onChange: text => change({ ...draft, entries: draft.entries.map(entry => entry.id === active.id ? { ...entry, text } : entry) }, active.id, true) }} />
        <details><summary>识别原文</summary>{active.sourceIds.map(id =>
          <p className="subtitle-text" key={id}>{rawDoc?.entries.find(item => item.id === id)?.text}</p>)}</details>
      </>}
      <div className="row">
        <Btn disabled={busy || !dirty} onClick={save}>保存校订</Btn>
        <Btn variant="ghost" disabled={dirty || !!draft.reviewGroups.length} onClick={organize}>整理成稿</Btn>
        <Btn variant="ghost" onClick={exportPage}>导出</Btn>
      </div>
    </section>
    {review && <BoundaryReviewPanel draft={draft} initialGroups={review} close={closeReview}
      apply={next => change(next)} play={play} stop={stop} seekPlayback={seekPlayback} positionMs={positionMs} bindings={bindings} />}
    {preview && <NativeDialog title="自动整理预览" onClose={() => setPreview(undefined)} className="subtitle-dialog">
      <div className="subtitle-preview">
        <div><strong>原分段</strong>{draft.entries.map(entry => <p key={entry.id}>{entry.text}</p>)}</div>
        <div><strong>建议分段</strong>{preview.entries.map(entry => <p key={entry.id}>{entry.text}</p>)}</div>
      </div>
      <div className="row"><Btn variant="ghost" onClick={() => setPreview(undefined)}>取消</Btn>
        <Btn onClick={() => { change(preview); setPreview(undefined); }}>应用</Btn></div>
    </NativeDialog>}
  </div>;
}
