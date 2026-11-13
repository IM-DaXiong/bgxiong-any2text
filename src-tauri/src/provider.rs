use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub protocol_min: u32,
    pub protocol_max: u32,
    pub schema_sha256: String,
    pub provider_id: String,
    pub endpoint_instance: String,
    pub service_id: String,
    pub models: Vec<Model>,
    pub operations: Vec<String>,
    pub input_mode: String,
    pub max_input_bytes: u64,
    pub max_duration_ms: u64,
    pub pause: bool,
    pub resume: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Model {
    pub id: String,
    pub name: String,
    pub languages: Vec<String>,
    pub timings: Vec<String>,
    pub installed: bool,
    pub ready: bool,
    pub inference_ready: bool,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Input {
    pub input_ref: String,
    pub fingerprint: String,
    pub duration_ms: u64,
    pub audio_tracks: Vec<AudioTrack>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioTrack {
    pub index: u32,
    pub codec: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub task_id: String,
    pub attempt_id: Uuid,
    pub client_request_id: Uuid,
    pub status: String,
    pub committed_ms: u64,
    pub duration_ms: u64,
    pub result: Option<Receipt>,
    pub error: Option<ProviderError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Receipt {
    pub sha256: String,
    pub bytes: u64,
    pub retained_until: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Segment {
    pub id: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transcript {
    pub schema_version: u32,
    pub source_fingerprint: String,
    pub duration_ms: u64,
    pub time_origin: String,
    pub segments: Vec<Segment>,
    pub timing_quality: String,
    #[serde(default)]
    pub units: Vec<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub waveform_peaks: Vec<f32>,
    pub provenance: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    pub client_request_id: Uuid,
    pub operation: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextInput {
    pub input_ref: String,
    pub fingerprint: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportProgress {
    pub phase: &'static str,
    pub bytes: Option<u64>,
    pub total: Option<u64>,
}

pub type ImportReporter = Box<dyn Fn(ImportProgress) + Send + Sync>;

#[async_trait::async_trait]
pub trait ServiceProvider: Send + Sync {
    async fn connect(&self) -> Result<Capabilities, String>;
    async fn health(&self) -> Result<(), String>;
    async fn prepare_input(&self, service_id: &str, path: &Path, report: ImportReporter) -> Result<Input, String>;
    async fn appearance(&self) -> Result<Option<String>, String> { Ok(None) }
    async fn prepare_text(&self, service_id: &str, snapshot: Vec<u8>) -> Result<TextInput, String>;
    async fn submit(&self, service_id: &str, request: &Request) -> Result<Job, String>;
    async fn get_job(&self, service_id: &str, task: &str) -> Result<Job, String>;
    async fn get_job_by_request(
        &self,
        service_id: &str,
        request: Uuid,
    ) -> Result<Option<Job>, String>;
    async fn control(
        &self,
        service_id: &str,
        task: &str,
        attempt: Uuid,
        action: &str,
    ) -> Result<Job, String>;
    async fn read_result(
        &self,
        service_id: &str,
        task: &str,
        receipt: &Receipt,
        file: &Path,
    ) -> Result<(), String>;
    async fn acknowledge(&self, service_id: &str, task: &str, digest: &str) -> Result<(), String>;
    async fn disconnect(&self);
}
