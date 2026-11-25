import type { Correction, Document, ReviewGroup, SubtitleEntry, TimeRange } from '../api/any2text';

export function fromDocument(document: Document): Correction {
  return { schemaVersion: 2, entries: document.entries, reviewGroups: document.reviewGroups };
}

export function entryRange(draft: Correction, entry?: SubtitleEntry): TimeRange | undefined {
  if (!entry) return;
  if (entry.reviewGroupId) return draft.reviewGroups.find(group => group.id === entry.reviewGroupId);
  if (entry.startMs !== undefined) return { startMs: entry.startMs, endMs: entry.endMs };
}

function withTiming(entry: SubtitleEntry, timing: TimeRange | { reviewGroupId: string }): SubtitleEntry {
  return { id: entry.id, text: entry.text, sourceIds: entry.sourceIds, ...timing };
}

function newId() {
  return crypto.randomUUID();
}

function regroup(draft: Correction, entries: SubtitleEntry[], start: number, members: SubtitleEntry[], range: TimeRange, old: ReviewGroup[], boundaries: ReviewGroup['boundaries']): Correction {
  const reviewGroups = draft.reviewGroups.filter(group => !old.includes(group));
  if (members.length > 1) {
    const id = old[0]?.id ?? newId();
    reviewGroups.push({ ...range, id, memberIds: members.map(entry => entry.id), boundaries });
    entries.splice(start, members.length, ...members.map(entry => withTiming(entry, { reviewGroupId: id })));
  } else {
    entries[start] = withTiming(members[0], range);
  }
  return { ...draft, entries, reviewGroups };
}

export function split(draft: Correction, id: string, at: number): Correction | undefined {
  const index = draft.entries.findIndex(entry => entry.id === id);
  const entry = draft.entries[index];
  const range = entryRange(draft, entry);
  if (!entry || !range || !entry.text.slice(0, at).trim() || !entry.text.slice(at).trim()) return;
  const segmenter = new Intl.Segmenter(undefined, { granularity: 'grapheme' });
  if (segmenter.segment(entry.text).containing(at)?.index !== at) return;
  const old = draft.reviewGroups.find(group => group.id === entry.reviewGroupId);
  const members = old ? draft.entries.filter(item => item.reviewGroupId === old.id) : [entry];
  const offset = members.findIndex(item => item.id === id);
  if (range.endMs - range.startMs < members.length + 1) return;
  const left = { ...entry, text: entry.text.slice(0, at) };
  const right = { ...entry, id: newId(), text: entry.text.slice(at) };
  const boundaries = [...(old?.boundaries ?? [])];
  const min = boundaries[offset - 1]?.suggestedMs ?? range.startMs;
  const max = boundaries[offset]?.suggestedMs ?? range.endMs;
  if (max - min < 2) return;
  boundaries.splice(offset, 0, {
    suggestedMs: Math.min(max - 1, Math.max(min + 1, Math.round(min + (max - min) * at / entry.text.length))),
    ready: false,
  });
  members.splice(offset, 1, left, right);
  const entries = [...draft.entries];
  entries.splice(index, 1, left, right);
  return regroup(draft, entries, index - offset, members, range, old ? [old] : [], boundaries);
}

function joinText(left: string, right: string) {
  return left + (/[A-Za-z0-9]$/.test(left) && /^[A-Za-z0-9]/.test(right) ? ' ' : '') + right;
}

export function merge(draft: Correction, index: number, sourceOrder: Map<string, number>): Correction | undefined {
  const left = draft.entries[index], right = draft.entries[index + 1];
  if (!left || !right) return;
  const leftRange = entryRange(draft, left), rightRange = entryRange(draft, right);
  if (!leftRange || !rightRange) return;
  const a = draft.reviewGroups.find(group => group.id === left.reviewGroupId);
  const b = draft.reviewGroups.find(group => group.id === right.reviewGroupId);
  const range = { startMs: Math.min(leftRange.startMs, rightRange.startMs), endMs: Math.max(leftRange.endMs, rightRange.endMs) };
  const start = a ? draft.entries.findIndex(entry => entry.id === a.memberIds[0]) : index;
  const end = b ? draft.entries.findIndex(entry => entry.id === b.memberIds.at(-1)) : index + 1;
  const members = draft.entries.slice(start, end + 1);
  const joined = { ...left, text: joinText(left.text, right.text),
    sourceIds: [...new Set([...left.sourceIds, ...right.sourceIds])].sort((x, y) =>
      (sourceOrder.get(x) ?? 0) - (sourceOrder.get(y) ?? 0)) };
  members.splice(index - start, 2, joined);
  const boundaries = a && a === b
    ? a.boundaries.filter((_, boundary) => boundary !== index - start)
    : [...(a?.boundaries ?? []), ...(b?.boundaries ?? [])];
  const entries = [...draft.entries];
  entries.splice(index, 2, joined);
  return regroup(draft, entries, start, members, range, [...new Set([a, b].filter(group => group !== undefined))], boundaries);
}

export function suggest(draft: Correction, sourceOrder: Map<string, number>): Correction {
  let next = draft;
  for (const original of draft.entries) {
    const index = next.entries.findIndex(entry => entry.id === original.id);
    const entry = next.entries[index], following = next.entries[index + 1];
    if (!entry) continue;
    if (following && entry.endMs !== undefined && following.startMs !== undefined
      && following.startMs >= entry.endMs && following.startMs - entry.endMs <= 180
      && !/[。！？!?；;][”’」』"')]*\s*$/.test(entry.text)
      && !/[.]\s*$/.test(entry.text) && entry.text.length + following.text.length <= 90) {
      next = merge(next, index, sourceOrder) ?? next;
    }
    let id = entry.id;
    const text = next.entries.find(item => item.id === id)?.text ?? '';
    const boundaries = [...text.matchAll(/[。！？!?]+[”’」』"')]*|(?<!\b[A-Z])\.(?=\s+[A-Z][a-z])/gu)];
    let offset = 0;
    for (const match of boundaries) {
      const at = match.index + match[0].length;
      if (text.length < 45 || at - offset < 12 || text.length - at < 12) continue;
      const result = split(next, id, at - offset);
      if (!result) continue;
      const rightIndex = result.entries.findIndex(item => item.id === id) + 1;
      next = result;
      id = result.entries[rightIndex].id;
      offset = at;
    }
    const tail = next.entries.find(item => item.id === id);
    if (tail && tail.text.length > 72 && !tail.text.includes('\n')) {
      const pause = /[，、；：,;:]\s*/gu;
      const points = [...tail.text.matchAll(pause)].map(match => match.index + match[0].length)
        .filter(at => at > 18 && at < tail.text.length - 18)
        .sort((a, b) => Math.abs(a - tail.text.length / 2) - Math.abs(b - tail.text.length / 2));
      const at = points[0];
      if (at !== undefined) next = { ...next, entries: next.entries.map(item =>
        item.id === id ? { ...item, text: item.text.slice(0, at) + '\n' + item.text.slice(at) } : item) };
    }
  }
  return next;
}

export function confirm(draft: Correction, groups: ReviewGroup[]): Correction | undefined {
  const timing = new Map<string, TimeRange>();
  for (const group of groups) {
    let start = group.startMs;
    for (const [index, id] of group.memberIds.entries()) {
      const point = group.boundaries[index];
      const end = point?.suggestedMs ?? group.endMs;
      if ((point && !point.ready) || end <= start || (point && end >= group.endMs)) return;
      timing.set(id, { startMs: start, endMs: end });
      start = end;
    }
  }
  const reviewed = new Set(groups.map(group => group.id));
  return { ...draft,
    entries: draft.entries.map(entry => {
      const range = timing.get(entry.id);
      return range ? withTiming(entry, range) : entry;
    }),
    reviewGroups: draft.reviewGroups.filter(group => !reviewed.has(group.id)) };
}
