use std::io::Write;
use std::path::Path;

use crate::provider::Segment;
use crate::workspace::Workspace;
use uuid::Uuid;

#[derive(serde::Deserialize, Copy, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TimeRange {
    pub start_ms: u64,
    pub end_ms: u64,
}

fn timestamp(ms: u64, separator: char) -> String {
    format!(
        "{:02}:{:02}:{:02}{separator}{:03}",
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1_000 % 60,
        ms % 1000
    )
}

fn subtitle_lines(segments: &[Segment], vtt: bool) -> String {
    let sep = if vtt { '.' } else { ',' };
    let mut output = if vtt {
        "WEBVTT\n\n".to_owned()
    } else {
        String::new()
    };
    for (index, segment) in segments.iter().enumerate() {
        let text = segment
            .text
            .lines()
            .filter(|line| !line.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        output.push_str(&format!(
            "{}\n{} --> {}\n{}\n\n",
            index + 1,
            timestamp(segment.start_ms, sep),
            timestamp(segment.end_ms, sep),
            text
        ));
    }
    output
}

pub fn render(
    workspace: &Workspace,
    project: Uuid,
    original: bool,
    format: &str,
    range: Option<TimeRange>,
) -> Result<String, String> {
    let mut document = workspace.document(project, original)?;
    if let Some(range) = range {
        if range.start_ms >= range.end_ms
            || range.end_ms > workspace.project(project)?.input.duration_ms
        {
            return Err("导出范围超出素材时长".into());
        }
        document
            .segments
            .retain(|segment| segment.end_ms > range.start_ms && segment.start_ms < range.end_ms);
        if document.segments.is_empty() {
            return Err("选定范围内没有字幕".into());
        }
    }
    match format {
        "txt" => Ok(document
            .segments
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")),
        "srt" => Ok(subtitle_lines(&document.segments, false)),
        "vtt" => Ok(subtitle_lines(&document.segments, true)),
        "json" => {
            let mut transcript = workspace.transcript(project)?;
            if document.revision != document.original || range.is_some() {
                transcript.segments = document.segments;
                transcript.units.clear();
                transcript.timing_quality = "segmentTimed".into();
            }
            serde_json::to_string_pretty(&transcript).map_err(|e| e.to_string())
        }
        _ => Err("不支持该导出格式".into()),
    }
}

pub fn save(path: &Path, format: &str, text: &str) -> Result<(), String> {
    if !path.is_absolute()
        || !path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case(format))
    {
        return Err("文件扩展名与所选导出格式不一致".into());
    }
    let folder = path.parent().ok_or("无效的导出文件夹")?;
    let mut temp = tempfile::NamedTempFile::new_in(folder).map_err(|e| e.to_string())?;
    temp.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
    temp.as_file().sync_all().map_err(|e| e.to_string())?;
    temp.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}
