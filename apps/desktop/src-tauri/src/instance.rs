//! One Zajil per user: a second start asks the running one to show its
//! window (#60).

use std::path::Path;
use std::time::Duration;

/// Whether a Zajil already runs on the data folder `data`: then it is
/// asked, over its private channel, to show its window (#60). On macOS and
/// Linux the channel is a socket in a folder of the data folder that only
/// this user owns and can enter (checked at every connect); on Windows the
/// client checks who serves the pipe. Nothing answering, or anything going
/// wrong: this one starts.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn show_running(data: &Path) -> bool {
    let Ok(mut stream) = zajil_platform::ipc::connect(data, Duration::from_millis(300)) else {
        return false;
    };
    let ask = zajil_host::Request::ShowWindow {
        protocol_version: zajil_host::PROTOCOL_VERSION,
    };
    if zajil_host::write_json(&mut stream, &ask).is_err() {
        return false;
    }
    // A Zajil that answered is there, whatever it said; one that does not
    // answer in time is there too (it holds the channel).
    let (done, wait) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = done.send(zajil_host::read_frame(&mut stream, 4096).is_ok());
    });
    let _ = wait.recv_timeout(Duration::from_secs(2));
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #60: Zajil started again on a data folder where it runs asks the
    /// running one, over its channel, to show its window, and says so; on
    /// a folder where nothing runs, it starts.
    #[test]
    fn a_second_start_asks_the_running_zajil_to_show_its_window() {
        let data = tempfile::tempdir().unwrap();
        assert!(!show_running(data.path()), "nothing runs there");
        let listener = zajil_platform::ipc::Listener::bind(data.path()).unwrap();
        let (seen, saw) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            zajil_host::serve(
                listener,
                move |request| {
                    let _ = seen.send(request);
                    zajil_host::Response::ok()
                },
                Default::default(),
            );
        });
        // On Windows the pipe exists once the serving thread waits on it:
        // asked a few times, not once (review of #243).
        let mut shown = false;
        for _ in 0..50 {
            shown = show_running(data.path());
            if shown {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(shown);
        assert_eq!(
            saw.recv_timeout(Duration::from_secs(5)).unwrap(),
            zajil_host::Request::ShowWindow {
                protocol_version: zajil_host::PROTOCOL_VERSION
            }
        );
    }
}
