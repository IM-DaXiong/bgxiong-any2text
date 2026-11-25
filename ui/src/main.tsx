import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { convertFileSrc } from '@tauri-apps/api/core';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { open, save } from '@tauri-apps/plugin-dialog';
import { api, type Candidate, type Capabilities, type Document, type DocumentRevision, type ImportProgress, type LocalJob, type Project } from './api/any2text';
import { isThemeId, themeIds, themeMeta, useAppearance } from './theme';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { Btn, NativeDialog, UiRangeSlider } from './uiChrome';
import { SubtitleReviewWorkspace } from './subtitles/SubtitleReviewWorkspace';
import { entryRange, fromDocument } from './subtitles/subtitleDraft';
import { useSubtitleBindings } from './subtitles/useSubtitleShortcuts';
import type { Correction, TimeRange } from './api/any2text';
import '@bgxiong/standalone-chrome/style.css';
import { WindowTitleBar } from './WindowTitleBar';
import './bigbear-theme.css';
import './style.css';

const pages = ['导入素材', '任务队列', '字幕校对', '整理成稿', '导出交付'] as const;
const statuses: Record<string, string> = {
  queued: '排队中', running: '处理中', pausing: '正在保存当前片段', paused: '已暂停',
  cancelling: '正在停止任务', cancelled: '已取消', failed: '执行失败', resultReady: '结果待取回',
};
const mediaFilters = [{ name: '音视频文件', extensions: ['wav', 'mp3', 'mp4', 'mov', 'mkv', 'avi', 'webm', 'm4a', 'aac', 'flac', 'ogg'] }];
const terminal = new Set(['resultReady', 'cancelled', 'failed']);
const modelIssues: Record<string, string> = {
  'asr.modelUnavailable': '缺少模型或模型模块',
  'asr.runtimeNotVerified': '识别运行环境未就绪',
  'asr.configurationInvalid': '模型配置无效',
  'asr.platformNotVerified': '此系统上的模型未验证',
};
const duration = (ms: number) => {
  const n = Math.floor(ms / 1000);
  return [Math.floor(n / 3600), Math.floor(n / 60) % 60, n % 60].map(v => `${v}`.padStart(2, '0')).join(':');
};
const titleFor = (type: string) => type === 'spokenScript' ? '口播稿' : '文学剧本';
const importLabels = { checking: '正在校验文件', sending: '正在传送素材', verifying: '等待比格熊检查' };
const fileName = (path: string) => path.split(/[\\/]/).at(-1) || path;
type ImportBatch = { current: number; total: number; name: string; phase?: ImportProgress };

function App() {
  const [page, setPage] = useState(0);
  const [projects, setProjects] = useState<Project[]>([]);
  const [projectId, setProjectId] = useState('');
  const [jobs, setJobs] = useState<LocalJob[]>([]);
  const [capabilities, setCapabilities] = useState<Capabilities>();
  const [providerOpen, setProviderOpen] = useState(false);
  const providerPanel = useRef<HTMLDivElement>(null);
  const providerButton = useRef<HTMLButtonElement>(null);
  const connectingRef = useRef(false);
  const [connecting, setConnecting] = useState(false);
  const [autoConnect, setAutoConnect] = useState(true);
  const [serviceIssue, setServiceIssue] = useState('');
  const [busy, setBusy] = useState(false);
  const [importBatch, setImportBatch] = useState<ImportBatch>();
  const [importErrors, setImportErrors] = useState<string[]>([]);
  const choosingMedia = useRef(false);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [modelId, setModelId] = useState('');
  const [track, setTrack] = useState(0);
  const [language, setLanguage] = useState('zh');
  const [doc, setDoc] = useState<Document>();
  const [rawDoc, setRawDoc] = useState<Document>();
  const [revisions, setRevisions] = useState<DocumentRevision[]>([]);
  const [edited, setEdited] = useState<Correction>();
  const [jobToDelete, setJobToDelete] = useState<LocalJob>();
  const [pendingDeparture, setPendingDeparture] = useState<(savedRevision?: string) => void>();
  const history = useRef<{ draft: Correction; activeId: string; typing?: string }[]>([]);
  const [activeId, setActiveId] = useState('');
  const [revealId, setRevealId] = useState('');
  const [mediaPath, setMediaPath] = useState('');
  const [mediaError, setMediaError] = useState('');
  const [positionMs, setPositionMs] = useState(0);
  const [waveformDetail, setWaveformDetail] = useState(false);
  const [draftType, setDraftType] = useState('spokenScript');
  const [mode, setMode] = useState('faithful');
  const [candidates, setCandidates] = useState<Candidate[]>([]);
  const [candidateId, setCandidateId] = useState('');
  const [draftChanges, setDraftChanges] = useState<Record<number, string>>({});
  const [exportSource, setExportSource] = useState('correction');
  const [format, setFormat] = useState('srt');
  const [selectionOnly, setSelectionOnly] = useState(false);
  const [preview, setPreview] = useState('');
  const player = useRef<HTMLMediaElement | null>(null);
  const nextSeek = useRef<number | undefined>(undefined);
  const runningAction = useRef(false);
  const project = projects.find(p => p.id === projectId);
  const timingReview = jobs.find(job => job.projectId === projectId && !!project?.original &&
    job.document === project.original && job.remote?.error?.code === 'timingReviewRequired')?.remote?.error;
  const chosen = capabilities?.models.find(m => m.id === modelId);
  const attached = !!capabilities && !serviceIssue;
  const needsAsrSetup = attached && (
    (chosen && (!chosen.installed || chosen.issues.some(issue =>
      issue === 'asr.modelUnavailable' || issue === 'asr.runtimeNotVerified'))) ||
    !capabilities.models.some(model => model.installed && !model.issues.length && model.timings.includes('segmentTimed'))
  );
  const { appearance, setAppearance } = useAppearance(capabilities);
  const serviceMatches = attached && !!project && project.serviceId === capabilities.serviceId && project.providerId === capabilities.providerId;
  const draft = useMemo(() => edited ?? (doc ? fromDocument(doc) : undefined), [edited, doc]);
  const dirty = edited !== undefined;
  const bindings = useSubtitleBindings(autoConnect, capabilities?.endpointInstance);
  const draftDirty = Object.keys(draftChanges).length > 0;
  const active = draft?.entries.find(s => s.id === activeId) ?? draft?.entries[0];
  const range = selectionOnly && draft ? entryRange(draft, active) ?? null : null;
  const expectedExport = exportSource === 'original' ? rawDoc?.revision : doc?.revision;
  const candidate = candidates.find(c => c.id === candidateId);
  const currentCandidate = candidate?.draft.sourceRevision === doc?.revision;

  const action = async (work: () => Promise<void>) => {
    if (runningAction.current) return;
    runningAction.current = true;
    setBusy(true); setError(''); setNotice('');
    try { await work(); } catch (reason) { setError(String(reason)); }
    finally { runningAction.current = false; setBusy(false); }
  };

  const loadProjects = async () => setProjects(await api.projects());
  const loadJobs = async () => setJobs(await api.jobs());
  const reloadCandidates = async (id: string) => {
    const rows = await api.candidates(id);
    setCandidates(rows);
    setCandidateId(old => rows.some(row => row.id === old) ? old : rows[0]?.id ?? '');
  };

  const selectProject = (id: string) => {
    if (id === projectId || runningAction.current) return;
    if (dirty || draftDirty) { guardDeparture(() => switchProject(id)); return; }
    switchProject(id);
  };

  const switchProject = (id: string) => {
    stopReviewPlayback(); nextSeek.current = undefined;
    setEdited(undefined); history.current = [];
    setProjectId(id); setError(''); setDoc(undefined); setRawDoc(undefined); setCandidateId('');
    setCandidates([]); setRevisions([]); setActiveId('');
  };

  const connectService = useCallback(async () => {
    if (connectingRef.current) return;
    connectingRef.current = true; setConnecting(true);
    try {
      const connected = await api.connect();
      setCapabilities(connected); setServiceIssue('');
    } catch (reason) {
      setServiceIssue(String(reason));
    } finally { connectingRef.current = false; setConnecting(false); }
  }, []);

  const connect = () => action(async () => {
    setAutoConnect(true);
    await connectService();
  });

  useEffect(() => {
    if (!autoConnect || attached) return;
    const tick = () => { if (!runningAction.current) void connectService(); };
    tick();
    const timer = window.setInterval(tick, 5000);
    return () => window.clearInterval(timer);
  }, [autoConnect, attached, connectService]);

  useEffect(() => {
    if (!capabilities) return;
    setModelId(previous => {
      if (capabilities.models.some(model => model.id === previous)) return previous;
      const ready = capabilities.models.filter(model => model.installed && !model.issues.length &&
        model.timings.includes('segmentTimed') && model.languages.includes(language));
      return ready.find(model => model.id === 'asr-offline:qwen3_asr')?.id ??
        (ready.length === 1 ? ready[0].id : '');
    });
  }, [capabilities, language]);

  useEffect(() => {
    if (!providerOpen) return;
    const outside = (event: PointerEvent) => {
      if (event.target instanceof Node && !providerPanel.current?.contains(event.target)) setProviderOpen(false);
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key === 'Escape') { setProviderOpen(false); providerButton.current?.focus(); }
    };
    document.addEventListener('pointerdown', outside);
    document.addEventListener('keydown', escape);
    return () => {
      document.removeEventListener('pointerdown', outside);
      document.removeEventListener('keydown', escape);
    };
  }, [providerOpen]);

  const cancellableJobs = jobs.filter(job => job.providerId === capabilities?.providerId && job.serviceId === capabilities?.serviceId &&
    ['queued', 'running', 'pausing', 'paused'].includes(job.remote?.status ?? ''));

  const disconnect = (cancelJobs: boolean) => action(async () => {
    setAutoConnect(false);
    let cancelUnknown = false;
    if (cancelJobs) {
      for (const job of cancellableJobs) {
        try { await api.control(job.request.clientRequestId, 'cancel'); }
        catch { cancelUnknown = true; }
      }
      await loadJobs();
    }
    await api.disconnect(); setCapabilities(undefined); setServiceIssue('');
    if (cancelUnknown) setNotice('部分取消请求待核对，重新连接后可查看状态。');
  });

  const importPaths = (selected: string[]) => action(async () => {
    const paths = [...new Set(selected)];
    if (!paths.length) return;
    setImportErrors([]);
    if (dirty || draftDirty) { setImportErrors(['请先保存当前编辑。']); return; }
    const total = paths.length;
    const failures: string[] = [];
    const imported: Project[] = [];
    setImportBatch({ current: 0, total, name: fileName(paths[0]) });
    try {
      // Let the visible WebView present the state before starting IPC. The timer also
      // lets import proceed if the window is minimized and frames stop arriving.
      await new Promise<void>(resolve => {
        const timeout = window.setTimeout(resolve, 100);
        window.requestAnimationFrame(() => window.requestAnimationFrame(() => {
          window.clearTimeout(timeout);
          resolve();
        }));
      });
      for (const [current, path] of paths.entries()) {
        const item = { current, total, name: fileName(path) };
        setImportBatch(item);
        let active = true;
        try {
          const project = await api.importMedia(path, phase => {
            if (active) setImportBatch({ ...item, phase });
          });
          imported.push(project);
        } catch (reason) {
          failures.push(`${item.name}：${String(reason)}`);
        } finally {
          active = false;
        }
      }
      if (imported.length) {
        await loadProjects();
        switchProject(imported[0].id);
        setPage(0);
        if (total > 1) setNotice(`已导入 ${imported.length} 个素材。`);
      }
    } finally {
      setImportErrors(failures);
      setImportBatch(undefined);
    }
  });

  const chooseMedia = async () => {
    if (choosingMedia.current || runningAction.current) return;
    choosingMedia.current = true;
    try {
      const paths = await open({ multiple: true, filters: mediaFilters });
      if (paths?.length) await importPaths(paths);
    } catch (reason) {
      setImportErrors([String(reason)]);
    } finally {
      choosingMedia.current = false;
    }
  };

  useEffect(() => {
    void api.projects().then(rows => { setProjects(rows); setProjectId(rows[0]?.id ?? ''); }).catch(e => setError(String(e)));
    void loadJobs().catch(e => setError(String(e)));
  }, []);

  useEffect(() => {
    if (!attached || page !== 0) return;
    let cleanup: (() => void) | undefined;
    let disposed = false;
    void getCurrentWebview().onDragDropEvent(({ payload }) => {
      if (!choosingMedia.current && payload.type === 'drop' && payload.paths.length) void importPaths(payload.paths);
    }).then(unlisten => { if (disposed) unlisten(); else cleanup = unlisten; }).catch(() => {});
    return () => { disposed = true; cleanup?.(); };
  }, [attached, page, dirty, draftDirty, projectId]);

  useEffect(() => {
    let disposed = false;
    if (!project) return;
    setTrack(project.input.audioTracks[0]?.index ?? 0);
    setMediaPath(''); setMediaError('');
    if (!project.original) { setDoc(undefined); setRawDoc(undefined); setRevisions([]); setCandidates([]); setCandidateId(''); }
    if (project.original) {
      void Promise.all([api.document(project.id), api.document(project.id, true), api.revisions(project.id)]).then(([current, raw, history]) => {
        if (disposed) return;
        setDoc(current); setRawDoc(raw); setRevisions(history);
      }).catch(e => { if (!disposed) setError(String(e)); });
      void api.playback(project.id).then(path => { if (!disposed) setMediaPath(path); })
        .catch(e => { if (!disposed) setMediaError(String(e)); });
      void api.candidates(project.id).then(rows => {
        if (disposed) return;
        setCandidates(rows); setCandidateId(old => rows.some(row => row.id === old) ? old : rows[0]?.id ?? '');
      }).catch(e => { if (!disposed) setError(String(e)); });
    }
    return () => { disposed = true; };
  }, [project?.id, project?.original]);

  useEffect(() => {
    if (!capabilities) return;
    let disposed = false;
    let polling = false;
    const tick = async () => {
      if (polling || runningAction.current || connectingRef.current) return;
      polling = true;
      try {
        await api.health();
        const current = await api.jobs();
        const activeJobs = current.filter(job => job.serviceId === capabilities.serviceId && (!job.remote || !terminal.has(job.remote.status)));
        for (const job of activeJobs) {
          if (disposed) return;
          await api.refresh(job.request.clientRequestId);
        }
        if (!disposed) { setJobs(await api.jobs()); setServiceIssue(''); }
      } catch (reason) {
        if (!disposed) setServiceIssue(String(reason));
      } finally { polling = false; }
    };
    void tick();
    const id = window.setInterval(() => void tick(), 1800);
    return () => { disposed = true; window.clearInterval(id); };
  }, [capabilities]);

  useEffect(() => {
    let disposed = false;
    setPreview('');
    if (page !== 4 || !project?.original) return;
    const request = exportSource === 'candidate' && candidate && candidate.accepted && currentCandidate && !draftDirty && !dirty
      ? api.previewCandidate(candidate.id, format)
      : exportSource !== 'candidate' && (exportSource === 'original' || !dirty) && (!selectionOnly || range)
        ? api.preview(project.id, exportSource === 'original', format, range, expectedExport) : undefined;
    void request?.then(text => { if (!disposed) setPreview(text); }).catch(e => { if (!disposed) setError(String(e)); });
    return () => { disposed = true; };
  }, [page, project?.id, project?.original, doc?.revision, exportSource, format, candidate?.revision, candidate?.accepted, currentCandidate, dirty, draftDirty, expectedExport, selectionOnly, range?.startMs, range?.endMs]);

  const seek = (id: string, ms?: number) => {
    stopReviewPlayback();
    nextSeek.current = undefined;
    setActiveId(id);
    if (ms === undefined) return;
    setPositionMs(ms);
    if (player.current) player.current.currentTime = ms / 1000;
    else nextSeek.current = ms;
  };
  const mediaReady = () => {
    if (player.current && nextSeek.current !== undefined) {
      player.current.currentTime = nextSeek.current / 1000;
      nextSeek.current = undefined;
    }
  };
  const mediaRef = (element: HTMLMediaElement | null) => { player.current = element; };
  const mediaSource = mediaPath ? convertFileSrc(mediaPath) : '';
  const reviewCleanup = useRef<() => void>(() => {});
  const stopReviewPlayback = useCallback(() => {
    reviewCleanup.current();
    reviewCleanup.current = () => {};
  }, []);
  useEffect(() => () => { stopReviewPlayback(); nextSeek.current = undefined; }, [projectId, mediaPath, page, doc?.revision, stopReviewPlayback]);
  const playReview = (range: TimeRange, loop: boolean, failed: (error: string) => void) => {
    stopReviewPlayback();
    const audio = player.current;
    if (!audio || audio.readyState < 1) { failed('素材当前无法试听。'); return; }
    audio.pause();
    let live = true;
    let timer = 0;
    const clearTimer = () => window.clearTimeout(timer);
    const start = () => {
      if (!live) return;
      if (Math.abs(audio.currentTime * 1000 - range.startMs) > 30) {
        audio.currentTime = range.startMs / 1000;
        return;
      }
      void audio.play().catch(reason => { if (live) failed(String(reason)); });
    };
    const seekStart = () => {
      if (Math.abs(audio.currentTime - range.startMs / 1000) < .001) start();
      else audio.currentTime = range.startMs / 1000;
    };
    const boundary = () => {
      clearTimer();
      if (!live || audio.paused) return;
      const left = range.endMs - audio.currentTime * 1000;
      if (left <= 0) {
        audio.pause();
        if (loop) seekStart();
        else stopReviewPlayback();
      } else timer = window.setTimeout(boundary, Math.max(1, left / audio.playbackRate));
    };
    audio.addEventListener('seeked', start);
    audio.addEventListener('playing', boundary);
    audio.addEventListener('timeupdate', boundary);
    const ended = () => { if (live && loop) seekStart(); else stopReviewPlayback(); };
    audio.addEventListener('ended', ended);
    reviewCleanup.current = () => {
      live = false; clearTimer(); audio.pause();
      audio.removeEventListener('seeked', start);
      audio.removeEventListener('playing', boundary);
      audio.removeEventListener('timeupdate', boundary);
      audio.removeEventListener('ended', ended);
    };
    seekStart();
  };
  const seekWithinReview = (ms: number) => {
    stopReviewPlayback();
    if (player.current && player.current.readyState > 0) {
      player.current.currentTime = ms / 1000;
      setPositionMs(ms);
    }
  };
  const changeDraft = (next: Correction, id = activeId, coalesce = false) => {
    if (!draft || next === draft) return;
    if (!coalesce || history.current.at(-1)?.typing !== id) {
      history.current.push({ draft, activeId, typing: coalesce ? id : undefined });
    }
    setEdited(next);
    setActiveId(id);
  };
  const undoDraft = () => {
    const previous = history.current.pop();
    if (!previous || !doc) return;
    setEdited(previous.draft.entries === doc.entries && previous.draft.reviewGroups === doc.reviewGroups ? undefined : previous.draft);
    setActiveId(previous.activeId);
  };
  const persistSubtitle = async () => {
    if (!dirty || !doc || !draft) return;
    const saved = await api.saveSubtitleRevision(projectId, doc.revision, draft);
    setDoc(saved);
    setEdited(current => current === draft ? undefined : current);
    for (const snapshot of history.current) snapshot.typing = undefined;
    setRevisions(await api.revisions(projectId)); await loadProjects();
    setNotice('校订已保存');
    return saved.revision;
  };
  const guardDeparture = (proceed: (savedRevision?: string) => void) => {
    if (dirty || draftDirty) setPendingDeparture(() => proceed);
    else proceed();
  };
  const checkoutSubtitle = (revision: string) => guardDeparture(savedRevision => void action(async () => {
    if (!doc) return;
    await api.checkout(projectId, revision, savedRevision ?? doc.revision);
    const [updated, raw] = await Promise.all([api.document(projectId), api.document(projectId, true)]);
    setDoc(updated); setRawDoc(raw); setActiveId(''); setEdited(undefined);
    history.current = [];
    await loadProjects();
  }));
  const requestWindowClose = useCallback(() => {
    if (runningAction.current) {
      setError('请等当前操作完成后再关闭。');
      return;
    }
    const close = () => { void getCurrentWindow().destroy().catch(reason => setError(String(reason))); };
    if (dirty || draftDirty) setPendingDeparture(() => close);
    else close();
  }, [dirty, draftDirty]);
  useEffect(() => {
    const listener = getCurrentWindow().onCloseRequested(event => {
      event.preventDefault();
      requestWindowClose();
    });
    return () => { void listener.then(unlisten => unlisten()).catch(() => {}); };
  }, [requestWindowClose]);
  const peaks = doc?.waveformPeaks ?? [];
  const peakBars = waveformDetail ? peaks : peaks.filter((_, index) => index % 4 === 0)
    .map((_, index) => Math.max(...peaks.slice(index * 4, index * 4 + 4)));
  const trackPosition = () => setPositionMs(Math.round((player.current?.currentTime ?? 0) * 1000));
  const canSubmit = serviceMatches && chosen?.installed && !chosen.issues.length && chosen.timings.includes('segmentTimed') && chosen.languages.includes(language);

  return <><WindowTitleBar onError={setError} onClose={requestWindowClose} /><div className={page === 2 ? 'shell subtitle-shell' : 'shell'}>
    <aside><nav aria-label="工作区导航">
      {pages.map((name, index) => <button key={name} aria-current={page === index ? 'page' : undefined} onClick={() => { setPage(index); setError(''); }}>{name}</button>)}
    </nav></aside>
    <div className="main">
      <header><select aria-label="项目" value={projectId} onChange={e => selectProject(e.target.value)}>
        <option value="">选择项目</option>{projects.map(item => <option key={item.id} value={item.id}>{item.name}</option>)}
      </select><div className="row"><select aria-label="外观主题" value={appearance} onChange={event => {
        const value = event.target.value;
        if (value === 'follow' || isThemeId(value)) setAppearance(value);
      }}><option value="follow">跟随比格熊</option>{themeIds.map(id => <option key={id} value={id}>{themeMeta[id].label}</option>)}</select>
      <div className="provider-anchor" ref={providerPanel}>
      <button ref={providerButton} aria-expanded={providerOpen} aria-controls="provider-panel" onClick={() => setProviderOpen(!providerOpen)}>比格熊 · {connecting ? '连接中…' : attached ? '已连接' : '未连接'}</button>
      {providerOpen && <section id="provider-panel" className="provider panel stack" aria-label="服务连接">
        <div className="row"><strong>比格熊本地服务</strong><span className="muted">{capabilities?.endpointInstance}</span></div>
        {serviceIssue && <p role="status">{serviceIssue}</p>}
        <div className="row"><button disabled={busy || connecting} onClick={connect}>{connecting ? '连接中…' : attached ? '刷新能力' : '连接服务'}</button>
          {capabilities && <button disabled={busy || connecting} onClick={() => void disconnect(false)}>{cancellableJobs.length ? '保留任务并断开' : '断开连接'}</button>}
          {attached && cancellableJobs.length > 0 && <button disabled={busy || connecting} onClick={() => void disconnect(true)}>请求取消后断开</button>}
          <button disabled={busy || connecting} onClick={() => void action(async () => {
            if (await api.launchBigBear()) { setAutoConnect(true); setNotice('比格熊已启动，服务就绪后将自动连接。'); return; }
            const path = await open({ multiple: false, filters: [{ name: '比格熊主程序', extensions: ['exe'] }] });
            if (typeof path === 'string') { await api.launchBigBear(path); setAutoConnect(true); setNotice('比格熊已启动，服务就绪后将自动连接。'); }
          })}>启动比格熊</button></div>
      </section>}
      </div></div></header>
      <main>
        {error && <p className="alert" role="alert">{error}</p>}
        {notice && <p className="notice" role="status">{notice}</p>}
        {page === 0 && <div className="columns">
          <section className="panel stack"><h2>本机素材</h2><button disabled={busy || !attached} className="primary" onClick={() => void chooseMedia()}>选择音视频</button>
            {importBatch && <div className="import-progress stack" role="status">
              <span>{importBatch.phase ? importLabels[importBatch.phase.phase] : '正在导入'}</span>
              {importBatch.total > 1 && <>
                <span>{importBatch.current + 1} / {importBatch.total} · {importBatch.name}</span>
                <progress aria-label="批量导入进度" max={importBatch.total} value={importBatch.current} />
              </>}
              <progress aria-label="导入进度" max={importBatch.phase?.total || 1}
                value={importBatch.phase?.bytes ?? undefined} />
            </div>}
            {!!importErrors.length && <div className="alert stack" role="alert">
              {importErrors.map((message, index) => <p key={index}>{message}</p>)}
            </div>}
            {project && <dl><dt>文件</dt><dd>{project.name}</dd><dt>时长</dt><dd>{duration(project.input.durationMs)}</dd></dl>}
            {project && <button disabled={busy} onClick={() => void action(async () => {
              await api.copyMedia(project.id); await loadProjects();
              setMediaPath(await api.playback(project.id)); setMediaError(''); setNotice('已复制原素材到本地项目');
            })}>复制素材到项目</button>}
            {!attached && <button disabled={busy || connecting} onClick={connect}>连接比格熊</button>}
          </section>
          <section className="panel stack"><h2>转写设置</h2>
            <label>音轨<select value={track} disabled={!project} onChange={e => setTrack(Number(e.target.value))}>
              {project?.input.audioTracks.map(a => <option key={a.index} value={a.index}>音轨 {a.index + 1} · {a.codec}</option>)}
            </select></label>
            <label>识别模型<select value={modelId} onChange={e => setModelId(e.target.value)}>
              <option value="">选择模型</option>{capabilities?.models.map(m => <option key={m.id} value={m.id}>{m.name}{!m.installed || m.issues.length ? ' · 不可用' : ''}</option>)}
            </select></label>
            {!!chosen?.issues.length && <p role="status">{chosen.issues.map(issue => modelIssues[issue] ?? issue).join('、')}</p>}
            {needsAsrSetup && <div className="stack">
              <p role="status">请在比格熊的「设置」中安装或导入语音识别资源，并检查「本地模型 → 语音识别」。装好后刷新模型列表。</p>
              <button disabled={busy || connecting} onClick={connect}>刷新模型列表</button>
            </div>}
            <label>语言<select value={language} onChange={e => setLanguage(e.target.value)}>
              {[...new Set(['zh', ...(chosen?.languages ?? [])])].map(lang => <option key={lang} value={lang}>{lang === 'zh' ? '简体中文' : lang}</option>)}
            </select></label>
            <button className="primary" disabled={busy || !canSubmit} onClick={() => void action(async () => {
              try { await api.submit(projectId, modelId, track, language); } finally { await loadJobs(); setPage(1); }
            })}>开始转写</button>
          </section>
        </div>}
        {page === 1 && <section className="panel stack" aria-label="任务">
          {!jobs.length && <p className="muted">暂无任务</p>}
          {jobs.map(job => {
            const id = job.request.clientRequestId;
            const status = job.remote?.status ?? 'unknown';
            const online = attached && capabilities?.serviceId === job.serviceId;
            const view = job.document ? '已保存到本工具' : !online && !terminal.has(status) ? '状态待核对' : statuses[status] ?? '受理状态待核对';
            const update = (op: string) => action(async () => { await api.control(id, op); await loadJobs(); });
            return <article key={id} className="job stack"><div className="row spread"><strong>{projects.find(item => item.id === job.projectId)?.name ?? '本地项目'}</strong><span>{job.request.operation.kind === 'organize' ? '整理' : '转写'} · {view}</span></div>
              {job.remote && <div className="row"><progress aria-label="服务已提交片段" max={Math.max(1, job.remote.durationMs)} value={job.remote.committedMs} /><span>{duration(job.remote.committedMs)} / {duration(job.remote.durationMs)}</span></div>}
              {job.remote?.error && <p className="muted">{job.remote.error.message}</p>}
              <div className="row">
                {!job.remote && <button disabled={busy || !online} onClick={() => void action(async () => { await api.refresh(id); await loadJobs(); })}>核对受理</button>}
                {!job.remote && <button disabled={busy || !online} onClick={() => void action(async () => { await api.retryPending(id); await loadJobs(); })}>核对并重试原请求</button>}
                {['queued', 'running'].includes(status) && <button disabled={busy || !online || !capabilities?.pause} onClick={() => void update('pause')}>暂停</button>}
                {['queued', 'running', 'paused', 'pausing'].includes(status) && <button disabled={busy || !online} onClick={() => void update('cancel')}>取消</button>}
                {['paused', 'cancelled', 'failed'].includes(status) && job.remote?.error?.code !== 'modelChanged' && <button disabled={busy || !online || !capabilities?.resume} onClick={() => void update('resume')}>继续未完成部分</button>}
                {job.remote?.error?.code === 'modelChanged' && <button disabled={busy || dirty || draftDirty} onClick={() => {
                  switchProject(job.projectId); setPage(0);
                }}>打开素材重新转写</button>}
                {status === 'resultReady' && !job.document && <button className="primary" disabled={busy || !online || dirty || draftDirty} onClick={() => void action(async () => {
                  const updated = await api.retrieve(id); await loadProjects(); await loadJobs(); selectProject(updated.id);
                  if (job.request.operation.kind === 'organize') { await reloadCandidates(updated.id); setPage(3); }
                  else setPage(2);
                })}>取回并保存</button>}
                {terminal.has(status) && <Btn variant="danger" size="compact" disabled={busy || dirty || draftDirty}
                  onClick={() => setJobToDelete(job)}>清理任务</Btn>}
                {job.document && <button disabled={busy || dirty || draftDirty} onClick={() => void action(async () => {
                  const owner = projects.find(item => item.id === job.projectId);
                  if (job.request.operation.kind === 'transcribe' && job.document && owner?.original && owner.original !== job.document) {
                    await api.checkout(owner.id, job.document, owner.correction ?? owner.original); await loadProjects();
                  }
                  switchProject(job.projectId); setPage(job.request.operation.kind === 'organize' ? 3 : 2);
                })}>打开本地成果</button>}
              </div></article>;
          })}
        </section>}
        {(page === 2 || page === 4) && timingReview && <p className="notice" role="status">{timingReview.message}</p>}
        {page === 2 && (!doc || !draft ? <p className="muted">请打开已取回原文的项目。</p> :
          <SubtitleReviewWorkspace key={projectId + ':' + doc.original} draft={draft} doc={doc} rawDoc={rawDoc}
            activeId={activeId} revealId={revealId} select={seek} change={changeDraft} undo={undoDraft} canUndo={history.current.length > 0}
            dirty={dirty} busy={busy} revisions={revisions} checkout={checkoutSubtitle} save={() => void action(async () => { await persistSubtitle(); })}
            organize={() => setPage(3)} exportPage={() => setPage(4)} play={playReview} stop={stopReviewPlayback} seekPlayback={seekWithinReview} positionMs={positionMs}
            bindings={bindings} reportError={reason => setError(String(reason))} media={<>
              <div className="row"><Btn variant="ghost" disabled={busy} onClick={() => void action(async () => {
                const path = await open({ multiple: false, filters: mediaFilters });
                if (typeof path === 'string') {
                  stopReviewPlayback(); nextSeek.current = undefined;
                  await api.relocate(projectId, path); await loadProjects();
                  setMediaPath(await api.playback(projectId)); setMediaError('');
                }
              })}>重新定位素材</Btn></div>
            {mediaSource && (/\.(wav|mp3|m4a|aac|flac|ogg)$/i.test(mediaPath)
              ? <audio ref={mediaRef} controls src={mediaSource} onLoadedMetadata={mediaReady} onTimeUpdate={trackPosition} onError={() => setMediaError('当前编码无法直接播放')} />
              : <video ref={mediaRef} controls src={mediaSource} onLoadedMetadata={mediaReady} onTimeUpdate={trackPosition} onError={() => setMediaError('当前编码无法直接播放')} />)}
            {!!peakBars.length && project && <div className="row wave-row">
              <div className="waveform">
                <div className="wave-bars" aria-hidden="true">{peakBars.map((value,index) => <i key={index} style={{ height:`${Math.max(3, value * 100)}%` }} />)}</div>
                <UiRangeSlider aria-label="波形定位" min={0} max={project.input.durationMs} step={10}
                  value={positionMs} onChange={ms => {
                    const segment = draft.entries.findLast(item => (entryRange(draft, item)?.startMs ?? Infinity) <= ms);
                    if (segment) seek(segment.id, ms);
                  }} />
              </div>
              <Btn variant="ghost" aria-pressed={waveformDetail} onClick={() => setWaveformDetail(!waveformDetail)}>波形细节</Btn>
            </div>}
            {mediaError && <p className="muted">{mediaError}</p>}
            </>} />)}
        {page === 3 && (!doc ? <p className="muted">请先取回转写结果。</p> : <div className="columns editor">
          <section className="panel stack"><h2>候选稿</h2>
            <select aria-label="候选稿" value={candidateId} onChange={e => { if (draftDirty) setError('请先保存稿件编辑。'); else setCandidateId(e.target.value); }}>
              <option value="">选择候选稿</option>{candidates.map(c => <option key={c.id} value={c.id}>{titleFor(c.draft.draftType)} · {c.draft.sourceRevision !== doc.revision ? '来源待核对' : c.accepted ? '已接受' : '候选'}</option>)}
            </select>
            {candidate?.draft.blocks.map((block, index) => <div className="stack draft-block" key={`${candidate.id}-${index}`}>
              <label>{block.addedOrInferred ? '新增 / 推测' : '稿件段落'}<textarea rows={4} value={draftChanges[index] ?? block.text} onChange={e => {
                const text = e.target.value;
                setDraftChanges(before => { const next = { ...before }; if (text === block.text) delete next[index]; else next[index] = text; return next; });
              }} /></label>
              <div className="row">{currentCandidate && block.sourceSegmentIds.map(id => { const source = doc.segments.find(s => s.id === id); return source && <button key={id} onClick={() => { seek(id, source.startMs); setRevealId(id); setPage(2); }}>回听 {duration(source.startMs)}</button>; })}</div>
            </div>)}
            {candidate && <div className="row"><button disabled={busy || !draftDirty} onClick={() => void action(async () => {
              await api.saveCandidate(candidate.id, candidate.revision, candidate.draft.blocks.map((block, i) => draftChanges[i] ?? block.text));
              setDraftChanges({}); await reloadCandidates(projectId);
            })}>保存编辑</button><button className="primary" disabled={busy || dirty || draftDirty || !currentCandidate || candidate.accepted} onClick={() => void action(async () => {
              await api.acceptCandidate(candidate.id, candidate.revision); await reloadCandidates(projectId);
            })}>{candidate.accepted && !draftDirty ? '已接受' : '接受稿件'}</button></div>}
          </section>
          <section className="panel stack"><h2>整理设置</h2>
            <label>类型<select value={draftType} onChange={e => setDraftType(e.target.value)}><option value="spokenScript">口播稿</option><option value="literaryScript">文学剧本</option></select></label>
            <label>方式<select value={mode} onChange={e => setMode(e.target.value)}><option value="faithful">忠实整理</option><option value="creative">创作改编</option></select></label>
            {attached && !capabilities?.operations.includes('organize') && <p className="muted">比格熊整理服务尚未就绪，请检查默认文本模型。</p>}
            <button className="primary" disabled={busy || dirty || draftDirty || !!draft?.reviewGroups.length || !serviceMatches || !capabilities?.operations.includes('organize') || !doc.segments.length} onClick={() => void action(async () => {
              try { await api.organize(projectId, doc.revision, draftType, mode); } finally { await loadJobs(); setPage(1); }
            })}>生成候选稿</button>
          </section>
        </div>)}
        {page === 4 && (!doc ? <p className="muted">请打开已取回原文的项目。</p> : <div className="columns">
          <section className="panel stack"><h2>导出设置</h2>
            <label>来源<select value={exportSource} onChange={e => { setExportSource(e.target.value); setFormat(e.target.value === 'candidate' ? 'txt' : 'srt'); }}>
              <option value="correction">校订字幕</option><option value="original">只读识别原文</option><option value="candidate">已接受稿件</option>
            </select></label>
            {exportSource === 'candidate' && <label>稿件<select value={candidateId} onChange={e => { if (!draftDirty) setCandidateId(e.target.value); }}>
              <option value="">选择稿件</option>{candidates.filter(c => c.accepted && c.draft.sourceRevision === doc.revision).map(c => <option key={c.id} value={c.id}>{titleFor(c.draft.draftType)}</option>)}
            </select></label>}
            <label>格式<select value={format} onChange={e => setFormat(e.target.value)}>{(exportSource === 'candidate' ? ['txt', 'md'] : ['srt', 'vtt', 'txt', 'json']).map(value => <option key={value} value={value}>{value.toUpperCase()}</option>)}</select></label>
            {exportSource !== 'candidate' && <label>范围<select value={selectionOnly ? 'current' : 'all'} onChange={e => setSelectionOnly(e.target.value === 'current')}>
              <option value="all">全部字幕</option><option value="current" disabled={!active}>当前字幕所在时间范围</option>
            </select></label>}
            <button className="primary" disabled={busy || (exportSource !== 'original' && dirty) || (exportSource !== 'candidate' && selectionOnly && !range) || (exportSource === 'candidate' && (!candidate?.accepted || !currentCandidate || draftDirty))} onClick={() => void action(async () => {
              const path = await save({ defaultPath: `${project?.name.replace(/\.[^.]+$/, '') ?? 'bgxiong-any2text'}.${format}`, filters: [{ name: format.toUpperCase(), extensions: [format] }] });
              if (!path) return;
              const saved = exportSource === 'candidate' && candidate ? await api.exportCandidate(candidate.id, format, path) : await api.export(projectId, exportSource === 'original', format, path, range, expectedExport);
              setNotice(`已导出 ${saved}`);
            })}>保存到文件</button>
          </section><section className="panel stack"><h2>内容预览</h2><pre>{preview}</pre></section>
        </div>)}
        {jobToDelete && <NativeDialog title="清理任务" onClose={() => setJobToDelete(undefined)} className="subtitle-dialog">
          <p>{jobToDelete.request.operation.kind === 'transcribe'
            ? '将删除该任务及关联的本地字幕、校订和整理稿。原始素材与手动导出的文件会保留。'
            : '将删除该任务及其本地整理稿。来源字幕、原始素材与手动导出的文件会保留。'}</p>
          <div className="row">
            <Btn variant="ghost" onClick={() => setJobToDelete(undefined)}>取消</Btn>
            <Btn variant="danger" disabled={busy} onClick={() => {
              const job = jobToDelete;
              setJobToDelete(undefined);
              void action(async () => {
                try { await api.deleteJob(job.request.clientRequestId); }
                finally {
                  await Promise.all([loadJobs(), loadProjects(), projectId ? reloadCandidates(projectId) : Promise.resolve()]);
                }
                setNotice('任务及本地生成文件已清理');
              });
            }}>确认清理</Btn>
          </div>
        </NativeDialog>}
        {pendingDeparture && <NativeDialog title="保存当前编辑" onClose={() => setPendingDeparture(undefined)} className="subtitle-dialog">
          <div className="row">
            <Btn disabled={busy} onClick={() => void action(async () => {
              const savedRevision = await persistSubtitle();
              if (draftDirty && candidate) {
                const saved = await api.saveCandidate(candidate.id, candidate.revision, candidate.draft.blocks.map((block, index) => draftChanges[index] ?? block.text));
                setCandidates(rows => rows.map(row => row.id === saved.id ? saved : row));
                setDraftChanges({});
              }
              setPendingDeparture(undefined); history.current = [];
              window.setTimeout(() => pendingDeparture(savedRevision), 0);
            })}>保存后继续</Btn>
            <Btn variant="danger" disabled={busy} onClick={() => {
              setEdited(undefined); setDraftChanges({}); history.current = [];
              setPendingDeparture(undefined); pendingDeparture();
            }}>放弃修改</Btn>
            <Btn variant="ghost" onClick={() => setPendingDeparture(undefined)}>返回编辑</Btn>
          </div>
        </NativeDialog>}
      </main>
    </div>
  </div></>;
}

createRoot(document.getElementById('root')!).render(<App />);
