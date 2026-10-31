//! The test server for the app on the emulator (android/README.md): the
//! local `zajil-testserver`, never the internet.
//!
//!   cargo run -p zajil-android --example emulator_server -- <log file>
//!   adb reverse tcp:8099 tcp:8099
//!
//! The app reaches it at 127.0.0.1 on the device, through `adb reverse`, not
//! at the emulator's 10.0.2.2: from Android 17 an app needs the "local
//! network" permission for a private address, and Zajil does not ask for it.
//!
//! - `http://127.0.0.1:8099/<random folder>/file.bin` (printed at start): 50
//!   MB of random bytes, each connection slowed to 256 KiB/s, so that a
//!   pause or a kill lands in the middle.
//! - An HTTPS port with a self-signed certificate, printed at start with its
//!   own `adb reverse` line: the app must refuse it.
//!
//! Every request (its range and the bytes sent) is written to the log file
//! every second, to check that a resume asks only for what is missing.

use std::io::Write;
use std::time::Duration;

use sha2::{Digest, Sha256};
use zajil_testserver::{FileSpec, TestServer};

fn main() {
    let log = std::env::args()
        .nth(1)
        .expect("usage: emulator_server <log file>");
    let mut body = vec![0u8; 50 * 1024 * 1024];
    getrandom::fill(&mut body).unwrap();
    let sha = Sha256::digest(&body);

    let server = TestServer::http_on(([127, 0, 0, 1], 8099).into()).expect("port 8099");
    server.put("/file.bin", FileSpec::new(body).throttle(256 * 1024));
    let tls = TestServer::https_self_signed(&["127.0.0.1"], "127.0.0.1");
    tls.put("/file.bin", FileSpec::new(vec![7u8; 1024]));

    println!("sha256 {}", hex(&sha));
    // The server puts a random folder in front of every path.
    println!("http  {}", server.url("/file.bin"));
    println!("https {}", tls.url("/file.bin"));
    println!("adb reverse tcp:{0} tcp:{0}", tls.addr().port());
    loop {
        std::thread::sleep(Duration::from_secs(1));
        let mut out = std::fs::File::create(&log).unwrap();
        for request in server.requests() {
            writeln!(
                out,
                "{} {} range={} sent={} status={}",
                request.method,
                request.path,
                request.range.as_deref().unwrap_or("-"),
                request.bytes_sent,
                request.status,
            )
            .unwrap();
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
