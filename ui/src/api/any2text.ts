import { Channel, invoke } from '@tauri-apps/api/core';

export interface ImportProgress {
  phase: 'checking' | 'sending' | 'verifying';
  bytes: number | null;
  total: number | null;
}

export interface Model {
  id: string;
  name: string;
  languages: string[];
  timings: string[];
  installed: boolean;
  ready: boolean;
  issues: string[];
}

export interface Capabilities {
  providerId: string;
  endpointInstance: string;
  serviceId: string;
  models: Model[];
  operations: string[];
  pause: boolean;
  resume: boolean;
}

export interface Project {
  id: string;
  name: string;
  providerId: string;
  serviceId: string;
  original: string | null;
  correction: string | null;
  input: { durationMs: number; audioTracks: { index: number; codec: string }[] };
}

export interface LocalJob {
  projectId: string;
  providerId: string;
  serviceId: string;
  request: { clientRequestId: string; operation: { kind: 'transcribe' | 'organize' } };
  remote: { status: string; committedMs: number; durationMs: number; error: { code: string; message: string } | null } | null;
  document: string | null;
}

export interface Segment { id: string; startMs: number; endMs: number; text: string }
export interface TimeRange { startMs: number; endMs: number }
export type SubtitleEntry = Pick<Segment, 'id' | 'text'> & { sourceIds: string[] } &
  (TimeRange & { reviewGroupId?: never } | { reviewGroupId: string; startMs?: never; endMs?: never });
export interface ReviewGroup extends TimeRange {
  id: string;
  memberIds: string[];
  boundaries: { suggestedMs: number; ready: boolean }[];
}
export interface Correction { schemaVersion: 2; entries: SubtitleEntry[]; reviewGroups: ReviewGroup[] }
export interface Document { entries: SubtitleEntry[]; reviewGroups: ReviewGroup[]; revision: string; original: string; segments: Segment[]; waveformPeaks: number[] }
export interface DocumentRevision { id: string; kind: 'original' | 'correction' }
export interface Candidate {
  id: string;
  revision: string;
  accepted: boolean;
  draft: {
    sourceRevision: string;
    draftType: 'spokenScript' | 'literaryScript';
    mode: string;
    blocks: { text: string; sourceSegmentIds: string[]; addedOrInferred: boolean; abridged: boolean }[];
    abridgedSourceSegmentIds: string[];
  };
}

export const api = {
  connect: () => invoke<Capabilities>('connect'),
  disconnect: () => invoke<void>('disconnect'),
  health: () => invoke<void>('provider_health'),
  shortcutBindings: () => invoke<unknown>('provider_shortcut_bindings'),
  appearance: () => invoke<string | null>('provider_appearance'),
  launchBigBear: (executable?: string) => invoke<boolean>('launch_bigbear', { executable: executable ?? null }),
  projects: () => invoke<Project[]>('projects'),
  importMedia: (path: string, onProgress: (update: ImportProgress) => void) => {
    const progress = new Channel<ImportProgress>(onProgress);
    return invoke<Project>('import_media', { path, progress });
  },
  relocate: (projectId: string, path: string) => invoke<Project>('relocate_media', { projectId, path }),
  copyMedia: (projectId: string) => invoke<Project>('copy_media', { projectId }),
  playback: (projectId: string) => invoke<string>('playback_path', { projectId }),
  deleteJob: (requestId: string) => invoke<void>('delete_job', { requestId }),
  jobs: () => invoke<LocalJob[]>('jobs'),
  submit: (projectId: string, modelConfigId: string, audioTrack: number, language: string) =>
    invoke<LocalJob>('submit_transcription', { projectId, modelConfigId, audioTrack, language }),
  refresh: (requestId: string) => invoke<LocalJob>('refresh_job', { requestId }),
  retryPending: (requestId: string) => invoke<LocalJob>('retry_pending_request', { requestId }),
  control: (requestId: string, action: string) => invoke<LocalJob>('control_job', { requestId, action }),
  retrieve: (requestId: string) => invoke<Project>('retrieve', { requestId }),
  document: (projectId: string, original = false) => invoke<Document>('document', { projectId, original }),
  revisions: (projectId: string) => invoke<DocumentRevision[]>('revisions', { projectId }),
  checkout: (projectId: string, revision: string, expectedRevision: string) => invoke<Project>('checkout_revision', { projectId, revision, expectedRevision }),
  saveSubtitleRevision: (projectId: string, expectedRevision: string, correction: Correction) =>
    invoke<Document>('save_subtitle_revision', { projectId, expectedRevision, correction }),
  saveCorrection: (projectId: string, expectedRevision: string, changes: Record<string, string>) =>
    invoke<Document>('save_correction', { projectId, expectedRevision, changes }),
  preview: (projectId: string, original: boolean, format: string, range: TimeRange | null, expectedRevision?: string) => invoke<string>('preview_export', { projectId, original, format, range, expectedRevision }),
  export: (projectId: string, original: boolean, format: string, path: string, range: TimeRange | null, expectedRevision?: string) =>
    invoke<string>('export_document', { projectId, original, format, path, range, expectedRevision }),
  organize: (projectId: string, expectedRevision: string, draftType: string, mode: string) =>
    invoke<LocalJob>('submit_organize', { projectId, expectedRevision, draftType, mode }),
  candidates: (projectId: string) => invoke<Candidate[]>('candidates', { projectId }),
  saveCandidate: (id: string, expectedRevision: string, blocks: string[]) =>
    invoke<Candidate>('save_candidate', { id, expectedRevision, blocks }),
  acceptCandidate: (id: string, expectedRevision: string) => invoke<Candidate>('accept_candidate', { id, expectedRevision }),
  previewCandidate: (id: string, format: string) => invoke<string>('preview_candidate', { id, format }),
  exportCandidate: (id: string, format: string, path: string) => invoke<string>('export_candidate', { id, format, path }),
};
