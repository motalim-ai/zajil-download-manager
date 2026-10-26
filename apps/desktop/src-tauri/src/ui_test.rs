//! Only with the `ui-test` feature, never in a release build (CI checks):
//! fixed demo data for the screenshots of the real window, and a way for
//! the page to report where it drew things, for the image test of mixed
//! names (owner's decision on the M4 plan).
//!
//! `zajil-desktop --ui-test <view>-<lang>-<theme> --ui-test-out <folder>`: the page
//! shows the view (`list`, `add`, `details`, `refresh`, `names`…), then calls
//! [`ui_test_report`], which writes `<folder>/<scenario>.json`. The CI
//! script waits for that file, takes a screenshot of the window with the
//! system's own tool, and closes the program.

use std::path::PathBuf;

use tauri::State;

use crate::view::{DetailsView, DownloadView, QueueView, SegmentView, Snapshot, StatusView};

pub struct UiTest {
    pub scenario: Option<String>,
    out: Option<PathBuf>,
}

impl UiTest {
    pub fn from_args(args: &[String]) -> Self {
        let value = |flag: &str| {
            args.iter()
                .position(|a| a == flag)
                .and_then(|i| args.get(i + 1))
                .cloned()
        };
        UiTest {
            scenario: value("--ui-test"),
            out: value("--ui-test-out").map(PathBuf::from),
        }
    }
}

impl UiTest {
    /// The demo list, only while a scene is shown: a `ui-test` build run
    /// without `--ui-test` shows the real downloads (review of #47).
    pub fn demo(&self) -> Option<Snapshot> {
        self.scenario.as_ref().map(|_| demo_snapshot())
    }
}

/// Fits the window inside the screen's work area (without the taskbar or
/// the Dock): the 1024 x 768 screens of the CI machines are smaller than
/// the window, and a capture would take the taskbar over the footer
/// (review of #47, Windows). `wanted`: the inner size in logical pixels to
/// give it first (an add window's scene); it is not read back, since the
/// resize is not done at once on Linux and Windows (review of #104).
pub fn fit_to_screen(
    window: &tauri::WebviewWindow,
    wanted: Option<(f64, f64)>,
) -> tauri::Result<()> {
    let Some(monitor) = window.current_monitor()? else {
        return Ok(());
    };
    let area = *monitor.work_area();
    let (outer, mut inner) = (window.outer_size()?, window.inner_size()?);
    let frame = (
        outer.width.saturating_sub(inner.width),
        outer.height.saturating_sub(inner.height),
    );
    if let Some((width, height)) = wanted {
        window.set_min_size(None::<tauri::LogicalSize<f64>>)?;
        inner = tauri::LogicalSize::new(width, height).to_physical(window.scale_factor()?);
    }
    let (frame_w, frame_h) = frame;
    let width = inner.width.min(area.size.width.saturating_sub(frame_w));
    let height = inner.height.min(area.size.height.saturating_sub(frame_h));
    window.set_size(tauri::PhysicalSize::new(width, height))?;
    window.set_position(area.position)
}

#[tauri::command]
pub fn ui_test_scenario(test: State<'_, UiTest>) -> Option<String> {
    test.scenario.clone()
}

/// Writes what the page reports (where it drew the rows, the language,
/// the device pixel ratio) next to the screenshot.
#[tauri::command]
pub fn ui_test_report(test: State<'_, UiTest>, report: serde_json::Value) -> Result<(), String> {
    let (Some(out), Some(scenario)) = (&test.out, &test.scenario) else {
        return Ok(());
    };
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let text = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
    // Written then renamed, so the script never reads half a file.
    let tmp = out.join(format!("{scenario}.json.tmp"));
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, out.join(format!("{scenario}.json"))).map_err(|e| e.to_string())
}

const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;

#[allow(clippy::too_many_arguments)] // a table of demo rows
fn d(
    id: u64,
    name: &str,
    size: u64,
    done: f64,
    speed: f64,
    status: StatusView,
    category: &str,
    host: &str,
    parts: &[f64],
) -> DownloadView {
    DownloadView {
        id,
        name: name.to_owned(),
        host: Some(host.to_owned()),
        size: Some(size),
        done: (size as f64 * done) as u64,
        speed,
        status,
        queue: if id == 5 {
            "night".into()
        } else {
            "main".into()
        },
        category: Some(category.to_owned()),
        folder: "~/Downloads".into(),
        parts: parts.to_vec(),
        // A demo row's segments are its feathers.
        segments: parts.len().max(1) as u32,
        error: (status == StatusView::Failed).then(|| "the server answered 404 Not Found".into()),
        // «Last try» and «Description» (#95), on a few rows.
        last_try: (status != StatusView::Waiting)
            .then(|| format!("2026-10-25T{:02}:{:02}:00Z", 6 + id % 7, (id * 13) % 60)),
        description: match id {
            1 => Some("للتثبيت على جهاز المكتب".into()),
            3 => Some("Q3 sales, for the board".into()),
            _ => None,
        },
        // The night queue's waiting archive came from a browser that is
        // not open (M5).
        waits_for_browser: id == 5,
        waits_for_site: false,
        needs_ok: false,
        video: false,
    }
}

/// The demo list of the approved design: Arabic, English and numbers mixed
/// in the names, every status.
pub fn demo_snapshot() -> Snapshot {
    use StatusView::*;
    let downloads = vec![
        d(
            1,
            "Ubuntu 24.04.3 desktop amd64.iso",
            6 * GIB + GIB / 10,
            0.62,
            4.8 * MIB as f64,
            Active,
            "programs",
            "releases.ubuntu.com",
            &[1.0, 1.0, 0.74, 0.81, 0.66, 0.52, 0.70, 0.58],
        ),
        d(
            2,
            "محاضرة 12 - الخوارزميات (Part 2).mp4",
            1400 * MIB,
            0.31,
            2.1 * MIB as f64,
            Active,
            "video",
            "lectures.example.edu",
            &[0.62, 0.35, 0.30, 0.28, 0.22, 0.25, 0.27, 0.19],
        ),
        d(
            3,
            "تقرير المبيعات Q3-2026 النهائي.pdf",
            18 * MIB,
            1.0,
            0.0,
            Done,
            "documents",
            "drive.example.com",
            &[1.0],
        ),
        d(
            4,
            "أغاني فيروز - Live 1971.flac",
            512 * MIB,
            0.48,
            0.0,
            Paused,
            "music",
            "archive.example.org",
            &[0.9, 0.4, 0.3, 0.32],
        ),
        d(
            5,
            "صور الرحلة 2025 (الجزء 3).zip",
            2 * GIB + 700 * MIB,
            0.0,
            0.0,
            Waiting,
            "archives",
            "files.example.net",
            &[0.0; 8],
        ),
        d(
            6,
            "zajil-0.4.0-arm64.dmg",
            14 * MIB,
            0.12,
            0.0,
            Failed,
            "programs",
            "github.com",
            &[0.2, 0.04],
        ),
        d(
            7,
            "ديوان المتنبي (طبعة 2019).epub",
            3 * MIB,
            1.0,
            0.0,
            Done,
            "documents",
            "books.example.org",
            &[1.0],
        ),
        d(
            8,
            "Blender 4.5 تعليم النمذجة.mkv",
            3900 * MIB,
            0.0,
            0.0,
            Waiting,
            "video",
            "cdn.example.com",
            &[0.0; 8],
        ),
        d(
            9,
            "backup_2026-10-20 النسخة الكاملة.tar.gz",
            9800 * MIB,
            0.07,
            6.3 * MIB as f64,
            Active,
            "archives",
            "nas.local",
            &[0.14, 0.12, 0.11, 0.1, 0.09, 0.08, 0.1, 0.07],
        ),
    ];
    Snapshot {
        downloads,
        queues: vec![
            QueueView {
                name: "main".into(),
                max_active: 3,
                active: 3,
                window: None,
            },
            QueueView {
                name: "night".into(),
                max_active: 2,
                active: 0,
                window: Some(("02:00".into(), "07:00".into())),
            },
        ],
        categories: zajil_core::category::CategoriesFile::built_in()
            .categories
            .into_iter()
            .map(|c| crate::view::CategoryView {
                id: c.id,
                name: c.name,
                extensions: c.extensions,
                sites: c.sites,
                folder: None,
            })
            .collect(),
        speed_limit: None,
        problem: None,
        categories_problem: None,
    }
}

pub fn demo_details(id: u64) -> Option<DetailsView> {
    let download = demo_snapshot().downloads.into_iter().find(|d| d.id == id)?;
    let size = download.size.unwrap_or(0);
    let n = download.parts.len() as u64;
    let segments = download
        .parts
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let start = size / n * i as u64;
            let end = if i as u64 == n - 1 {
                size - 1
            } else {
                size / n * (i as u64 + 1) - 1
            };
            SegmentView {
                start,
                end: Some(end),
                done: ((end - start + 1) as f64 * f) as u64,
            }
        })
        .collect();
    let url = format!(
        "https://{}/{}",
        download.host.clone().unwrap_or_default(),
        download.name
    );
    Some(DetailsView {
        url,
        download,
        segments,
        resumable: Some(true),
        limit: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|a| (*a).to_owned()).collect()
    }

    /// Review of #47: a `ui-test` build without `--ui-test` showed the
    /// demo list while the real queues ran.
    #[test]
    fn the_demo_list_is_shown_only_in_a_scene() {
        assert!(
            UiTest::from_args(&args(&["zajil-desktop"]))
                .demo()
                .is_none()
        );
        let scene = UiTest::from_args(&args(&["zajil-desktop", "--ui-test", "names"]));
        assert!(!scene.demo().expect("the demo list").downloads.is_empty());
    }
}
