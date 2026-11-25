use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::provider::Segment;

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Correction {
    pub schema_version: u32,
    pub entries: Vec<Entry>,
    pub review_groups: Vec<ReviewGroup>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Entry {
    pub id: String,
    pub text: String,
    pub source_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review_group_id: Option<String>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReviewGroup {
    pub id: String,
    pub member_ids: Vec<String>,
    pub start_ms: u64,
    pub end_ms: u64,
    pub boundaries: Vec<Boundary>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Boundary {
    pub suggested_ms: u64,
    pub ready: bool,
}

impl From<&Segment> for Entry {
    fn from(segment: &Segment) -> Self {
        Self {
            id: segment.id.clone(),
            text: segment.text.clone(),
            source_ids: vec![segment.id.clone()],
            start_ms: Some(segment.start_ms),
            end_ms: Some(segment.end_ms),
            review_group_id: None,
        }
    }
}

impl Entry {
    pub fn timed(&self) -> Option<Segment> {
        Some(Segment {
            id: self.id.clone(),
            text: self.text.clone(),
            start_ms: self.start_ms?,
            end_ms: self.end_ms?,
        })
    }
}

fn root(parents: &mut [usize], mut index: usize) -> usize {
    while parents[index] != index {
        parents[index] = parents[parents[index]];
        index = parents[index];
    }
    index
}

fn extend_range(regions: &mut HashMap<usize, (u64, u64)>, id: usize, start: u64, end: u64) {
    regions
        .entry(id)
        .and_modify(|range| {
            range.0 = range.0.min(start);
            range.1 = range.1.max(end);
        })
        .or_insert((start, end));
}

impl Correction {
    pub fn validate(&self, source: &[Segment], duration: u64) -> Result<(), String> {
        if self.schema_version != 2 || self.entries.is_empty() != source.is_empty() {
            return Err("不支持的校订格式或空字幕稿".into());
        }
        let sources: HashMap<_, _> = source
            .iter()
            .enumerate()
            .map(|(index, segment)| (segment.id.as_str(), index))
            .collect();
        let mut positions = HashMap::new();
        let mut referenced = HashSet::new();
        let mut previous_source = None;
        for (index, entry) in self.entries.iter().enumerate() {
            let provenance = entry
                .source_ids
                .iter()
                .map(|id| sources.get(id.as_str()).copied())
                .collect::<Option<Vec<_>>>();
            let Some(provenance) =
                provenance.filter(|ids| !ids.is_empty() && ids.windows(2).all(|w| w[0] < w[1]))
            else {
                return Err(format!("第 {} 条字幕来源无效", index + 1));
            };
            if previous_source.is_some_and(|value| value > provenance[0]) {
                return Err(format!("第 {} 条字幕来源顺序无效", index + 1));
            }
            previous_source = Some(provenance[0]);
            referenced.extend(provenance);

            if entry.id.is_empty()
                || entry.id.len() > 256
                || positions.insert(entry.id.as_str(), index).is_some()
            {
                return Err(format!("第 {} 条字幕 ID 无效", index + 1));
            }
            match (entry.start_ms, entry.end_ms, &entry.review_group_id) {
                (Some(start), Some(end), None) if start < end && end <= duration => {}
                (None, None, Some(_)) => {}
                _ => return Err(format!("第 {} 条字幕时间无效", index + 1)),
            }
        }
        if referenced.len() != source.len() {
            return Err("字幕稿缺少原文来源".into());
        }
        let mut groups = HashSet::new();
        let mut members = HashSet::new();
        for group in &self.review_groups {
            let indexes = group
                .member_ids
                .iter()
                .map(|id| positions.get(id.as_str()).copied())
                .collect::<Option<Vec<_>>>();
            if group.id.is_empty()
                || !groups.insert(group.id.as_str())
                || group.start_ms >= group.end_ms
                || group.end_ms > duration
                || group.member_ids.len() < 2
                || group.boundaries.len() + 1 != group.member_ids.len()
                || !indexes.is_some_and(|ids| {
                    ids.windows(2).all(|w| w[0] + 1 == w[1])
                        && ids.iter().all(|index| {
                            self.entries[*index].review_group_id.as_deref()
                                == Some(group.id.as_str())
                        })
                })
                || !group
                    .member_ids
                    .iter()
                    .all(|id| members.insert(id.as_str()))
            {
                return Err("待复核组无效".into());
            }
            let mut previous = group.start_ms;
            for boundary in &group.boundaries {
                if boundary.ready {
                    if boundary.suggested_ms <= previous || boundary.suggested_ms >= group.end_ms {
                        return Err("已认可的断点无效".into());
                    }
                    previous = boundary.suggested_ms;
                } else if !(group.start_ms..=group.end_ms).contains(&boundary.suggested_ms) {
                    return Err("建议断点超出复核范围".into());
                }
            }
        }
        if self
            .entries
            .iter()
            .any(|entry| entry.review_group_id.is_some() && !members.contains(entry.id.as_str()))
        {
            return Err("字幕缺少有效待复核组".into());
        }
        let mut parents: Vec<_> = (0..source.len()).collect();
        let mut anchor = HashMap::new();
        let review_by_id: HashMap<_, _> = self
            .review_groups
            .iter()
            .map(|group| (group.id.as_str(), group))
            .collect();
        for entry in &self.entries {
            let first = sources[entry.source_ids[0].as_str()];
            let pivot = if let Some(id) = entry.review_group_id.as_deref() {
                *anchor.entry(id).or_insert(first)
            } else {
                first
            };
            for id in &entry.source_ids {
                let origin = root(&mut parents, sources[id.as_str()]);
                let group = root(&mut parents, pivot);
                parents[origin] = group;
            }
        }
        let mut original_ranges = HashMap::new();
        for (index, segment) in source.iter().enumerate() {
            extend_range(
                &mut original_ranges,
                root(&mut parents, index),
                segment.start_ms,
                segment.end_ms,
            );
        }
        let mut edited_ranges = HashMap::new();
        for entry in &self.entries {
            let (start, end) = if let Some(id) = entry.review_group_id.as_deref() {
                let group = review_by_id[id];
                (group.start_ms, group.end_ms)
            } else {
                (entry.start_ms.unwrap(), entry.end_ms.unwrap())
            };
            extend_range(
                &mut edited_ranges,
                root(&mut parents, sources[entry.source_ids[0].as_str()]),
                start,
                end,
            );
        }
        if original_ranges != edited_ranges {
            return Err("校订改变了原文时间范围的外沿".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subtitle_rejects_duplicate_members_and_out_of_range_review() {
        let source = vec![Segment {
            id: "original".into(),
            text: "甲。乙。".into(),
            start_ms: 0,
            end_ms: 1000,
        }];
        let mut entry = Entry::from(&source[0]);
        entry.start_ms = None;
        entry.end_ms = None;
        entry.review_group_id = Some("review".into());
        let mut right = entry.clone();
        right.id = "right".into();
        let mut draft = Correction {
            schema_version: 2,
            entries: vec![entry, right],
            review_groups: vec![ReviewGroup {
                id: "review".into(),
                member_ids: vec!["original".into(), "right".into()],
                start_ms: 0,
                end_ms: 1000,
                boundaries: vec![Boundary {
                    suggested_ms: 500,
                    ready: false,
                }],
            }],
        };
        assert!(draft.validate(&source, 1000).is_ok());
        draft.review_groups[0].boundaries[0].suggested_ms = 1000;
        draft.review_groups[0].boundaries[0].ready = true;
        assert!(draft.validate(&source, 1000).is_err());
        draft.review_groups[0].boundaries[0].suggested_ms = 500;
        draft.review_groups[0].member_ids[1] = "original".into();
        assert!(draft.validate(&source, 1000).is_err());
    }
}
