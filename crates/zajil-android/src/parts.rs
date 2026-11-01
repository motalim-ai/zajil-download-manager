//! How far each connection of a running download has come, for the
//! download's details on the phone (the D5 design's segment bars).

use std::path::Path;

use zajil_core::state::DownloadState;

/// How much of each part of a download is on disk, read from its state
/// file (written about once a second while it runs): one fraction from 0
/// to 1 per segment, in the file's order. Empty when the state file cannot
/// be read, is not a regular file, or a part's end is not known.
#[uniffi::export]
pub fn segment_parts(state_file: String) -> Vec<f32> {
    let Ok(state) = DownloadState::load(Path::new(&state_file)) else {
        return Vec::new();
    };
    fractions(&state)
}

fn fractions(state: &DownloadState) -> Vec<f32> {
    let mut parts = Vec::with_capacity(state.segments.len());
    for segment in &state.segments {
        let Some(end) = segment.end else {
            return Vec::new();
        };
        // `end` is inclusive; a range that ends before it starts is empty.
        let length = end.saturating_sub(segment.start).saturating_add(1);
        if end < segment.start {
            parts.push(1.0);
            continue;
        }
        parts.push((segment.done.min(length) as f64 / length as f64) as f32);
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;
    use zajil_core::state::{Segment, Validator};

    fn state(segments: Vec<Segment>) -> DownloadState {
        let mut s = DownloadState::new(
            "https://example.com/f".into(),
            "https://example.com/f".into(),
            Validator::default(),
            Some(400),
            true,
        );
        s.segments = segments;
        s
    }

    #[test]
    fn each_part_in_order() {
        let s = state(vec![
            Segment {
                start: 0,
                end: Some(99),
                done: 100,
            },
            Segment {
                start: 100,
                end: Some(199),
                done: 50,
            },
            Segment {
                start: 200,
                end: Some(399),
                done: 0,
            },
        ]);
        assert_eq!(fractions(&s), vec![1.0, 0.5, 0.0]);
    }

    #[test]
    fn unknown_end_gives_nothing() {
        let s = state(vec![Segment {
            start: 0,
            end: None,
            done: 10,
        }]);
        assert!(fractions(&s).is_empty());
    }

    #[test]
    fn huge_values_do_not_overflow() {
        let s = state(vec![Segment {
            start: 0,
            end: Some(u64::MAX),
            done: u64::MAX,
        }]);
        assert_eq!(fractions(&s), vec![1.0]);
    }

    #[test]
    fn written_state_file_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f.zajil-state");
        let mut s = state(vec![
            Segment {
                start: 0,
                end: Some(199),
                done: 50,
            },
            Segment {
                start: 200,
                end: Some(399),
                done: 200,
            },
        ]);
        s.save(&path).unwrap();
        assert_eq!(
            segment_parts(path.to_string_lossy().into_owned()),
            vec![0.25, 1.0]
        );
    }

    #[test]
    fn missing_or_damaged_file_gives_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(segment_parts(dir.path().join("none").to_string_lossy().into_owned()).is_empty());
        let damaged = dir.path().join("bad.zajil-state");
        std::fs::write(&damaged, b"{").unwrap();
        assert!(segment_parts(damaged.to_string_lossy().into_owned()).is_empty());
    }
}
