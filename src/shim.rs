use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::state::ToolFilter;

pub const DEFAULT_SOCKET_PATH: &str = "/tmp/tailery.sock";

#[derive(Error, Debug)]
pub enum ShimError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Child process error: {0}")]
    Process(String),
    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Structured telemetry event emitted across the Unix domain socket to the TUI.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[derive(PartialEq)]
pub enum TelemetryMessage {
    Request {
        server: String,
        id: Value,
        method: String,
        params: Option<Value>,
        timestamp: String,
    },
    Response {
        server: String,
        id: Value,
        result: Option<Value>,
        error: Option<Value>,
        timestamp: String,
        duration_ms: Option<u64>,
    },
    Notification {
        server: String,
        method: String,
        params: Option<Value>,
        timestamp: String,
    },
    Log {
        server: String,
        level: String,
        message: String,
        timestamp: String,
    },
    Status {
        server: String,
        status: String,
        pid: Option<u32>,
        timestamp: String,
    },
    Blocked {
        server: String,
        tool: String,
        reason: String,
        timestamp: String,
    },
}

/// Asynchronous non-blocking telemetry emitter connected to Tailery's Unix socket.
#[derive(Clone)]
pub struct TelemetryClient {
    tx: mpsc::UnboundedSender<TelemetryMessage>,
}

impl TelemetryClient {
    pub fn new(socket_path: Option<&str>) -> Self {
        let (tx, mut rx) = mpsc::unbounded_channel::<TelemetryMessage>();
        let sock_path = socket_path
            .map(String::from)
            .or_else(|| std::env::var("TAILERY_SOCKET").ok())
            .unwrap_or_else(|| DEFAULT_SOCKET_PATH.to_string());

        let spawn_task = async move {
            let mut stream: Option<UnixStream> = None;

            while let Some(msg) = rx.recv().await {
                if stream.is_none() {
                    stream = UnixStream::connect(&sock_path).await.ok();
                }

                if let Some(s) = stream.as_mut()
                    && let Ok(serialized) = serde_json::to_string(&msg)
                {
                    let line = format!("{}\n", serialized);
                    if s.write_all(line.as_bytes()).await.is_err() {
                        stream = None;
                    }
                }
            }
        };

        if tokio::runtime::Handle::try_current().is_ok() {
            tokio::spawn(spawn_task);
        }

        Self { tx }
    }

    pub fn emit(&self, message: TelemetryMessage) {
        let _ = self.tx.send(message);
    }
}

/// The headless interceptor that wraps an MCP server command.
pub struct ShimInterceptor {
    pub server_name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub tool_filter: ToolFilter,
    pub telemetry: TelemetryClient,
}

impl ShimInterceptor {
    pub fn new(
        server_name: String,
        command: String,
        args: Vec<String>,
        env: HashMap<String, String>,
        tool_filter: ToolFilter,
        socket_path: Option<&str>,
    ) -> Self {
        let telemetry = TelemetryClient::new(socket_path);
        Self {
            server_name,
            command,
            args,
            env,
            tool_filter,
            telemetry,
        }
    }

    /// Check if a tool execution is permitted under current policy.
    #[allow(dead_code)]
    pub fn is_tool_allowed(&self, tool_name: &str) -> bool {
        if self
            .tool_filter
            .deny
            .iter()
            .any(|d| d == tool_name || d == "*")
        {
            return false;
        }
        if !self.tool_filter.allow.is_empty() {
            return self
                .tool_filter
                .allow
                .iter()
                .any(|a| a == tool_name || a == "*");
        }
        true
    }

    /// Run the interceptor proxying stdio between the client and child MCP server.
    pub async fn run(&self) -> Result<i32, ShimError> {
        let now_str = Utc::now().to_rfc3339();
        self.telemetry.emit(TelemetryMessage::Status {
            server: self.server_name.to_string(),
            status: "starting".to_string(),
            pid: None,
            timestamp: now_str.clone(),
        });

        let mut child = Command::new(&self.command)
            .args(&self.args)
            .envs(&self.env)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| ShimError::Process(format!("Failed to spawn {}: {}", self.command, e)))?;

        let pid = child.id();
        self.telemetry.emit(TelemetryMessage::Status {
            server: self.server_name.to_string(),
            status: "running".to_string(),
            pid,
            timestamp: Utc::now().to_rfc3339(),
        });

        let child_stdin = child.stdin.take().expect("Child stdin unavailable");
        let child_stdout = child.stdout.take().expect("Child stdout unavailable");
        let child_stderr = child.stderr.take().expect("Child stderr unavailable");

        let mut child_stdin_writer = child_stdin;
        let mut child_stdout_reader = BufReader::new(child_stdout).lines();
        let mut child_stderr_reader = BufReader::new(child_stderr).lines();

        let mut host_stdin_reader = BufReader::new(tokio::io::stdin()).lines();
        let mut host_stdout_writer = tokio::io::stdout();
        let mut host_stderr_writer = tokio::io::stderr();

        let server_name = self.server_name.to_string();
        let telemetry = self.telemetry.clone();
        let filter = self.tool_filter.clone();

        // In-flight request tracking for latency metrics
        let pending_requests: Arc<tokio::sync::Mutex<HashMap<String, Instant>>> =
            Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let pending_requests_clone = pending_requests.clone();

        // Loop handling stdio forwarding and interception
        loop {
            tokio::select! {
                // Read from AI Client Stdin -> Forward to Child
                line_res = host_stdin_reader.next_line() => {
                    match line_res {
                        Ok(Some(line)) => {
                            if let Ok(val) = serde_json::from_str::<Value>(&line) {
                                let id = val.get("id").cloned();
                                let method = val.get("method").and_then(|m| m.as_str()).unwrap_or_default().to_string();
                                let params = val.get("params").cloned();

                                if let Some(ref req_id) = id {
                                    pending_requests.lock().await.insert(req_id.to_string(), Instant::now());
                                    telemetry.emit(TelemetryMessage::Request {
                                        server: server_name.to_string(),
                                        id: req_id.clone(),
                                        method: method.clone(),
                                        params: params.clone(),
                                        timestamp: Utc::now().to_rfc3339(),
                                    });
                                } else if !method.is_empty() {
                                    telemetry.emit(TelemetryMessage::Notification {
                                        server: server_name.to_string(),
                                        method: method.clone(),
                                        params: params.clone(),
                                        timestamp: Utc::now().to_rfc3339(),
                                    });
                                }

                                // Security Tool Filtering Check
                                if method == "tools/call" {
                                    let tool_name = params.as_ref()
                                        .and_then(|p| p.get("name"))
                                        .and_then(|n| n.as_str())
                                        .unwrap_or_default();

                                    let is_allowed = {
                                        if filter.deny.iter().any(|d| d == tool_name || d == "*") {
                                            false
                                        } else if !filter.allow.is_empty() {
                                            filter.allow.iter().any(|a| a == tool_name || a == "*")
                                        } else {
                                            true
                                        }
                                    };

                                    if !is_allowed {
                                        telemetry.emit(TelemetryMessage::Blocked {
                                            server: server_name.to_string(),
                                            tool: tool_name.to_string(),
                                            reason: "Blocked by Tailery tool filter policy".to_string(),
                                            timestamp: Utc::now().to_rfc3339(),
                                        });

                                        if let Some(req_id) = id {
                                            let error_response = json!({
                                                "jsonrpc": "2.0",
                                                "id": req_id,
                                                "error": {
                                                    "code": -32000,
                                                    "message": format!("Tool '{}' execution blocked by Tailery security policy", tool_name)
                                                }
                                            });
                                            let resp_line = format!("{}\n", serde_json::to_string(&error_response)?);
                                            host_stdout_writer.write_all(resp_line.as_bytes()).await?;
                                            host_stdout_writer.flush().await?;
                                            continue;
                                        }
                                    }
                                }
                            }

                            let to_write = format!("{}\n", line);
                            child_stdin_writer.write_all(to_write.as_bytes()).await?;
                            child_stdin_writer.flush().await?;
                        }
                        Ok(None) => break, // Client closed stdin
                        Err(e) => {
                            eprintln!("Error reading stdin: {}", e);
                            break;
                        }
                    }
                }

                // Read from Child Stdout -> Forward to AI Client
                line_res = child_stdout_reader.next_line() => {
                    match line_res {
                        Ok(Some(line)) => {
                            if let Ok(val) = serde_json::from_str::<Value>(&line) {
                                let id = val.get("id").cloned();
                                let result = val.get("result").cloned();
                                let error = val.get("error").cloned();
                                let method = val.get("method").and_then(|m| m.as_str());

                                if let Some(ref req_id) = id {
                                    let duration = pending_requests_clone.lock().await.remove(&req_id.to_string()).map(|start| start.elapsed().as_millis() as u64);
                                    telemetry.emit(TelemetryMessage::Response {
                                        server: server_name.to_string(),
                                        id: req_id.clone(),
                                        result,
                                        error,
                                        timestamp: Utc::now().to_rfc3339(),
                                        duration_ms: duration,
                                    });
                                } else if let Some(m) = method {
                                    telemetry.emit(TelemetryMessage::Notification {
                                        server: server_name.to_string(),
                                        method: m.to_string(),
                                        params: val.get("params").cloned(),
                                        timestamp: Utc::now().to_rfc3339(),
                                    });
                                }
                            }

                            let to_write = format!("{}\n", line);
                            host_stdout_writer.write_all(to_write.as_bytes()).await?;
                            host_stdout_writer.flush().await?;
                        }
                        Ok(None) => break, // Child closed stdout
                        Err(e) => {
                            eprintln!("Error reading child stdout: {}", e);
                            break;
                        }
                    }
                }

                // Read from Child Stderr -> Forward to Host Stderr & Telemetry
                line_res = child_stderr_reader.next_line() => {
                    match line_res {
                        Ok(Some(line)) => {
                            telemetry.emit(TelemetryMessage::Log {
                                server: server_name.to_string(),
                                level: "stderr".to_string(),
                                message: line.clone(),
                                timestamp: Utc::now().to_rfc3339(),
                            });
                            let to_write = format!("{}\n", line);
                            host_stderr_writer.write_all(to_write.as_bytes()).await?;
                            host_stderr_writer.flush().await?;
                        }
                        Ok(None) => break,
                        Err(e) => {
                            eprintln!("Error reading child stderr: {}", e);
                            break;
                        }
                    }
                }

                // Child exit check
                status_res = child.wait() => {
                    let exit_code = match status_res {
                        Ok(status) => status.code().unwrap_or(0),
                        Err(_) => 1,
                    };
                    telemetry.emit(TelemetryMessage::Status {
                        server: server_name.to_string(),
                        status: format!("exited ({})", exit_code),
                        pid,
                        timestamp: Utc::now().to_rfc3339(),
                    });
                    return Ok(exit_code);
                }
            }
        }

        let status = child
            .wait()
            .await
            .map_err(|e| ShimError::Process(e.to_string()))?;
        Ok(status.code().unwrap_or(0))
    }
}

use axum::{
    Router,
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::any,
};
use std::net::SocketAddr;

#[derive(Clone)]
pub struct HttpShimState {
    pub server_name: String,
    pub remote_url: String,
    pub tool_filter: ToolFilter,
    pub telemetry: TelemetryClient,
    pub http_client: reqwest::Client,
}

pub struct HttpShimInterceptor {
    pub state: HttpShimState,
}

impl HttpShimInterceptor {
    pub fn new(
        server_name: String,
        remote_url: String,
        tool_filter: ToolFilter,
        socket_path: Option<&str>,
    ) -> Self {
        let telemetry = TelemetryClient::new(socket_path);

        /// How long to wait for the HTTP MCP server to respond.
        ///
        /// 60 seconds is chosen to accommodate slow LLM responses or
        /// complex tool executions that might be forwarded by the remote MCP.
        /// Setting this lower might abort valid requests prematurely.
        const HTTP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

        let http_client = reqwest::Client::builder()
            .timeout(HTTP_TIMEOUT)
            .build()
            .unwrap_or_default();
        Self {
            state: HttpShimState {
                server_name,
                remote_url,
                tool_filter,
                telemetry,
                http_client,
            },
        }
    }

    #[allow(dead_code)]
    pub fn is_tool_allowed(&self, tool_name: &str) -> bool {
        if self
            .state
            .tool_filter
            .deny
            .iter()
            .any(|d| d == tool_name || d == "*")
        {
            return false;
        }
        if !self.state.tool_filter.allow.is_empty() {
            return self
                .state
                .tool_filter
                .allow
                .iter()
                .any(|a| a == tool_name || a == "*");
        }
        true
    }

    pub async fn run(&self, port: u16) -> Result<(), ShimError> {
        let state = self.state.clone();
        let app = Router::new()
            .fallback(any(handle_http_proxy))
            .with_state(state.clone());

        let addr = SocketAddr::from(([0, 0, 0, 0], port));
        let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| {
            ShimError::Process(format!(
                "Failed to bind HTTP shim listener to {}: {}",
                addr, e
            ))
        })?;

        state.telemetry.emit(TelemetryMessage::Status {
            server: state.server_name.to_string(),
            status: format!("http_reverse_proxy_listening:{}", port),
            pid: None,
            timestamp: Utc::now().to_rfc3339(),
        });

        axum::serve(listener, app)
            .await
            .map_err(|e| ShimError::Process(format!("HTTP shim server error: {}", e)))?;

        Ok(())
    }
}

async fn handle_http_proxy(State(state): State<HttpShimState>, req: Request) -> Response {
    let method = req.method().clone();
    let uri = req.uri().clone();
    let headers = req.headers().clone();

    let bytes = match axum::body::to_bytes(req.into_body(), 10 * 1024 * 1024).await {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                format!("Failed to read request body: {}", e),
            )
                .into_response();
        }
    };

    // Inspect JSON-RPC
    let mut req_id_opt: Option<Value> = None;
    if let Ok(val) = serde_json::from_slice::<Value>(&bytes) {
        let id = val.get("id").cloned();
        let rpc_method = val
            .get("method")
            .and_then(|m| m.as_str())
            .unwrap_or_default()
            .to_string();
        let params = val.get("params").cloned();
        req_id_opt = id.clone();

        if let Some(ref req_id) = id {
            state.telemetry.emit(TelemetryMessage::Request {
                server: state.server_name.to_string(),
                id: req_id.clone(),
                method: rpc_method.clone(),
                params: params.clone(),
                timestamp: Utc::now().to_rfc3339(),
            });
        }

        // Security check for tool calling
        if rpc_method == "tools/call" {
            let tool_name = params
                .as_ref()
                .and_then(|p| p.get("name"))
                .and_then(|n| n.as_str())
                .unwrap_or_default();

            let is_allowed = {
                if state
                    .tool_filter
                    .deny
                    .iter()
                    .any(|d| d == tool_name || d == "*")
                {
                    false
                } else if !state.tool_filter.allow.is_empty() {
                    state
                        .tool_filter
                        .allow
                        .iter()
                        .any(|a| a == tool_name || a == "*")
                } else {
                    true
                }
            };

            if !is_allowed {
                state.telemetry.emit(TelemetryMessage::Blocked {
                    server: state.server_name.to_string(),
                    tool: tool_name.to_string(),
                    reason: "Blocked by Tailery tool filter policy".to_string(),
                    timestamp: Utc::now().to_rfc3339(),
                });

                let error_response = json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {
                        "code": -32000,
                        "message": format!("Tool '{}' execution blocked by Tailery security policy", tool_name)
                    }
                });
                return (
                    StatusCode::OK,
                    [("content-type", "application/json")],
                    serde_json::to_string(&error_response).unwrap_or_default(),
                )
                    .into_response();
            }
        }
    }

    // Forward to target remote_url
    let path_and_query = uri.path_and_query().map(|pq| pq.as_str()).unwrap_or("");
    let target_url = if state.remote_url.ends_with('/') && path_and_query.starts_with('/') {
        format!(
            "{}{}",
            state.remote_url.trim_end_matches('/'),
            path_and_query
        )
    } else if !path_and_query.is_empty() && path_and_query != "/" {
        format!("{}{}", state.remote_url, path_and_query)
    } else {
        state.remote_url.clone()
    };

    let start = Instant::now();
    let mut forward_req = state.http_client.request(
        reqwest::Method::from_bytes(method.as_str().as_bytes()).unwrap_or(reqwest::Method::POST),
        &target_url,
    );

    for (k, v) in &headers {
        if k != "host"
            && k != "content-length"
            && let Ok(hv) = reqwest::header::HeaderValue::from_bytes(v.as_bytes())
        {
            forward_req = forward_req.header(k.as_str(), hv);
        }
    }

    if !bytes.is_empty() {
        forward_req = forward_req.body(bytes.to_vec());
    }

    let upstream_res = match forward_req.send().await {
        Ok(res) => res,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                format!(
                    "Failed to connect to upstream MCP endpoint ({}): {}",
                    target_url, e
                ),
            )
                .into_response();
        }
    };

    let duration_ms = start.elapsed().as_millis() as u64;
    let status = StatusCode::from_u16(upstream_res.status().as_u16()).unwrap_or(StatusCode::OK);
    let mut resp_headers = HeaderMap::new();
    for (k, v) in upstream_res.headers() {
        if let Ok(name) = axum::http::HeaderName::from_bytes(k.as_str().as_bytes())
            && let Ok(val) = HeaderValue::from_bytes(v.as_bytes())
        {
            resp_headers.insert(name, val);
        }
    }

    let resp_bytes = upstream_res.bytes().await.unwrap_or_default();

    if let Ok(val) = serde_json::from_slice::<Value>(&resp_bytes) {
        let res_id = val.get("id").cloned().or(req_id_opt);
        let result = val.get("result").cloned();
        let error = val.get("error").cloned();

        if let Some(id) = res_id {
            state.telemetry.emit(TelemetryMessage::Response {
                server: state.server_name.to_string(),
                id,
                result,
                error,
                timestamp: Utc::now().to_rfc3339(),
                duration_ms: Some(duration_ms),
            });
        }
    }

    (status, resp_headers, resp_bytes).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_filter_policy() {
        let filter = ToolFilter {
            allow: vec!["read_file".to_string(), "list_dir".to_string()],
            deny: vec!["delete_file".to_string()],
            auto_approve: vec![],
        };

        let shim = ShimInterceptor::new(
            "test".to_string(),
            "echo".to_string(),
            vec![],
            HashMap::new(),
            filter,
            None,
        );

        assert!(shim.is_tool_allowed("read_file"));
        assert!(shim.is_tool_allowed("list_dir"));
        assert!(!shim.is_tool_allowed("delete_file"));
        assert!(!shim.is_tool_allowed("execute_command")); // Not in allow list
    }
}
