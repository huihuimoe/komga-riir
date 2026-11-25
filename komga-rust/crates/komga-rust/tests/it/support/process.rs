//! `komga-rust` as a separate process, for tests that kill it.

use std::fs::{self, File};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use reqwest::header::SET_COOKIE;
use reqwest::{Client, StatusCode};
use serde_json::{Value, json};
use tokio::time;

use crate::support::{ADMIN_EMAIL, ADMIN_PASSWORD};

/// A `komga-rust` process logging to a file in the data directory.
pub struct Server {
    child: Child,
    pub base_url: String,
    log: PathBuf,
}

impl Server {
    pub async fn start(directory: &Path, port: u16, log_name: &str) -> Self {
        let log = directory.join(log_name);
        let output = File::create(&log).expect("Log file");
        let child = Command::new(env!("CARGO_BIN_EXE_komga-rust"))
            .args([
                "--host",
                "127.0.0.1",
                "--port",
                &port.to_string(),
                "--config-dir",
            ])
            .arg(directory)
            .stdout(Stdio::from(output.try_clone().expect("Log handle")))
            .stderr(Stdio::from(output))
            .spawn()
            .expect("Start komga-rust");
        let server = Self {
            child,
            base_url: format!("http://127.0.0.1:{port}"),
            log,
        };
        time::timeout(Duration::from_secs(10), async {
            while reqwest::get(format!("{}/api/v1/claim", server.base_url))
                .await
                .is_err()
            {
                time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("komga-rust listens within ten seconds");
        server
    }

    /// Kills the process without a graceful shutdown and returns its log without ANSI styling.
    pub fn kill(mut self) -> String {
        self.child.kill().expect("SIGKILL komga-rust");
        self.child.wait().expect("Reap komga-rust");
        let log = fs::read_to_string(&self.log).expect("Read log");
        let mut plain = String::with_capacity(log.len());
        let mut chars = log.chars();
        while let Some(character) = chars.next() {
            if character == '\u{1b}' {
                chars.by_ref().find(|character| *character == 'm');
            } else {
                plain.push(character);
            }
        }
        plain
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        // A failed assertion must not leave the process holding the port and data directory.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .expect("Free port")
        .port()
}

pub async fn create_library(client: &Client, server: &Server, root: &Path) -> String {
    fs::create_dir_all(root).expect("Library root");
    let response = client
        .post(format!("{}/api/v1/libraries", server.base_url))
        .basic_auth(ADMIN_EMAIL, Some(ADMIN_PASSWORD))
        .json(&json!({"name": root.file_name().expect("Name").to_string_lossy(), "root": root}))
        .send()
        .await
        .expect("Create library");
    let library: Value = response.json().await.expect("Library JSON");
    library["id"].as_str().expect("Library ID").to_owned()
}

/// Claims the server and returns a session cookie, so later requests do not write to the main
/// database.
pub async fn admin_session(client: &Client, server: &Server) -> String {
    let response = client
        .post(format!("{}/api/v1/claim", server.base_url))
        .header("X-Komga-Email", ADMIN_EMAIL)
        .header("X-Komga-Password", ADMIN_PASSWORD)
        .send()
        .await
        .expect("Claim");
    assert_eq!(response.status(), StatusCode::OK);
    let response = client
        .get(format!("{}/api/v2/users/me", server.base_url))
        .basic_auth(ADMIN_EMAIL, Some(ADMIN_PASSWORD))
        .send()
        .await
        .expect("Admin session");
    let cookie = response.headers()[SET_COOKIE]
        .to_str()
        .expect("ASCII cookie");
    cookie.split(';').next().expect("Cookie pair").to_owned()
}
