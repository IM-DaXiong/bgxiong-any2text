use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde_json::json;
use tauri::{AppHandle, Manager, State};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::provider::{Capabilities, ImportProgress, Request, ServiceProvider};
use crate::workspace::{Candidate, Document, DocumentRevision, LocalJob, Project, Workspace};

pub struct Application {
    pub workspace: Workspace,
    provider: Arc<dyn ServiceProvider>,
    connected: AtomicBool,
    job_operations: Mutex<()>,
}

impl Application {
    pub fn new(workspace: Workspace, provider: Arc<dyn ServiceProvider>) -> Self {
        Self {
            workspace,
            provider,
            connected: AtomicBool::new(false),
            job_operations: Mutex::new(()),
        }
    }

    fn service(&self) -> Result<&dyn ServiceProvider, String> {
        if self.connected.load(Ordering::Acquire) {
            Ok(self.provider.as_ref())
        } else {
            Err("请先连接识别服务".into())
        }
    }

    async fn service_for(
        &self,
        provider_id: &str,
        service_id: &str,
    ) -> Result<&dyn ServiceProvider, String> {
        let service = self.service()?;
        let caps = service.connect().await?;
        if caps.provider_id != provider_id || caps.service_id != service_id {
            return Err("请连接该任务原来的提供方和服务实例".into());
        }
        Ok(service)
    }
}

#[tauri::command]
pub async fn connect(app: State<'_, Application>) -> Result<Capabilities, String> {
    app.provider.disconnect().await;
    let result = app.provider.connect().await;
    app.connected.store(result.is_ok(), Ordering::Release);
    result
}

#[tauri::command]
pub async fn disconnect(app: State<'_, Application>) -> Result<(), String> {
    app.connected.store(false, Ordering::Release);
    app.provider.disconnect().await;
    Ok(())
}

#[tauri::command]
pub async fn provider_health(app: State<'_, Application>) -> Result<(), String> {
    app.service()?.health().await
}

#[tauri::command]
pub async fn provider_appearance(app: State<'_, Application>) -> Result<Option<String>, String> {
    app.service()?.appearance().await
}

#[tauri::command]
pub async fn launch_bigbear(executable: Option<PathBuf>) -> Result<bool, String> {
    let executable = match executable {
        Some(path) => path,
        None => {
            let running = std::env::current_exe().map_err(|e| e.to_string())?;
            let folder = running.parent().ok_or("无效的安装目录")?;
            let Some(host) = ["bgxiong-ai-story.exe", "bgxiong-ai-story-next.exe"]
                .into_iter()
                .map(|name| folder.join(name))
                .find(|file| file.is_file())
            else {
                return Ok(false);
            };
            host
        }
    };
    if !executable.is_absolute()
        || !executable
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                matches!(
                    name.to_ascii_lowercase().as_str(),
                    "bgxiong-ai-story.exe" | "bgxiong-ai-story-next.exe"
                )
            })
    {
        return Err("请选择比格熊安装目录中的主程序".into());
    }
    let folder = executable.parent().ok_or("无效的比格熊安装目录")?;
    std::process::Command::new(&executable)
        .current_dir(folder)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(true)
}

#[tauri::command]
pub fn projects(app: State<'_, Application>) -> Result<Vec<Project>, String> {
    app.workspace.projects()
}

#[tauri::command]
pub async fn import_media(
    app: State<'_, Application>,
    path: PathBuf,
    progress: tauri::ipc::Channel<ImportProgress>,
) -> Result<Project, String> {
    if !path.is_absolute() {
        return Err("请选择本地素材".into());
    }
    let service = app.service()?;
    let caps = service.connect().await?;
    let started = std::time::Instant::now();
    let input = service.prepare_input(&caps.service_id, &path, Box::new(move |update| {
        let _ = progress.send(update);
    })).await?;
    let project = app.workspace.add_project(path, caps.provider_id, caps.service_id, input)?;
    eprintln!("Any2Text: media import registered in {} ms", started.elapsed().as_millis());
    Ok(project)
}

#[tauri::command]
pub fn playback_path(
    app: State<'_, Application>,
    handle: AppHandle,
    project_id: Uuid,
) -> Result<PathBuf, String> {
    let path = app.workspace.project(project_id)?.media_path;
    let canonical = path
        .canonicalize()
        .map_err(|_| "找不到原素材，请重新定位".to_string())?;
    handle
        .asset_protocol_scope()
        .allow_file(&canonical)
        .map_err(|e| e.to_string())?;
    Ok(path)
}

#[tauri::command]
pub async fn relocate_media(
    handle: AppHandle,
    project_id: Uuid,
    path: PathBuf,
) -> Result<Project, String> {
    tauri::async_runtime::spawn_blocking(move || {
        handle
            .state::<Application>()
            .workspace
            .relocate(project_id, path)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn copy_media(handle: AppHandle, project_id: Uuid) -> Result<Project, String> {
    tauri::async_runtime::spawn_blocking(move || {
        handle
            .state::<Application>()
            .workspace
            .copy_media(project_id)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub fn jobs(app: State<'_, Application>) -> Result<Vec<LocalJob>, String> {
    app.workspace.jobs()
}

#[tauri::command]
pub async fn submit_transcription(
    app: State<'_, Application>,
    project_id: Uuid,
    model_config_id: String,
    audio_track: u32,
    language: String,
) -> Result<LocalJob, String> {
    let service = app.service()?;
    let project = app.workspace.project(project_id)?;
    let caps = service.connect().await?;
    if caps.service_id != project.service_id || caps.provider_id != project.provider_id {
        return Err("请连接该素材所属的服务实例".into());
    }
    let eligible = caps.models.iter().any(|model| {
        model.id == model_config_id
            && model.installed
            && model.issues.is_empty()
            && model.timings.iter().any(|t| t == "segmentTimed")
            && model
                .languages
                .iter()
                .any(|supported| supported == &language)
    });
    if !eligible
        || !project
            .input
            .audio_tracks
            .iter()
            .any(|track| track.index == audio_track)
    {
        return Err("请选择可用的模型、语言和音轨".into());
    }
    let request = Request {
        client_request_id: Uuid::new_v4(),
        operation: json!({"kind":"transcribe","inputRef":project.input.input_ref,
        "inputFingerprint":project.input.fingerprint,"modelConfigId":model_config_id,"audioTrack":audio_track,"language":language,"timing":"segmentTimed"}),
    };
    let _guard = app.job_operations.lock().await;
    app.workspace.add_job(&project, request.clone(), None)?;
    let result = service
        .submit(&project.service_id, &request)
        .await
        .map_err(|detail| format!("受理状态待核对。{detail}"))?;
    app.workspace.update_job(request.client_request_id, result)
}

#[tauri::command]
pub async fn refresh_job(
    app: State<'_, Application>,
    request_id: Uuid,
) -> Result<LocalJob, String> {
    let _guard = app.job_operations.lock().await;
    let job = app.workspace.job(request_id)?;
    let service = app.service_for(&job.provider_id, &job.service_id).await?;
    let remote = if let Some(ref remote) = job.remote {
        Some(service.get_job(&job.service_id, &remote.task_id).await?)
    } else {
        service
            .get_job_by_request(&job.service_id, request_id)
            .await?
    };
    match remote {
        Some(remote) => app.workspace.update_job(request_id, remote),
        None => Ok(job),
    }
}

#[tauri::command]
pub async fn retry_pending_request(app: State<'_, Application>, request_id: Uuid) -> Result<LocalJob, String> {
    let _guard = app.job_operations.lock().await;
    let job = app.workspace.job(request_id)?;
    if job.remote.is_some() { return Ok(job) }
    let service = app.service_for(&job.provider_id, &job.service_id).await?;
    let remote = match service.get_job_by_request(&job.service_id, request_id).await? {
        Some(remote) => remote,
        None => service.submit(&job.service_id, &job.request).await?,
    };
    app.workspace.update_job(request_id, remote)
}

#[tauri::command]
pub async fn control_job(
    app: State<'_, Application>,
    request_id: Uuid,
    action: String,
) -> Result<LocalJob, String> {
    let _guard = app.job_operations.lock().await;
    let job = app.workspace.job(request_id)?;
    let service = app.service_for(&job.provider_id, &job.service_id).await?;
    let remote = job.remote.ok_or("请先核对任务受理状态")?;
    let current = service
        .control(&job.service_id, &remote.task_id, remote.attempt_id, &action)
        .await?;
    app.workspace.update_job(request_id, current)
}

#[tauri::command]
pub async fn retrieve(app: State<'_, Application>, request_id: Uuid) -> Result<Project, String> {
    let _guard = app.job_operations.lock().await;
    let job = app.workspace.job(request_id)?;
    let service = app.service_for(&job.provider_id, &job.service_id).await?;
    let remote = job.remote.ok_or("请先核对任务受理状态")?;
    let receipt = remote.result.ok_or("任务结果尚不可取回")?;
    if remote.status != "resultReady" || receipt.bytes == 0 || receipt.bytes > 64 * 1024 * 1024 {
        return Err("任务结果尚不可取回或超出大小限制".into());
    }
    let temp = tempfile::NamedTempFile::new_in(app.workspace.directory(job.project_id))
        .map_err(|e| e.to_string())?;
    service
        .read_result(&job.service_id, &remote.task_id, &receipt, temp.path())
        .await?;
    let project = app.workspace.import_result(request_id, temp)?;
    if let Err(error) = service
        .acknowledge(&job.service_id, &remote.task_id, &receipt.sha256)
        .await
    {
        eprintln!("Any2Text: local result saved; confirmation pending: {error}");
    }
    Ok(project)
}

#[tauri::command]
pub fn document(
    app: State<'_, Application>,
    project_id: Uuid,
    original: bool,
) -> Result<Document, String> {
    app.workspace.document(project_id, original)
}

#[tauri::command]
pub fn revisions(
    app: State<'_, Application>,
    project_id: Uuid,
) -> Result<Vec<DocumentRevision>, String> {
    app.workspace.revisions(project_id)
}

#[tauri::command]
pub fn checkout_revision(
    app: State<'_, Application>,
    project_id: Uuid,
    revision: Uuid,
    expected_revision: Uuid,
) -> Result<Project, String> {
    app.workspace
        .checkout(project_id, revision, expected_revision)
}

#[tauri::command]
pub fn save_correction(
    app: State<'_, Application>,
    project_id: Uuid,
    expected_revision: Uuid,
    changes: HashMap<String, String>,
) -> Result<Document, String> {
    app.workspace
        .save_correction(project_id, expected_revision, changes)
}

#[tauri::command]
pub fn preview_export(
    app: State<'_, Application>,
    project_id: Uuid,
    original: bool,
    format: String,
    range: Option<crate::export::TimeRange>,
) -> Result<String, String> {
    crate::export::render(&app.workspace, project_id, original, &format, range)
}

#[tauri::command]
pub fn export_document(
    app: State<'_, Application>,
    project_id: Uuid,
    original: bool,
    format: String,
    path: PathBuf,
    range: Option<crate::export::TimeRange>,
) -> Result<PathBuf, String> {
    let text = crate::export::render(&app.workspace, project_id, original, &format, range)?;
    crate::export::save(&path, &format, &text)?;
    Ok(path)
}

#[tauri::command]
pub async fn submit_organize(
    app: State<'_, Application>,
    project_id: Uuid,
    expected_revision: Uuid,
    draft_type: String,
    mode: String,
) -> Result<LocalJob, String> {
    let service = app.service()?;
    let project = app.workspace.project(project_id)?;
    let caps = service.connect().await?;
    if caps.service_id != project.service_id
        || caps.provider_id != project.provider_id
        || !caps.operations.iter().any(|op| op == "organize")
    {
        return Err("当前服务未提供该项目的整理能力".into());
    }
    if !matches!(draft_type.as_str(), "spokenScript" | "literaryScript")
        || !matches!(mode.as_str(), "faithful" | "creative")
    {
        return Err("无效的稿件类型或整理方式".into());
    }
    let document = app.workspace.document(project_id, false)?;
    if document.revision != expected_revision || document.segments.is_empty() {
        return Err("请先保存当前版本的校订".into());
    }
    let snapshot = serde_json::to_vec(
        &json!({"sourceRevision":document.revision,"segments":document.segments}),
    )
    .map_err(|e| e.to_string())?;
    let input = service.prepare_text(&project.service_id, snapshot).await?;
    let request = Request {
        client_request_id: Uuid::new_v4(),
        operation: json!({"kind":"organize","inputRef":input.input_ref,
        "inputFingerprint":input.fingerprint,"draftType":draft_type,"mode":mode}),
    };
    let _guard = app.job_operations.lock().await;
    app.workspace
        .add_job(&project, request.clone(), Some(expected_revision))?;
    let result = service
        .submit(&project.service_id, &request)
        .await
        .map_err(|detail| format!("受理状态待核对。{detail}"))?;
    app.workspace.update_job(request.client_request_id, result)
}

#[tauri::command]
pub fn candidates(app: State<'_, Application>, project_id: Uuid) -> Result<Vec<Candidate>, String> {
    app.workspace.candidates(project_id)
}

#[tauri::command]
pub fn save_candidate(
    app: State<'_, Application>,
    id: Uuid,
    expected_revision: Uuid,
    blocks: Vec<String>,
) -> Result<Candidate, String> {
    app.workspace.save_candidate(id, expected_revision, blocks)
}

#[tauri::command]
pub fn accept_candidate(
    app: State<'_, Application>,
    id: Uuid,
    expected_revision: Uuid,
) -> Result<Candidate, String> {
    app.workspace.accept_candidate(id, expected_revision)
}

#[tauri::command]
pub fn preview_candidate(
    app: State<'_, Application>,
    id: Uuid,
    format: String,
) -> Result<String, String> {
    app.workspace.export_candidate(id, &format)
}

#[tauri::command]
pub fn export_candidate(
    app: State<'_, Application>,
    id: Uuid,
    format: String,
    path: PathBuf,
) -> Result<PathBuf, String> {
    let text = app.workspace.export_candidate(id, &format)?;
    crate::export::save(&path, &format, &text)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::path::Path;
    use std::process::{Child, Command, Stdio};

    use crate::provider::{AudioTrack, Input, Job, Receipt, TextInput};
    use sha2::{Digest, Sha256};

    struct ChildService(Child);

    impl Drop for ChildService {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    struct Replacement {
        endpoint: String,
        _child: ChildService,
    }

    impl Replacement {
        fn start() -> Self {
            let code = r#"
import http.server, json
class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path != '/capabilities':
            self.send_error(403)
            return
        body = json.dumps(dict(protocolMin=1, protocolMax=1, schemaSha256='',
            providerId='replacement', endpointInstance='fixture', serviceId='shared-service-id',
            models=[], operations=[], inputMode='localStream', maxInputBytes=1, maxDurationMs=1,
            pause=False, resume=False)).encode()
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)
server = http.server.HTTPServer(('127.0.0.1', 0), Handler)
print(server.server_port, flush=True)
server.serve_forever()
"#;
            let mut child = ChildService(
                Command::new("python")
                    .args(["-u", "-c", code])
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .spawn()
                    .expect("launch process-external fixture"),
            );
            let mut port = String::new();
            BufReader::new(child.0.stdout.take().unwrap())
                .read_line(&mut port)
                .unwrap();
            let port: u16 = port.trim().parse().unwrap();
            Self {
                endpoint: format!("http://127.0.0.1:{port}/capabilities"),
                _child: child,
            }
        }
    }

    #[async_trait::async_trait]
    impl ServiceProvider for Replacement {
        async fn connect(&self) -> Result<Capabilities, String> {
            reqwest::Client::builder()
                .no_proxy()
                .build()
                .map_err(|e| e.to_string())?
                .get(&self.endpoint)
                .send()
                .await
                .map_err(|e| e.to_string())?
                .json()
                .await
                .map_err(|e| e.to_string())
        }
        async fn health(&self) -> Result<(), String> {
            self.connect().await.map(|_| ())
        }
        async fn prepare_input(&self, _: &str, _: &Path, _: crate::provider::ImportReporter) -> Result<Input, String> {
            panic!("unexpected media upload")
        }
        async fn prepare_text(&self, _: &str, _: Vec<u8>) -> Result<TextInput, String> {
            panic!("unexpected text upload")
        }
        async fn submit(&self, _: &str, _: &Request) -> Result<Job, String> {
            panic!("unexpected job submission")
        }
        async fn get_job(&self, _: &str, _: &str) -> Result<Job, String> {
            panic!("old job forwarded to new provider")
        }
        async fn get_job_by_request(&self, _: &str, _: Uuid) -> Result<Option<Job>, String> {
            panic!("old request forwarded to new provider")
        }
        async fn control(&self, _: &str, _: &str, _: Uuid, _: &str) -> Result<Job, String> {
            panic!("old attempt forwarded to new provider")
        }
        async fn read_result(&self, _: &str, _: &str, _: &Receipt, _: &Path) -> Result<(), String> {
            panic!("old result forwarded to new provider")
        }
        async fn acknowledge(&self, _: &str, _: &str, _: &str) -> Result<(), String> {
            panic!("old result acknowledged on new provider")
        }
        async fn disconnect(&self) {}
    }

    #[test]
    fn external_replacement_cannot_receive_old_jobs_and_documents_survive() {
        tauri::async_runtime::block_on(async {
            let root = tempfile::tempdir().unwrap();
            let workspace = Workspace::open(root.path().to_owned()).unwrap();
            let fingerprint = "a".repeat(64);
            let project = workspace
                .add_project(
                    root.path().join("source.wav"),
                    "bigbear".into(),
                    "shared-service-id".into(),
                    Input {
                        input_ref: fingerprint.clone(),
                        fingerprint: fingerprint.clone(),
                        duration_ms: 1000,
                        audio_tracks: vec![AudioTrack {
                            index: 0,
                            codec: "pcm_s16le".into(),
                        }],
                    },
                )
                .unwrap();
            let request_id = Uuid::new_v4();
            workspace
                .add_job(
                    &project,
                    Request {
                        client_request_id: request_id,
                        operation: json!({"kind":"transcribe"}),
                    },
                    None,
                )
                .unwrap();
            let transcript = serde_json::to_vec(&json!({"schemaVersion":1,"sourceFingerprint":fingerprint,"durationMs":1000,
                "timeOrigin":"sourceMedia","timingQuality":"segmentTimed","provenance":{"providerId":"bigbear","modelConfigId":"test"},
                "segments":[{"id":"s1","startMs":0,"endMs":1000,"text":"原文"}]})).unwrap();
            let attempt = Uuid::new_v4();
            workspace
                .update_job(
                    request_id,
                    Job {
                        task_id: "remote-task".into(),
                        attempt_id: attempt,
                        client_request_id: request_id,
                        status: "resultReady".into(),
                        committed_ms: 1000,
                        duration_ms: 1000,
                        error: None,
                        result: Some(Receipt {
                            bytes: transcript.len() as u64,
                            sha256: format!("{:x}", Sha256::digest(&transcript)),
                            retained_until: None,
                        }),
                    },
                )
                .unwrap();
            let mut file =
                tempfile::NamedTempFile::new_in(workspace.directory(project.id)).unwrap();
            file.write_all(&transcript).unwrap();
            file.as_file().sync_all().unwrap();
            let saved = workspace.import_result(request_id, file).unwrap();
            let original = saved.original.unwrap();
            assert_ne!(original, attempt);
            let edited = workspace
                .save_correction(
                    project.id,
                    original,
                    HashMap::from([("s1".into(), "校订".into())]),
                )
                .unwrap();
            assert!(workspace
                .save_correction(project.id, original, HashMap::new())
                .is_err());

            let app = Application::new(workspace, Arc::new(Replacement::start()));
            assert_eq!(
                app.provider.connect().await.unwrap().provider_id,
                "replacement"
            );
            app.connected.store(true, Ordering::Release);
            assert!(app
                .service_for("bigbear", "shared-service-id")
                .await
                .is_err());
            assert!(app
                .service_for("replacement", "shared-service-id")
                .await
                .is_ok());
            app.provider.disconnect().await;
            app.connected.store(false, Ordering::Release);
            assert_eq!(
                app.workspace.document(project.id, false).unwrap().revision,
                edited.revision
            );
            assert_eq!(
                crate::export::render(&app.workspace, project.id, false, "txt", None).unwrap(),
                "校订"
            );
            assert_eq!(
                crate::export::render(&app.workspace, project.id, true, "txt", None).unwrap(),
                "原文"
            );
        });
    }
}
