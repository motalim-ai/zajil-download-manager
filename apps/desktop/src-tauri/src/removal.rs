//! Taking downloads off the list with their unfinished files (M5.5, an IDM
//! touch): the part and state files of a download that has not finished
//! go too; a finished file always stays. The runner stops a download once
//! it is off the list, so the files are deleted a little later, trying
//! again while it still has them open.

use std::path::PathBuf;

use zajil_core::queue::QueuesFile;

/// The unfinished files of `ids` (their state and part files), read before
/// they are taken off the list.
pub fn files_of(queues: &QueuesFile, ids: &[u64]) -> Vec<PathBuf> {
    ids.iter()
        .filter_map(|id| queues.item(*id))
        .filter(|item| item.file.is_none())
        .filter_map(|item| item.state_file.as_deref())
        .flat_map(zajil_core::queue::private_files)
        .collect()
}

/// Deletes `files`, trying again for up to 10 s while they are still open.
pub fn delete_soon(files: Vec<PathBuf>) {
    if files.is_empty() {
        return;
    }
    std::thread::spawn(move || {
        let mut left = files;
        for _ in 0..20 {
            left.retain(|f| f.exists() && std::fs::remove_file(f).is_err());
            if left.is_empty() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        for f in left {
            eprintln!("zajil: could not delete {}", f.display());
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only an unfinished download's files are deleted: a finished file
    /// stays.
    #[test]
    fn only_unfinished_files_are_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("a.bin.zajil-state");
        let part = dir.path().join("a.bin.zajil-part");
        let finished = dir.path().join("b.bin");
        for f in [&state, &part, &finished] {
            std::fs::write(f, b"x").unwrap();
        }
        let mut queues = QueuesFile::empty();
        let id = queues
            .add("main", "https://example.com/a.bin", dir.path())
            .unwrap();
        queues.item_mut(id).unwrap().state_file = Some(state.clone());
        let done = queues
            .add("main", "https://example.com/b.bin", dir.path())
            .unwrap();
        queues.item_mut(done).unwrap().file = Some(finished.clone());
        let files = files_of(&queues, &[id, done]);
        assert_eq!(files, vec![state.clone(), part.clone()]);
        delete_soon(files);
        for _ in 0..50 {
            if !state.exists() && !part.exists() {
                assert!(finished.exists());
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        panic!("the unfinished files were not deleted");
    }
}
