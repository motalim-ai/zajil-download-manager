//! Which sounds Zajil plays (M5.5, an IDM touch): when a download
//! finishes, when one fails, and when a queue finishes. The page plays
//! them (tones it makes itself, no sound files); these are the user's
//! choices, kept with the window preferences (`window.json`). At first
//! only the finished download's sound is on.

use serde::{Deserialize, Serialize};
use tauri::State;
use ts_rs::TS;

use crate::closing::Closing;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Sounds {
    #[serde(default = "yes")]
    pub done: bool,
    #[serde(default)]
    pub failed: bool,
    #[serde(default)]
    pub queue: bool,
}

fn yes() -> bool {
    true
}

impl Default for Sounds {
    fn default() -> Self {
        Sounds {
            done: true,
            failed: false,
            queue: false,
        }
    }
}

/// A sound, as the page names it (`sounds.ts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sound {
    Done,
    Failed,
    Queue,
}

impl Sound {
    /// Its name in the page.
    pub fn name(self) -> &'static str {
        match self {
            Sound::Done => "done",
            Sound::Failed => "failed",
            Sound::Queue => "queue",
        }
    }
}

/// Plays `sound` while the main window is freed in the tray (`in_tray.rs`;
/// the page plays it otherwise): the page's own tones (`sounds.ts`), in a
/// window made for them alone, never shown, gone a few seconds later. No
/// sound library, and no page kept for a sound now and then.
pub fn play<R: tauri::Runtime>(handle: &tauri::AppHandle<R>, sound: Sound) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let label = format!("sound-{}", NEXT.fetch_add(1, Ordering::Relaxed));
    let page = format!("index.html?win=sound&sound={}", sound.name());
    let handle = handle.clone();
    // Not on the main thread (`windows::open`).
    tauri::async_runtime::spawn(async move {
        let made =
            tauri::WebviewWindowBuilder::new(&handle, &label, tauri::WebviewUrl::App(page.into()))
                .visible(false)
                .focused(false)
                .skip_taskbar(true)
                .inner_size(1.0, 1.0)
                // Its page loaded: the tones (under half a second) start
                // now, and the window goes a second later.
                .on_page_load(|window, payload| {
                    if payload.event() == tauri::webview::PageLoadEvent::Finished {
                        tauri::async_runtime::spawn(async move {
                            tokio::time::sleep(AFTER_LOAD).await;
                            let _ = window.destroy();
                        });
                    }
                })
                .build();
        match made {
            // A page that never says it loaded goes all the same.
            Ok(window) => {
                tokio::time::sleep(AT_MOST).await;
                let _ = window.destroy();
            }
            Err(err) => eprintln!("zajil: no sound: {err}"),
        }
    });
}

/// How long a sound's window stays once its page loaded (the owner's
/// choice: about a second).
const AFTER_LOAD: std::time::Duration = std::time::Duration::from_secs(1);
/// How long it stays at most.
const AT_MOST: std::time::Duration = std::time::Duration::from_secs(5);

#[tauri::command]
pub fn sound_prefs(closing: State<'_, Closing>) -> Sounds {
    closing.prefs().sounds
}

#[tauri::command]
pub fn set_sound_prefs(closing: State<'_, Closing>, sounds: Sounds) -> Result<(), String> {
    closing
        .change(|p| p.sounds = sounds)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// At first, and in a preferences file from before the sounds, only
    /// the finished download's sound is on.
    #[test]
    fn only_the_finished_sound_is_on_at_first() {
        assert_eq!(
            Sounds::default(),
            Sounds {
                done: true,
                failed: false,
                queue: false
            }
        );
        let old: crate::closing::WindowPrefs =
            serde_json::from_str(r#"{"close_quits":false}"#).unwrap();
        assert_eq!(old.sounds, Sounds::default());
        let some: Sounds = serde_json::from_str(r#"{"failed":true}"#).unwrap();
        assert_eq!(
            some,
            Sounds {
                done: true,
                failed: true,
                queue: false
            }
        );
    }
}
