use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::provider::{Input, Job, Request, Segment, Transcript};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: Uuid,
    pub media_path: PathBuf,
    pub name: String,
    pub provider_id: String,
    pub service_id: String,
    pub input: Input,
    pub original: Option<Uuid>,
    pub correction: Option<Uuid>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalJob {
    pub project_id: Uuid,
    pub provider_id: String,
    pub service_id: String,
    pub request: Request,
    pub remote: Option<Job>,
    pub document: Option<Uuid>,
    #[serde(default)]
    pub source_revision: Option<Uuid>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Document {
    pub revision: Uuid,
    pub original: Uuid,
    pub segments: Vec<Segment>,
    pub waveform_peaks: Vec<f32>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentRevision {
    pub id: Uuid,
    pub kind: String,
    pub parent: Option<String>,
    pub source: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftBlock {
    pub text: String,
    pub source_segment_ids: Vec<String>,
    pub added_or_inferred: bool,
    pub abridged: bool,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Draft {
    pub schema_version: u32,
    pub source_revision: Uuid,
    pub draft_type: String,
    pub mode: String,
    pub blocks: Vec<DraftBlock>,
    pub abridged_source_segment_ids: Vec<String>,
    pub complete: bool,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub id: Uuid,
    pub revision: Uuid,
    pub project_id: Uuid,
    pub accepted: bool,
    pub draft: Draft,
}

pub struct Workspace {
    pub root: PathBuf,
    db: Mutex<Connection>,
}

type AppResult<T> = Result<T, String>;

fn encode(value: &impl Serialize) -> AppResult<String> {
    serde_json::to_string(value).map_err(|e| e.to_string())
}

fn decode<T: DeserializeOwned>(raw: &str) -> AppResult<T> {
    serde_json::from_str(raw).map_err(|e| e.to_string())
}

fn read_file<T: DeserializeOwned>(path: &Path) -> AppResult<T> {
    let file = std::fs::File::open(path).map_err(|_| "本地成果文件缺失".to_string())?;
    serde_json::from_reader(std::io::BufReader::new(file)).map_err(|_| "本地成果文件损坏".into())
}

fn verify_file(path: &Path, expected_bytes: u64, expected_sha256: &str) -> AppResult<()> {
    let mut file = std::fs::File::open(path).map_err(|_| "成果文件缺失".to_string())?;
    let mut hash = Sha256::new();
    let size = std::io::copy(&mut file, &mut hash).map_err(|e| e.to_string())?;
    if size != expected_bytes || format!("{:x}", hash.finalize()) != expected_sha256 {
        return Err("成果文件摘要校验失败".into());
    }
    Ok(())
}

fn local_revision(project: Uuid, request: Uuid, attempt: Uuid) -> Uuid {
    let mut hash = Sha256::new();
    hash.update(b"any2text-local-revision-v1");
    hash.update(project.as_bytes());
    hash.update(request.as_bytes());
    hash.update(attempt.as_bytes());
    let mut bytes = [0; 16];
    bytes.copy_from_slice(&hash.finalize()[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> AppResult<()> {
    let dir = path.parent().ok_or("无效的保存路径")?;
    let mut file = tempfile::NamedTempFile::new_in(dir).map_err(|e| e.to_string())?;
    file.write_all(bytes).map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    file.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}

impl Workspace {
    pub fn open(root: PathBuf) -> AppResult<Self> {
        std::fs::create_dir_all(root.join("projects")).map_err(|e| e.to_string())?;
        let db = Connection::open(root.join("workspace.sqlite3")).map_err(|e| e.to_string())?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;
            CREATE TABLE IF NOT EXISTS projects (id TEXT PRIMARY KEY, body TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS jobs (id TEXT PRIMARY KEY, project TEXT NOT NULL REFERENCES projects(id), body TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS revisions (id TEXT PRIMARY KEY, project TEXT NOT NULL REFERENCES projects(id), kind TEXT NOT NULL, parent TEXT, source TEXT);
            CREATE TABLE IF NOT EXISTS drafts (id TEXT PRIMARY KEY, project TEXT NOT NULL REFERENCES projects(id), body TEXT NOT NULL);"
        ).map_err(|e| e.to_string())?;
        Ok(Self {
            root,
            db: Mutex::new(db),
        })
    }

    fn database(&self) -> AppResult<std::sync::MutexGuard<'_, Connection>> {
        self.db.lock().map_err(|e| e.to_string())
    }

    pub fn client_key(&self) -> AppResult<String> {
        let path = self.root.join("client-key");
        match std::fs::read_to_string(&path) {
            Ok(key) if key.len() == 64 && key.bytes().all(|b| b.is_ascii_hexdigit()) => Ok(key),
            Ok(_) => Err("客户端配对凭据已损坏".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let key = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
                atomic_write(&path, key.as_bytes())?;
                Ok(key)
            }
            Err(e) => Err(e.to_string()),
        }
    }

    pub fn directory(&self, project: Uuid) -> PathBuf {
        self.root.join("projects").join(project.to_string())
    }

    pub fn add_project(
        &self,
        media_path: PathBuf,
        provider_id: String,
        service_id: String,
        input: Input,
    ) -> AppResult<Project> {
        let id = Uuid::new_v4();
        let name = media_path
            .file_name()
            .ok_or("素材文件名无效")?
            .to_string_lossy()
            .into_owned();
        let project = Project {
            id,
            name,
            media_path,
            provider_id,
            service_id,
            input,
            original: None,
            correction: None,
        };
        std::fs::create_dir_all(self.directory(id)).map_err(|e| e.to_string())?;
        self.database()?
            .execute(
                "INSERT INTO projects(id, body) VALUES(?1,?2)",
                params![id.to_string(), encode(&project)?],
            )
            .map_err(|e| e.to_string())?;
        Ok(project)
    }

    fn load_project(db: &Connection, id: Uuid) -> AppResult<Project> {
        let raw: String = db
            .query_row(
                "SELECT body FROM projects WHERE id=?1",
                [id.to_string()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .ok_or("找不到项目")?;
        decode(&raw)
    }

    pub fn project(&self, id: Uuid) -> AppResult<Project> {
        let db = self.database()?;
        Self::load_project(&db, id)
    }

    pub fn projects(&self) -> AppResult<Vec<Project>> {
        let db = self.database()?;
        let mut query = db
            .prepare("SELECT body FROM projects ORDER BY rowid DESC")
            .map_err(|e| e.to_string())?;
        let rows = query
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        rows.map(|row| decode(&row.map_err(|e| e.to_string())?))
            .collect()
    }

    pub fn relocate(&self, id: Uuid, path: PathBuf) -> AppResult<Project> {
        let mut project = self.project(id)?;
        let mut file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
        let mut digest = Sha256::new();
        std::io::copy(&mut file, &mut digest).map_err(|e| e.to_string())?;
        if format!("{:x}", digest.finalize()) != project.input.fingerprint {
            return Err("所选文件与原素材不一致".into());
        }
        let db = self.database()?;
        project = Self::load_project(&db, id)?;
        project.media_path = path;
        db.execute(
            "UPDATE projects SET body=?2 WHERE id=?1",
            params![id.to_string(), encode(&project)?],
        )
        .map_err(|e| e.to_string())?;
        Ok(project)
    }

    pub fn copy_media(&self, id: Uuid) -> AppResult<Project> {
        let project = self.project(id)?;
        let folder = self.directory(id).join("media");
        std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
        let name = project.media_path.file_name().ok_or("素材文件名无效")?;
        let target = folder.join(name);
        if target.exists() {
            return self.relocate(id, target);
        }
        let mut source = std::io::BufReader::new(
            std::fs::File::open(&project.media_path).map_err(|e| e.to_string())?,
        );
        let mut temp = tempfile::NamedTempFile::new_in(&folder).map_err(|e| e.to_string())?;
        let mut buffer = vec![0; 128 * 1024];
        let mut hash = Sha256::new();
        use std::io::Read;
        loop {
            let size = source.read(&mut buffer).map_err(|e| e.to_string())?;
            if size == 0 {
                break;
            }
            hash.update(&buffer[..size]);
            temp.write_all(&buffer[..size]).map_err(|e| e.to_string())?;
        }
        if format!("{:x}", hash.finalize()) != project.input.fingerprint {
            return Err("源文件已变化，请重新定位原素材".into());
        }
        temp.as_file().sync_all().map_err(|e| e.to_string())?;
        temp.persist_noclobber(&target).map_err(|e| e.to_string())?;
        self.relocate(id, target)
    }

    pub fn add_job(
        &self,
        project: &Project,
        request: Request,
        source_revision: Option<Uuid>,
    ) -> AppResult<LocalJob> {
        let id = request.client_request_id;
        let job = LocalJob {
            project_id: project.id,
            provider_id: project.provider_id.clone(),
            service_id: project.service_id.clone(),
            request,
            remote: None,
            document: None,
            source_revision,
        };
        self.database()?
            .execute(
                "INSERT INTO jobs(id, project, body) VALUES(?1,?2,?3)",
                params![id.to_string(), project.id.to_string(), encode(&job)?],
            )
            .map_err(|e| e.to_string())?;
        Ok(job)
    }

    fn load_job(db: &Connection, id: Uuid) -> AppResult<LocalJob> {
        let raw: String = db
            .query_row(
                "SELECT body FROM jobs WHERE id=?1",
                [id.to_string()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .ok_or("找不到任务")?;
        decode(&raw)
    }

    pub fn job(&self, id: Uuid) -> AppResult<LocalJob> {
        let db = self.database()?;
        Self::load_job(&db, id)
    }

    pub fn jobs(&self) -> AppResult<Vec<LocalJob>> {
        let db = self.database()?;
        let mut query = db
            .prepare("SELECT body FROM jobs ORDER BY rowid DESC")
            .map_err(|e| e.to_string())?;
        let rows = query
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        rows.map(|row| {
            let mut job: LocalJob = decode(&row.map_err(|e| e.to_string())?)?;
            if let Some(revision) = job.document {
                let path = self
                    .directory(job.project_id)
                    .join(format!("{revision}.json"));
                if !path.is_file() {
                    job.document = None
                }
            }
            Ok(job)
        })
        .collect()
    }

    pub fn update_job(&self, id: Uuid, remote: Job) -> AppResult<LocalJob> {
        if id != remote.client_request_id {
            return Err("回执中的请求 ID 不一致".into());
        }
        let db = self.database()?;
        let mut job = Self::load_job(&db, id)?;
        if job
            .remote
            .as_ref()
            .is_some_and(|existing| existing.task_id != remote.task_id)
        {
            return Err("回执中的任务 ID 不一致".into());
        }
        job.remote = Some(remote);
        db.execute(
            "UPDATE jobs SET body=?2 WHERE id=?1",
            params![id.to_string(), encode(&job)?],
        )
        .map_err(|e| e.to_string())?;
        Ok(job)
    }

    pub fn import_result(&self, id: Uuid, temp: tempfile::NamedTempFile) -> AppResult<Project> {
        let job = self.job(id)?;
        let project = self.project(job.project_id)?;
        if job.source_revision.is_some() {
            self.import_draft(id, temp)?;
            return Ok(project);
        }
        let remote = job.remote.ok_or("未收到服务回执")?;
        let receipt = remote.result.ok_or("服务结果待生成")?;
        verify_file(temp.path(), receipt.bytes, &receipt.sha256)?;
        let transcript: Transcript = read_file(temp.path())?;
        validate_transcript(&transcript, &project)?;
        let attempt = remote.attempt_id;
        let revision = job
            .document
            .unwrap_or_else(|| local_revision(project.id, id, attempt));
        let path = self.directory(project.id).join(format!("{revision}.json"));
        match temp.persist_noclobber(&path) {
            Ok(_) => {}
            Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => {
                if verify_file(&path, receipt.bytes, &receipt.sha256).is_err() {
                    e.file.persist(&path).map_err(|e| e.to_string())?;
                }
            }
            Err(e) => return Err(e.error.to_string()),
        }
        let mut db = self.database()?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        let mut job = Self::load_job(&tx, id)?;
        let mut project = Self::load_project(&tx, job.project_id)?;
        if job.document == Some(revision) {
            return Ok(project);
        }
        if job
            .remote
            .as_ref()
            .is_none_or(|remote| remote.attempt_id != attempt)
        {
            return Err("任务版本已改变".into());
        }
        tx.execute("INSERT INTO revisions(id, project, kind, parent, source) VALUES(?1,?2,'original',?3,NULL)",
            params![revision.to_string(), project.id.to_string(),project.original.map(|v| v.to_string())]).map_err(|e| e.to_string())?;
        project.original = Some(revision);
        project.correction = None;
        job.document = Some(revision);
        tx.execute(
            "UPDATE projects SET body=?2 WHERE id=?1",
            params![project.id.to_string(), encode(&project)?],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "UPDATE jobs SET body=?2 WHERE id=?1",
            params![id.to_string(), encode(&job)?],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(project)
    }

    fn import_draft(&self, id: Uuid, temp: tempfile::NamedTempFile) -> AppResult<()> {
        let job = self.job(id)?;
        let remote = job.remote.as_ref().ok_or("未收到整理回执")?;
        let attempt = remote.attempt_id;
        let revision = job
            .document
            .unwrap_or_else(|| local_revision(job.project_id, id, attempt));
        let draft: Draft = read_file(temp.path())?;
        let source = job.source_revision.ok_or("整理任务缺少来源修订")?;
        let source_doc = self.document_at(job.project_id, source)?;
        let source_ids: HashSet<_> = source_doc.segments.iter().map(|s| s.id.as_str()).collect();
        let mut referenced = HashSet::new();
        if draft.schema_version != 1
            || !draft.complete
            || draft.source_revision != source
            || draft.blocks.is_empty()
            || job.request.operation["draftType"].as_str() != Some(draft.draft_type.as_str())
            || job.request.operation["mode"].as_str() != Some(draft.mode.as_str())
            || draft.blocks.iter().any(|block| {
                block.text.trim().is_empty()
                    || block
                        .source_segment_ids
                        .iter()
                        .any(|id| !source_ids.contains(id.as_str()))
                    || (block.source_segment_ids.is_empty() && !block.added_or_inferred)
                    || (draft.mode == "faithful" && block.added_or_inferred)
            })
            || draft
                .abridged_source_segment_ids
                .iter()
                .any(|id| !source_ids.contains(id.as_str()))
        {
            return Err("整理稿的结构、来源或完成范围无效".into());
        }
        referenced.extend(
            draft
                .blocks
                .iter()
                .flat_map(|b| b.source_segment_ids.iter())
                .map(String::as_str),
        );
        referenced.extend(draft.abridged_source_segment_ids.iter().map(String::as_str));
        if referenced != source_ids {
            return Err("整理稿缺少来源段落".into());
        }
        let receipt = remote.result.as_ref().ok_or("整理结果待生成")?;
        verify_file(temp.path(), receipt.bytes, &receipt.sha256)?;
        let path = self
            .directory(job.project_id)
            .join(format!("{revision}.json"));
        match temp.persist_noclobber(&path) {
            Ok(_) => {}
            Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => {
                if verify_file(&path, receipt.bytes, &receipt.sha256).is_err() {
                    e.file.persist(&path).map_err(|e| e.to_string())?;
                }
            }
            Err(e) => return Err(e.error.to_string()),
        }
        let mut db = self.database()?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        let mut job = Self::load_job(&tx, id)?;
        if job.document == Some(revision) {
            return Ok(());
        }
        if job
            .remote
            .as_ref()
            .is_none_or(|current| current.attempt_id != attempt)
        {
            return Err("整理任务版本已改变".into());
        }
        let candidate = Candidate {
            id: revision,
            revision,
            project_id: job.project_id,
            accepted: false,
            draft,
        };
        tx.execute(
            "INSERT INTO revisions(id,project,kind,parent,source) VALUES(?1,?2,'draft',NULL,?3)",
            params![
                revision.to_string(),
                job.project_id.to_string(),
                source.to_string()
            ],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT INTO drafts(id,project,body) VALUES(?1,?2,?3)",
            params![
                revision.to_string(),
                job.project_id.to_string(),
                encode(&candidate)?
            ],
        )
        .map_err(|e| e.to_string())?;
        job.document = Some(revision);
        tx.execute(
            "UPDATE jobs SET body=?2 WHERE id=?1",
            params![id.to_string(), encode(&job)?],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    }

    pub fn candidates(&self, id: Uuid) -> AppResult<Vec<Candidate>> {
        let db = self.database()?;
        let mut query = db
            .prepare("SELECT body FROM drafts WHERE project=?1 ORDER BY rowid DESC")
            .map_err(|e| e.to_string())?;
        let rows = query
            .query_map([id.to_string()], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        rows.map(|row| decode(&row.map_err(|e| e.to_string())?))
            .collect()
    }

    fn load_candidate(db: &Connection, id: Uuid) -> AppResult<Candidate> {
        let raw: String = db
            .query_row(
                "SELECT body FROM drafts WHERE id=?1",
                [id.to_string()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .ok_or("找不到候选稿")?;
        decode(&raw)
    }

    pub fn save_candidate(
        &self,
        id: Uuid,
        expected: Uuid,
        blocks: Vec<String>,
    ) -> AppResult<Candidate> {
        let mut candidate = Self::load_candidate(&*self.database()?, id)?;
        if candidate.revision != expected {
            return Err("候选稿版本已改变".into());
        }
        if blocks.len() != candidate.draft.blocks.len()
            || blocks
                .iter()
                .any(|text| text.trim().is_empty() || text.len() > 64 * 1024)
        {
            return Err("稿件段落为空或不完整".into());
        }
        for (block, text) in candidate.draft.blocks.iter_mut().zip(blocks) {
            block.text = text
        }
        candidate.revision = Uuid::new_v4();
        candidate.accepted = false;
        atomic_write(
            &self
                .directory(candidate.project_id)
                .join(format!("{}.json", candidate.revision)),
            encode(&candidate.draft)?.as_bytes(),
        )?;
        let mut db = self.database()?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        if Self::load_candidate(&tx, id)?.revision != expected {
            return Err("候选稿版本已改变".into());
        }
        tx.execute(
            "INSERT INTO revisions(id,project,kind,parent,source) VALUES(?1,?2,'draft',?3,?4)",
            params![
                candidate.revision.to_string(),
                candidate.project_id.to_string(),
                expected.to_string(),
                candidate.draft.source_revision.to_string()
            ],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "UPDATE drafts SET body=?2 WHERE id=?1",
            params![id.to_string(), encode(&candidate)?],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(candidate)
    }

    pub fn accept_candidate(&self, id: Uuid, expected: Uuid) -> AppResult<Candidate> {
        let db = self.database()?;
        let mut candidate = Self::load_candidate(&db, id)?;
        let project = Self::load_project(&db, candidate.project_id)?;
        if candidate.revision != expected
            || project.correction.or(project.original) != Some(candidate.draft.source_revision)
        {
            return Err("候选稿或来源版本已改变，请重新核对".into());
        }
        candidate.accepted = true;
        db.execute(
            "UPDATE drafts SET body=?2 WHERE id=?1",
            params![id.to_string(), encode(&candidate)?],
        )
        .map_err(|e| e.to_string())?;
        Ok(candidate)
    }

    pub fn export_candidate(&self, id: Uuid, format: &str) -> AppResult<String> {
        let db = self.database()?;
        let candidate = Self::load_candidate(&db, id)?;
        let project = Self::load_project(&db, candidate.project_id)?;
        if !candidate.accepted
            || project.correction.or(project.original) != Some(candidate.draft.source_revision)
        {
            return Err("请先接受当前来源版本的候选稿".into());
        }
        if !matches!(format, "txt" | "md") {
            return Err("稿件支持 TXT 和 Markdown".into());
        }
        let blocks = candidate
            .draft
            .blocks
            .iter()
            .map(|block| {
                if block.added_or_inferred && !block.text.starts_with("【新增/推测】") {
                    format!("【新增/推测】{}", block.text)
                } else {
                    block.text.clone()
                }
            })
            .collect::<Vec<_>>();
        Ok(blocks.join("\n\n"))
    }

    pub fn transcript(&self, project: Uuid) -> AppResult<Transcript> {
        let current = self.project(project)?;
        let revision = current.original.ok_or("项目还没有已取回的原文")?;
        let transcript = self.read_original(project, revision)?;
        validate_transcript(&transcript, &current)?;
        Ok(transcript)
    }

    fn read_original(&self, id: Uuid, revision: Uuid) -> AppResult<Transcript> {
        let row: String = self
            .database()?
            .query_row(
                "SELECT body FROM jobs WHERE project=?1 AND json_extract(body,'$.document')=?2",
                params![id.to_string(), revision.to_string()],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        let job: LocalJob = decode(&row)?;
        let receipt = job
            .remote
            .and_then(|job| job.result)
            .ok_or("本地原文缺少服务回执")?;
        let path = self.directory(id).join(format!("{revision}.json"));
        verify_file(&path, receipt.bytes, &receipt.sha256)?;
        read_file(&path)
    }

    pub fn document(&self, id: Uuid, original_only: bool) -> AppResult<Document> {
        let project = self.project(id)?;
        let original = project.original.ok_or("项目还没有已取回的原文")?;
        let revision = if original_only {
            original
        } else {
            project.correction.unwrap_or(original)
        };
        self.document_at(id, revision)
    }

    pub fn revisions(&self, id: Uuid) -> AppResult<Vec<DocumentRevision>> {
        let db = self.database()?;
        let mut query = db.prepare("SELECT id,kind,parent,source FROM revisions WHERE project=?1 AND kind IN ('original','correction') ORDER BY rowid DESC")
            .map_err(|e| e.to_string())?;
        let rows = query
            .query_map([id.to_string()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        rows.map(|row| {
            let (id, kind, parent, source) = row.map_err(|e| e.to_string())?;
            Ok(DocumentRevision {
                id: Uuid::parse_str(&id).map_err(|e| e.to_string())?,
                kind,
                parent,
                source,
            })
        })
        .collect()
    }

    pub fn checkout(&self, id: Uuid, revision: Uuid, expected: Uuid) -> AppResult<Project> {
        let document = self.document_at(id, revision)?;
        let db = self.database()?;
        let mut project = Self::load_project(&db, id)?;
        if project.correction.or(project.original) != Some(expected) {
            return Err("文稿版本已改变，请重新载入".into());
        }
        project.original = Some(document.original);
        project.correction = (document.revision != document.original).then_some(document.revision);
        db.execute(
            "UPDATE projects SET body=?2 WHERE id=?1",
            params![id.to_string(), encode(&project)?],
        )
        .map_err(|e| e.to_string())?;
        Ok(project)
    }

    fn document_at(&self, id: Uuid, revision: Uuid) -> AppResult<Document> {
        let (kind, source): (String, Option<String>) = self
            .database()?
            .query_row(
                "SELECT kind,source FROM revisions WHERE id=?1 AND project=?2",
                params![revision.to_string(), id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|e| e.to_string())?;
        let original = match kind.as_str() {
            "original" => revision,
            "correction" => source
                .and_then(|source| Uuid::parse_str(&source).ok())
                .ok_or("校订来源已损坏")?,
            _ => return Err("找不到转写修订".into()),
        };
        let transcript = self.read_original(id, original)?;
        validate_transcript(&transcript, &self.project(id)?)?;
        let mut segments = transcript.segments;
        if revision != original {
            let changes: HashMap<String, String> =
                read_file(&self.directory(id).join(format!("{revision}.json")))?;
            for segment in &mut segments {
                if let Some(text) = changes.get(&segment.id) {
                    segment.text.clone_from(text)
                }
            }
        }
        Ok(Document {
            revision,
            original,
            segments,
            waveform_peaks: transcript.waveform_peaks,
        })
    }

    pub fn save_correction(
        &self,
        id: Uuid,
        expected: Uuid,
        changes: HashMap<String, String>,
    ) -> AppResult<Document> {
        let document = self.document(id, false)?;
        if document.revision != expected {
            return Err("文稿版本已改变，请重新载入".into());
        }
        let mut segments = document.segments;
        let current_ids: HashSet<_> = segments.iter().map(|s| s.id.as_str()).collect();
        if changes.iter().any(|(id, text)| {
            !current_ids.contains(id.as_str()) || text.trim().is_empty() || text.len() > 64 * 1024
        }) {
            return Err("校订包含未知字幕或无效文本".into());
        }
        for segment in &mut segments {
            if let Some(text) = changes.get(&segment.id) {
                segment.text.clone_from(text)
            }
        }
        let original: HashMap<_, _> = self
            .transcript(id)?
            .segments
            .into_iter()
            .map(|s| (s.id, s.text))
            .collect();
        let corrections: HashMap<_, _> = segments
            .iter()
            .filter(|s| original.get(&s.id) != Some(&s.text))
            .map(|s| (&s.id, &s.text))
            .collect();
        let revision = Uuid::new_v4();
        let path = self.directory(id).join(format!("{revision}.json"));
        atomic_write(&path, encode(&corrections)?.as_bytes())?;
        let mut db = self.database()?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        let mut project = Self::load_project(&tx, id)?;
        if project.correction.or(project.original) != Some(expected) {
            return Err("文稿版本已改变，请重新载入".into());
        }
        tx.execute("INSERT INTO revisions(id, project, kind, parent, source) VALUES(?1,?2,'correction',?3,?4)",
            params![revision.to_string(), id.to_string(), expected.to_string(),document.original.to_string()]).map_err(|e| e.to_string())?;
        project.correction = Some(revision);
        tx.execute(
            "UPDATE projects SET body=?2 WHERE id=?1",
            params![id.to_string(), encode(&project)?],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(Document {
            revision,
            original: document.original,
            segments,
            waveform_peaks: document.waveform_peaks,
        })
    }
}

fn validate_transcript(transcript: &Transcript, project: &Project) -> AppResult<()> {
    if transcript.schema_version != 1
        || transcript.source_fingerprint != project.input.fingerprint
        || transcript.duration_ms != project.input.duration_ms
        || transcript.time_origin != "sourceMedia"
        || !matches!(
            transcript.timing_quality.as_str(),
            "segmentTimed" | "unitTimed"
        )
    {
        return Err("转写结果与项目或协议不一致".into());
    }
    if transcript.waveform_peaks.len() > 200
        || transcript
            .waveform_peaks
            .iter()
            .any(|peak| !(0.0..=1.0).contains(peak))
    {
        return Err("波形采样值无效".into());
    }
    let mut ids = HashSet::new();
    let mut previous_start = 0;
    for segment in &transcript.segments {
        if segment.id.is_empty()
            || !ids.insert(&segment.id)
            || segment.start_ms < previous_start
            || segment.end_ms <= segment.start_ms
            || segment.end_ms > transcript.duration_ms
            || segment.text.trim().is_empty()
        {
            return Err("转写结果含无效时间或字幕".into());
        }
        previous_start = segment.start_ms;
    }
    Ok(())
}
