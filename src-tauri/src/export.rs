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
    expected_revision: Option<Uuid>,
) -> Result<String, String> {
    let mut document = workspace.document(project, original)?;
    if expected_revision.is_some_and(|expected| expected != document.revision) {
        return Err("文稿版本已改变，请刷新预览".into());
    }
    if let Some(range) = range {
        if range.start_ms >= range.end_ms
            || range.end_ms > workspace.project(project)?.input.duration_ms
        {
            return Err("导出范围超出素材时长".into());
        }
    }
    let intersects = |start: u64, end: u64| {
        range.is_none_or(|r| end > r.start_ms && start < r.end_ms)
    };
    let pending: std::collections::HashSet<_> = document.review_groups.iter()
        .filter(|group| intersects(group.start_ms, group.end_ms))
        .map(|group| group.id.as_str()).collect();
    if format == "txt" {
        let lines: Vec<_> = document.entries.iter().filter(|entry| {
            if let Some(group) = &entry.review_group_id {
                pending.contains(group.as_str())
            } else {
                entry.start_ms.zip(entry.end_ms).is_some_and(|(start, end)| intersects(start, end))
            }
        }).map(|entry| entry.text.as_str()).collect();
        return if range.is_some() && lines.is_empty() {
            Err("选定范围内没有字幕".into())
        } else {
            Ok(lines.join("\n"))
        };
    }
    if !pending.is_empty() {
        return Err("请先复核导出范围内的字幕时间".into());
    }
    document.segments.retain(|segment| intersects(segment.start_ms, segment.end_ms));
    if range.is_some() && document.segments.is_empty() {
        return Err("选定范围内没有字幕".into());
    }
    match format {
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
