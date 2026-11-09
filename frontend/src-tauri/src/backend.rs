use std::fs::{self, File};
use std::net::TcpListener;
use std::os::windows::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Manager, State};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendInfo {
    base_url: String,
    token: String,
}

pub struct Backend {
    info: Result<BackendInfo, String>,
    child: Mutex<Option<Child>>,
}

impl Backend {
    pub fn start(app: &AppHandle) -> Self {
        match launch(app) {
            Ok((info, child)) => Self { info: Ok(info), child: Mutex::new(Some(child)) },
            Err(message) => Self { info: Err(message), child: Mutex::new(None) },
        }
    }

    pub fn shutdown(&self) {
        if let Some(mut child) = self.child.lock().unwrap().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[tauri::command]
pub fn backend_info(backend: State<'_, Backend>) -> Result<BackendInfo, String> {
    backend.info.clone()
}

#[cfg(debug_assertions)]
fn backend_command(_app: &AppHandle) -> Result<Command, String> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../backend");
    let python = dir.join(".venv/Scripts/python.exe");
    if !python.exists() {
        return Err(format!("Python environment not found at {}", python.display()));
    }
    let mut command = Command::new(python);
    command.args(["-m", "app"]).current_dir(dir);
    Ok(command)
}

#[cfg(not(debug_assertions))]
fn backend_command(_app: &AppHandle) -> Result<Command, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let sidecar = exe.with_file_name("tutor-backend.exe");
    if !sidecar.exists() {
        return Err(format!("Backend executable not found at {}", sidecar.display()));
    }
    Ok(Command::new(sidecar))
}

fn launch(app: &AppHandle) -> Result<(BackendInfo, Child), String> {
    let port = TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .map_err(|e| format!("Could not reserve a local port: {e}"))?
        .port();
    let token = uuid::Uuid::new_v4().simple().to_string();

    let log_dir = app.path().app_log_dir().map_err(|e| e.to_string())?;
    fs::create_dir_all(&log_dir).map_err(|e| e.to_string())?;
    let log = File::create(log_dir.join("backend.log")).map_err(|e| e.to_string())?;
    let log_err = log.try_clone().map_err(|e| e.to_string())?;

    let child = backend_command(app)?
        .env("TUTOR_PORT", port.to_string())
        .env("TUTOR_API_TOKEN", &token)
        .env("TUTOR_WATCH_STDIN", "1")
        // Held open for the shell's lifetime; the backend exits when it closes.
        .stdin(Stdio::piped())
        .stdout(log)
        .stderr(log_err)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| format!("Could not start the backend: {e}"))?;

    Ok((BackendInfo { base_url: format!("http://127.0.0.1:{port}"), token }, child))
}
