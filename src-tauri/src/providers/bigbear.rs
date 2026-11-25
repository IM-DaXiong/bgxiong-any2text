use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::{Client, Response, StatusCode, Url};
use futures_util::StreamExt;
use serde::{de::DeserializeOwned, Deserialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;
use tokio_util::io::ReaderStream;
use uuid::Uuid;

use crate::provider::{
    Capabilities, ImportProgress, ImportReporter, Input, Job, ProviderError, Receipt, Request, ServiceProvider, TextInput,
};

const SCHEMA: &[u8] = include_bytes!("../../../contracts/v1/service-provider.schema.json");

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Registry {
    base_url: String,
    instance_id: String,
    install_token: Option<String>,
    transport_features: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionReceipt {
    session_token: String,
    endpoint_instance: String,
}

#[derive(Clone)]
struct Connection {
    base: Url,
    token: String,
    capabilities: Capabilities,
    refresh_at: Instant,
}

pub struct BigBear {
    client: Client,
    client_key: String,
    registry: PathBuf,
    connection: Mutex<Option<Connection>>,
}

impl BigBear {
    pub fn new(client_key: String, registry: PathBuf) -> Result<Self, String> {
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(300))
            .build()
            .map_err(network)?;
        Ok(Self {
            client,
            client_key,
            registry,
            connection: Mutex::new(None),
        })
    }

    async fn connection(&self, expected_service: Option<&str>) -> Result<Connection, String> {
        let bytes = tokio::fs::read(&self.registry)
            .await
            .map_err(|_| "比格熊服务未启动".to_string())?;
        let registry: Registry =
            serde_json::from_slice(&bytes).map_err(|_| "比格熊服务登记暂不可读".to_string())?;
        let registry_base = validate_registry(&registry)?;
        let mut current = self.connection.lock().await;
        if let Some(cached) = current.as_ref().filter(|cached| {
            cached.capabilities.endpoint_instance == registry.instance_id
                && Instant::now() < cached.refresh_at
        }) {
            require_service(expected_service, &cached.capabilities)?;
            return Ok(cached.clone());
        }
        let base = registry_base.join("any2text/").map_err(|e| e.to_string())?;
        let install_token = registry.install_token.ok_or("比格熊 Bridge 未启用配对")?;
        let session: SessionReceipt = decode(
            self.client
                .post(join(&base, "session")?)
                .bearer_auth(install_token)
                .json(&json!({"clientKey":self.client_key}))
                .send()
                .await
                .map_err(network)?,
        )
        .await?;
        if session.endpoint_instance != registry.instance_id {
            return Err("比格熊服务实例已变更".into());
        }
        let caps: Capabilities = decode(
            self.client
                .get(join(&base, "capabilities")?)
                .bearer_auth(&session.session_token)
                .send()
                .await
                .map_err(network)?,
        )
        .await?;
        if caps.endpoint_instance != registry.instance_id
            || caps.provider_id != "bigbear"
            || caps.input_mode != "localStream"
            || !(caps.protocol_min..=caps.protocol_max).contains(&1)
            || caps.schema_sha256 != format!("{:x}", Sha256::digest(SCHEMA))
        {
            return Err("比格熊服务版本与 Any2Text 不兼容".into());
        }
        require_service(expected_service, &caps)?;
        let connection = Connection {
            base,
            token: session.session_token,
            capabilities: caps,
            refresh_at: Instant::now() + Duration::from_secs(12 * 60),
        };
        *current = Some(connection.clone());
        Ok(connection)
    }
}

fn validate_registry(registry: &Registry) -> Result<Url, String> {
    let base = Url::parse(&registry.base_url).map_err(|_| "比格熊服务地址无效".to_string())?;
    if base.scheme() != "http"
        || base.host_str() != Some("127.0.0.1")
        || base.port().is_none()
        || base.path() != "/v1/"
        || !base.username().is_empty()
        || base.password().is_some()
        || base.query().is_some()
        || base.fragment().is_some()
    {
        return Err("比格熊服务地址无效".into());
    }
    if !registry
        .transport_features
        .iter()
        .any(|feature| feature == "any2textV1")
    {
        return Err("当前比格熊实例未提供 Any2Text v1 服务".into());
    }
    Ok(base)
}

fn require_service(expected: Option<&str>, caps: &Capabilities) -> Result<(), String> {
    if expected.is_some_and(|id| id != caps.service_id) {
        return Err("当前比格熊安装实例与原任务不匹配".into());
    }
    Ok(())
}

fn join(base: &Url, relative: &str) -> Result<Url, String> {
    base.join(relative).map_err(|e| e.to_string())
}

fn job_path(task: &str) -> Result<String, String> {
    if task.is_empty()
        || task.len() > 256
        || !task
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err("任务引用无效".into());
    }
    Ok(format!("jobs/{task}"))
}

fn network(_: reqwest::Error) -> String {
    "比格熊连接失败，请核对服务状态".into()
}

fn import_network(error: reqwest::Error) -> String {
    if error.is_timeout() {
        "导入等待超时，请确认比格熊仍在运行后重试".into()
    } else {
        network(error)
    }
}

async fn check(response: Response) -> Result<Response, String> {
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    let reason = response.json::<ProviderError>().await.ok();
    Err(reason
        .map(|e| e.message)
        .unwrap_or_else(|| format!("比格熊服务返回 {status}")))
}

async fn decode<T: DeserializeOwned>(response: Response) -> Result<T, String> {
    check(response)
        .await?
        .json()
        .await
        .map_err(|_| "比格熊回执格式无效".into())
}

#[async_trait::async_trait]
impl ServiceProvider for BigBear {
    async fn connect(&self) -> Result<Capabilities, String> {
        Ok(self.connection(None).await?.capabilities)
    }

    async fn health(&self) -> Result<(), String> {
        let session = self.connection(None).await?;
        let response = self
            .client
            .get(join(&session.base, "../health")?)
            .timeout(Duration::from_secs(4))
            .send()
            .await
            .map_err(network)?;
        let envelope: serde_json::Value = decode(response).await?;
        if envelope["data"]["instanceId"].as_str()
            != Some(session.capabilities.endpoint_instance.as_str())
            || envelope["data"]["status"] != "ok"
        {
            return Err("比格熊服务状态待核对".into());
        }
        Ok(())
    }

    async fn shortcut_bindings(&self) -> Result<serde_json::Value, String> {
        let session = self.connection(None).await?;
        let response = self.client.get(join(&session.base, "shortcut-bindings")?)
            .bearer_auth(&session.token).timeout(Duration::from_secs(3))
            .send().await.map_err(network)?;
        decode(response).await
    }

    async fn appearance(&self) -> Result<Option<String>, String> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Appearance { theme_id: Option<String> }
        let session = self.connection(None).await?;
        let response = self.client.get(join(&session.base, "appearance")?)
            .bearer_auth(&session.token).timeout(Duration::from_secs(3))
            .send().await.map_err(network)?;
        if response.status() == StatusCode::NOT_FOUND { return Ok(None); }
        Ok(decode::<Appearance>(response).await?.theme_id)
    }

    async fn prepare_input(&self, service_id: &str, path: &Path, report: ImportReporter) -> Result<Input, String> {
        let session = self.connection(Some(service_id)).await?;
        let mut file = tokio::fs::File::open(path)
            .await
            .map_err(|e| e.to_string())?;
        let meta = file.metadata().await.map_err(|e| e.to_string())?;
        if !meta.is_file() || meta.len() == 0 || meta.len() > session.capabilities.max_input_bytes {
            return Err("素材为空或超出大小限制".into());
        }
        let total = meta.len();
        let report: Arc<dyn Fn(ImportProgress) + Send + Sync> = report.into();
        let send_progress = |phase, bytes| report(ImportProgress { phase, bytes: Some(bytes), total: Some(total) });
        send_progress("checking", 0);
        let mut hash = Sha256::new();
        let mut buffer = vec![0; 256 * 1024];
        let mut checked = 0;
        let hash_start = Instant::now();
        let mut updated = hash_start;
        loop {
            let count = file.read(&mut buffer).await.map_err(|e| e.to_string())?;
            if count == 0 {
                break;
            }
            hash.update(&buffer[..count]);
            checked += count as u64;
            if updated.elapsed() >= Duration::from_millis(100) {
                send_progress("checking", checked);
                updated = Instant::now();
            }
        }
        if checked != total { return Err("素材在导入时发生变化，请重新选择".into()); }
        use tokio::io::AsyncSeekExt;
        file.rewind().await.map_err(|e| e.to_string())?;
        let fingerprint = format!("{:x}", hash.finalize());
        eprintln!("Any2Text: verified {total} media bytes in {} ms", hash_start.elapsed().as_millis());
        send_progress("sending", 0);
        let mut sent = 0;
        let upload_start = Instant::now();
        let mut updated = upload_start;
        let stream = ReaderStream::with_capacity(file, 256 * 1024).map(move |chunk| {
            if let Ok(bytes) = &chunk {
                sent += bytes.len() as u64;
                if sent == total {
                    eprintln!("Any2Text: read {total} media bytes into the request in {} ms", upload_start.elapsed().as_millis());
                    report(ImportProgress { phase: "verifying", bytes: None, total: None });
                } else if updated.elapsed() >= Duration::from_millis(100) {
                    report(ImportProgress { phase: "sending", bytes: Some(sent), total: Some(total) });
                    updated = Instant::now();
                }
            }
            chunk
        });
        let response = self
            .client
            .put(join(&session.base, &format!("inputs/{fingerprint}"))?)
            .bearer_auth(&session.token)
            .header("content-length", meta.len())
            .body(reqwest::Body::wrap_stream(stream))
            .send()
            .await
            .map_err(import_network)?;
        let input: Input = decode(response).await?;
        eprintln!("Any2Text: media receipt confirmed in {} ms from upload start", upload_start.elapsed().as_millis());
        if input.fingerprint != fingerprint {
            return Err("素材回执摘要不匹配".into());
        }
        Ok(input)
    }

    async fn submit(&self, service_id: &str, request: &Request) -> Result<Job, String> {
        let session = self.connection(Some(service_id)).await?;
        decode(
            self.client
                .post(join(&session.base, "jobs")?)
                .bearer_auth(&session.token)
                .json(request)
                .send()
                .await
                .map_err(network)?,
        )
        .await
    }

    async fn prepare_text(&self, service_id: &str, snapshot: Vec<u8>) -> Result<TextInput, String> {
        let session = self.connection(Some(service_id)).await?;
        let fingerprint = format!("{:x}", Sha256::digest(&snapshot));
        let result: TextInput = decode(
            self.client
                .put(join(&session.base, &format!("text-inputs/{fingerprint}"))?)
                .bearer_auth(&session.token)
                .header("content-type", "application/json")
                .body(snapshot)
                .send()
                .await
                .map_err(network)?,
        )
        .await?;
        if result.fingerprint != fingerprint {
            return Err("校订快照摘要不匹配".into());
        }
        Ok(result)
    }

    async fn get_job(&self, service_id: &str, task: &str) -> Result<Job, String> {
        let session = self.connection(Some(service_id)).await?;
        decode(
            self.client
                .get(join(&session.base, &job_path(task)?)?)
                .bearer_auth(&session.token)
                .send()
                .await
                .map_err(network)?,
        )
        .await
    }

    async fn get_job_by_request(
        &self,
        service_id: &str,
        request: Uuid,
    ) -> Result<Option<Job>, String> {
        let session = self.connection(Some(service_id)).await?;
        let response = self
            .client
            .get(join(&session.base, &format!("jobs/by-request/{request}"))?)
            .bearer_auth(&session.token)
            .send()
            .await
            .map_err(network)?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        decode(response).await.map(Some)
    }

    async fn control(
        &self,
        service_id: &str,
        task: &str,
        attempt: Uuid,
        action: &str,
    ) -> Result<Job, String> {
        let session = self.connection(Some(service_id)).await?;
        let url = join(&session.base, &format!("{}/control", job_path(task)?))?;
        decode(
            self.client
                .post(url)
                .bearer_auth(&session.token)
                .json(&json!({"attemptId":attempt,"action":action}))
                .send()
                .await
                .map_err(network)?,
        )
        .await
    }

    async fn read_result(
        &self,
        service_id: &str,
        task: &str,
        receipt: &Receipt,
        file: &Path,
    ) -> Result<(), String> {
        let session = self.connection(Some(service_id)).await?;
        let url = join(&session.base, &format!("{}/result", job_path(task)?))?;
        let mut response = check(
            self.client
                .get(url)
                .bearer_auth(&session.token)
                .send()
                .await
                .map_err(network)?,
        )
        .await?;
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(file)
            .await
            .map_err(|e| e.to_string())?;
        let mut size = 0u64;
        let mut digest = Sha256::new();
        while let Some(chunk) = response.chunk().await.map_err(network)? {
            size = size.saturating_add(chunk.len() as u64);
            if size > receipt.bytes {
                return Err("结果超过声明大小".into());
            }
            digest.update(&chunk);
            file.write_all(&chunk).await.map_err(|e| e.to_string())?;
        }
        if size != receipt.bytes || format!("{:x}", digest.finalize()) != receipt.sha256 {
            return Err("取回结果的大小或摘要不匹配".into());
        }
        file.sync_all().await.map_err(|e| e.to_string())
    }

    async fn acknowledge(&self, service_id: &str, task: &str, digest: &str) -> Result<(), String> {
        let session = self.connection(Some(service_id)).await?;
        let url = join(&session.base, &format!("{}/result", job_path(task)?))?;
        check(
            self.client
                .post(url)
                .bearer_auth(&session.token)
                .json(&json!({"sha256":digest}))
                .send()
                .await
                .map_err(network)?,
        )
        .await?;
        Ok(())
    }

    async fn disconnect(&self) {
        self.connection.lock().await.take();
    }
}
