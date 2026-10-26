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
