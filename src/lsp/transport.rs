use colored::*;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command as TokioCommand};
use tokio::sync::{Mutex, oneshot};

type PendingRequests = Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>;

pub struct LspTransport {
    stdin: ChildStdin,
    process: Child,
    pending_requests: PendingRequests,
}

impl LspTransport {
    pub async fn start(server_name: &str) -> Result<Self, std::io::Error> {
        let mut child = TokioCommand::new(server_name)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null()) // On rend le serveur silencieux
            .spawn()?;

        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let pending_requests = Arc::new(Mutex::new(HashMap::new()));

        // Lancement de la boucle de lecture
        Self::start_read_loop(stdout, pending_requests.clone());

        Ok(Self {
            stdin,
            process: child,
            pending_requests,
        })
    }

    pub async fn send_request(
        &mut self,
        id: u64,
        method: &str,
        params: Value,
    ) -> Result<Value, std::io::Error> {
        let (tx, rx) = oneshot::channel();
        self.pending_requests.lock().await.insert(id, tx);

        let payload = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });

        self.send_raw(payload).await?;
        rx.await
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::Other, "LSP channel dropped"))
    }

    pub async fn send_notification(
        &mut self,
        method: &str,
        params: Value,
    ) -> Result<(), std::io::Error> {
        let payload = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        });
        self.send_raw(payload).await
    }

    async fn send_raw(&mut self, payload: Value) -> Result<(), std::io::Error> {
        let msg = payload.to_string();
        let frame = format!("Content-Length: {}\r\n\r\n{}", msg.len(), msg);
        self.stdin.write_all(frame.as_bytes()).await?;
        self.stdin.flush().await?;
        Ok(())
    }

    fn start_read_loop(stdout: tokio::process::ChildStdout, pending: PendingRequests) {
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();

            loop {
                line.clear();
                if reader.read_line(&mut line).await.is_err() || line.is_empty() {
                    break;
                }

                if line.starts_with("Content-Length:") {
                    let len = line["Content-Length:".len()..]
                        .trim()
                        .parse::<usize>()
                        .unwrap_or(0);
                    let _ = reader.read_line(&mut String::new()).await; // skip \r\n
                    let mut body = vec![0u8; len];

                    if reader.read_exact(&mut body).await.is_ok() {
                        let json: Value = serde_json::from_slice(&body).unwrap_or_default();
                        if let Some(id) = json.get("id").and_then(|i| i.as_u64()) {
                            let mut p = pending.lock().await;
                            if let Some(tx) = p.remove(&id) {
                                let _ = tx.send(json);
                            }
                        } else {
                            Self::handle_notification(json);
                        }
                    }
                }
            }
        });
    }

    fn handle_notification(json: Value) {
        let method = json["method"].as_str().unwrap_or("");
        if method == "window/showMessage" {
            print!("\r\x1b[2K");
            println!(
                "{} {}",
                "[LSP Alert]".yellow().bold(),
                json["params"]["message"]
            );
            print!("{}", "neurogit> ".green().bold());
            let _ = std::io::Write::flush(&mut std::io::stdout());
        }
    }

    pub async fn shutdown(mut self) -> Result<(), std::io::Error> {
        self.process.kill().await?;
        Ok(())
    }
}
