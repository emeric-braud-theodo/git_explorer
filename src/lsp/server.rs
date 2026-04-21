use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command as TokioCommand};
use tokio::sync::{Mutex, oneshot};

// Type alias pour clarifier la Map de promesses
type PendingRequests = Arc<Mutex<HashMap<u64, oneshot::Sender<serde_json::Value>>>>;

pub struct Server {
    stdin: Option<ChildStdin>,
    stdout: Option<ChildStdout>,
    process: Option<Child>,
    folder_path: String,
    // La Map partagée entre le Server et la boucle de lecture
    pending_requests: PendingRequests,
    request_counter: u64,
}

impl Server {
    pub fn new() -> Self {
        Self {
            stdin: None,
            stdout: None,
            process: None,
            folder_path: String::new(),
            pending_requests: Arc::new(Mutex::new(HashMap::new())),
            request_counter: 1, // On commence à 1 pour éviter les conflits avec init (id: 0)
        }
    }

    pub async fn start(
        &mut self,
        folder_path: &str,
        server_name: &str,
    ) -> Result<(), std::io::Error> {
        self.folder_path = folder_path.to_string();
        let mut child = TokioCommand::new(server_name)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;

        self.stdin = child.stdin.take();
        self.stdout = child.stdout.take();
        self.process = Some(child);

        if self.stdout.is_some() {
            // On passe la Map à la boucle
            self.start_loop(self.pending_requests.clone());
        }

        self.initialize().await?;
        Ok(())
    }

    /// MÉTHODE CLÉ : Envoie et ATTEND la réponse
    async fn request(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, std::io::Error> {
        let id = self.request_counter;
        self.request_counter += 1;

        let (tx, rx) = oneshot::channel();

        // Enregistrer l'attente
        {
            let mut pending = self.pending_requests.lock().await;
            pending.insert(id, tx);
        }

        let payload = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });

        self.send(payload).await?;

        // Attendre la réponse du canal (rempli par la boucle de lecture)
        rx.await.map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::Other, "LSP response channel dropped")
        })
    }

    // --- MÉTHODES PUBLIQUES ---

    pub async fn get_symbols(
        &mut self,
        file_path: &str,
    ) -> Result<serde_json::Value, std::io::Error> {
        let content = std::fs::read_to_string(file_path)?;
        let abs_path = std::fs::canonicalize(file_path)?;
        let uri = format!("file://{}", abs_path.display());

        // Notification (pas de réponse attendue)
        let did_open = serde_json::json!({
            "textDocument": {
                "uri": uri,
                "languageId": "rust",
                "version": 1,
                "text": content
            }
        });
        self.send_notification("textDocument/didOpen", did_open)
            .await?;

        // Requête (on attend la réponse !)
        let params = serde_json::json!({ "textDocument": { "uri": uri } });
        self.request("textDocument/documentSymbol", params).await
    }

    /// Arrête proprement le serveur LSP en suivant le protocole shutdown/exit.
    pub async fn stop(&mut self) -> Result<(), std::io::Error> {
        // 1. Envoie la requête shutdown et attend la réponse (le serveur ne doit plus
        //    accepter de nouvelles requêtes après ça, seulement "exit").
        let _ = self.request("shutdown", serde_json::json!(null)).await;

        // 2. Envoie la notification exit (pas de réponse attendue).
        let _ = self
            .send_notification("exit", serde_json::json!(null))
            .await;

        // 3. Ferme stdin pour signaler la fin du flux au processus.
        self.stdin.take();

        // 4. Attend la terminaison effective du processus enfant.
        if let Some(mut child) = self.process.take() {
            child.wait().await?;
        }

        // 5. Annule toutes les requêtes en attente (leurs canaux seront droppés).
        self.pending_requests.lock().await.clear();

        Ok(())
    }

    // --- PLOMBERIE ---

    async fn send_notification(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<(), std::io::Error> {
        let notif = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        });
        self.send(notif).await
    }

    async fn send(&mut self, payload: serde_json::Value) -> Result<(), std::io::Error> {
        let message = payload.to_string();
        if let Some(ref mut stdin) = self.stdin {
            let frame = format!("Content-Length: {}\r\n\r\n{}", message.len(), message);
            stdin.write_all(frame.as_bytes()).await?;
            stdin.flush().await?;
            Ok(())
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "Stdin dead",
            ))
        }
    }

    fn start_loop(&mut self, pending_requests: PendingRequests) {
        let Some(stdout) = self.stdout.take() else {
            return;
        };

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
                    let _ = reader.read_line(&mut String::new()).await;
                    let mut body = vec![0u8; len];
                    if reader.read_exact(&mut body).await.is_ok() {
                        let json: serde_json::Value =
                            serde_json::from_slice(&body).unwrap_or_default();

                        // ROUTAGE : Est-ce une réponse à une requête ?
                        if let Some(id) = json.get("id").and_then(|i| i.as_u64()) {
                            let mut pending = pending_requests.lock().await;
                            if let Some(tx) = pending.remove(&id) {
                                let _ = tx.send(json); // Réveille la fonction en attente
                            }
                        } else {
                            // C'est une notification (logs, diagnostics...)
                            // println!("[LSP Notification] {}", json["method"]);
                        }
                    }
                }
            }
        });
    }

    pub async fn initialize(&mut self) -> Result<(), std::io::Error> {
        let abs_path = std::fs::canonicalize(&self.folder_path)?;
        let uri = format!("file://{}", abs_path.display());

        let params = serde_json::json!({
            "process_id": std::process::id(),
            "rootUri": uri,
            "capabilities": { "textDocument": { "documentSymbol": { "hierarchicalDocumentSymbolSupport": true } } }
        });

        // On utilise request pour être sûr que l'init est fini avant de continuer
        let _ = self.request("initialize", params).await?;
        self.send_notification("initialized", serde_json::json!({}))
            .await
    }
}
