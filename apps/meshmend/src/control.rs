use std::{
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    sync::mpsc::{self, Sender},
    thread,
    time::Duration,
};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

use crate::commands::{AppCommand, StateSnapshot};

const CONTROL_RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub struct ControlSocketConfig {
    pub path: PathBuf,
    pub replace_existing: bool,
}

#[derive(Debug, Clone)]
pub struct ControlEvent {
    pub request: ControlRequest,
    pub response_tx: Sender<ControlResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlRequest {
    pub id: u64,
    pub command: AppCommand,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlResponse {
    pub id: u64,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<StateSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ControlResponse {
    pub fn ok(id: u64, status: impl Into<String>, state: StateSnapshot) -> Self {
        Self {
            id,
            ok: true,
            status: Some(status.into()),
            state: Some(state),
            error: None,
        }
    }

    pub fn error(id: u64, error: impl Into<String>) -> Self {
        Self {
            id,
            ok: false,
            status: None,
            state: None,
            error: Some(error.into()),
        }
    }
}

pub struct ControlSocketGuard {
    path: PathBuf,
}

impl Drop for ControlSocketGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub fn resolve_control_socket(value: &str) -> PathBuf {
    if value == "auto" {
        PathBuf::from("target")
            .join("meshmend-control")
            .join(format!("meshmend-{}.sock", std::process::id()))
    } else {
        PathBuf::from(value)
    }
}

#[cfg(unix)]
pub fn spawn_control_socket<F>(
    config: ControlSocketConfig,
    dispatch: F,
) -> Result<ControlSocketGuard>
where
    F: Fn(ControlEvent) -> Result<()> + Send + 'static,
{
    use std::os::unix::net::UnixListener;

    if let Some(parent) = config
        .path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    if config.path.exists() {
        if config.replace_existing {
            std::fs::remove_file(&config.path)?;
        } else {
            return Err(anyhow!(
                "control socket {} already exists; pass --replace-control-socket to replace it",
                config.path.display()
            ));
        }
    }

    let listener = UnixListener::bind(&config.path)
        .with_context(|| format!("bind control socket {}", config.path.display()))?;
    let path = config.path.clone();
    thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    if let Err(err) = handle_control_stream(stream, &dispatch) {
                        tracing::warn!(error = %err, "control request failed");
                    }
                }
                Err(err) => {
                    tracing::warn!(error = %err, "control socket accept failed");
                    break;
                }
            }
        }
    });
    Ok(ControlSocketGuard { path })
}

#[cfg(not(unix))]
pub fn spawn_control_socket<F>(
    _config: ControlSocketConfig,
    _dispatch: F,
) -> Result<ControlSocketGuard>
where
    F: Fn(ControlEvent) -> Result<()> + Send + 'static,
{
    Err(anyhow!(
        "control sockets are currently supported on Unix platforms only"
    ))
}

#[cfg(unix)]
fn handle_control_stream<F>(mut stream: std::os::unix::net::UnixStream, dispatch: &F) -> Result<()>
where
    F: Fn(ControlEvent) -> Result<()> + Send + 'static,
{
    let mut line = String::new();
    BufReader::new(stream.try_clone()?).read_line(&mut line)?;
    let request: ControlRequest = serde_json::from_str(line.trim())?;
    let (response_tx, response_rx) = mpsc::channel();
    dispatch(ControlEvent {
        request: request.clone(),
        response_tx,
    })?;
    let response = response_rx
        .recv_timeout(CONTROL_RESPONSE_TIMEOUT)
        .unwrap_or_else(|_| ControlResponse::error(request.id, "control request timed out"));
    writeln!(stream, "{}", serde_json::to_string(&response)?)?;
    Ok(())
}

#[cfg(unix)]
pub fn send_control_command(socket: PathBuf, command: AppCommand) -> Result<ControlResponse> {
    use std::os::unix::net::UnixStream;

    let mut stream = UnixStream::connect(&socket)
        .with_context(|| format!("connect control socket {}", socket.display()))?;
    let request = ControlRequest { id: 1, command };
    writeln!(stream, "{}", serde_json::to_string(&request)?)?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line)?;
    let response: ControlResponse = serde_json::from_str(line.trim())?;
    Ok(response)
}

#[cfg(not(unix))]
pub fn send_control_command(_socket: PathBuf, _command: AppCommand) -> Result<ControlResponse> {
    Err(anyhow!(
        "control sockets are currently supported on Unix platforms only"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_socket_path_contains_process_id() {
        let path = resolve_control_socket("auto");
        let text = path.to_string_lossy();

        assert!(text.contains("target/meshmend-control"));
        assert!(text.contains(&std::process::id().to_string()));
    }

    #[test]
    fn request_round_trips_json() {
        let request = ControlRequest {
            id: 7,
            command: AppCommand::FitCamera,
        };

        let json = serde_json::to_string(&request).expect("serialize");
        let parsed: ControlRequest = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(parsed.id, 7);
        assert!(matches!(parsed.command, AppCommand::FitCamera));
    }
}
