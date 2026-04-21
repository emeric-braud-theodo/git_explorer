use super::transport::LspTransport;
use serde_json::{Value, json};
use std::fs; // Utilisation de la crate lsp-types

pub struct LspClient {
    transport: LspTransport,
    request_id: u64,
    root_path: String,
}

impl LspClient {
    pub async fn new(server_name: &str, root_path: &str) -> Result<Self, std::io::Error> {
        let transport = LspTransport::start(server_name).await?;
        let mut client = Self {
            transport,
            request_id: 1,
            root_path: root_path.to_string(),
        };

        client.initialize().await?;
        Ok(client)
    }

    async fn initialize(&mut self) -> Result<(), std::io::Error> {
        let abs_path = std::fs::canonicalize(&self.root_path)?;
        let uri = format!("file://{}", abs_path.display());

        let params = json!({
            "processId": std::process::id(),
            "rootUri": uri,
            "capabilities": {
                "textDocument": {
                    "documentSymbol": { "hierarchicalDocumentSymbolSupport": true },
                    "references": { "dynamicRegistration": true },
                    "callHierarchy": { "dynamicRegistration": true }
                },
                // Optionnel mais recommandé pour éviter des timeouts serveurs
                "workspace": { "configuration": true }
            }
        });

        self.request("initialize", params).await?;
        self.transport
            .send_notification("initialized", json!({}))
            .await
    }

    pub async fn get_symbols(&mut self, file_path: &str) -> Result<Value, std::io::Error> {
        let uri = self.path_to_uri(file_path)?;
        let content = fs::read_to_string(file_path)?;

        // Notification d'ouverture
        self.transport
            .send_notification(
                "textDocument/didOpen",
                json!({
                    "textDocument": {
                        "uri": uri,
                        "languageId": "rust",
                        "version": 1,
                        "text": content
                    }
                }),
            )
            .await?;

        self.request(
            "textDocument/documentSymbol",
            json!({
                "textDocument": { "uri": uri }
            }),
        )
        .await
    }

    pub async fn get_references(
        &mut self,
        file_path: &str,
        line: u32,
        col: u32,
    ) -> Result<Value, std::io::Error> {
        let uri = self.path_to_uri(file_path)?;
        let content = fs::read_to_string(file_path)?; // Ajouté

        // 1. Signaler au serveur que le fichier est ouvert/mis à jour
        self.transport
            .send_notification(
                "textDocument/didOpen",
                json!({
                    "textDocument": {
                        "uri": uri,
                        "languageId": "rust",
                        "version": 1,
                        "text": content
                    }
                }),
            )
            .await?;

        // 2. Maintenant la requête a une chance de réussir
        self.request(
            "textDocument/references",
            json!({
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": col },
                "context": { "includeDeclaration": true }
            }),
        )
        .await
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value, std::io::Error> {
        let id = self.request_id;
        self.request_id += 1;
        self.transport.send_request(id, method, params).await
    }

    fn path_to_uri(&self, path: &str) -> Result<String, std::io::Error> {
        let abs = fs::canonicalize(path)?;
        Ok(format!("file://{}", abs.display()))
    }

    pub async fn stop(self) -> Result<(), std::io::Error> {
        self.transport.shutdown().await
    }

    pub async fn get_outgoing_calls(
        &mut self,
        file_path: &str,
        line: u32,
        col: u32,
    ) -> Result<Value, std::io::Error> {
        let uri = self.path_to_uri(file_path)?;

        // ÉTAPE 1 : Préparer la hiérarchie pour obtenir l'objet "item"
        let prepare_res = self
            .request(
                "textDocument/prepareCallHierarchy",
                json!({
                    "textDocument": { "uri": uri },
                    "position": { "line": line, "character": col }
                }),
            )
            .await?;

        // On extrait le premier item (souvent un seul)
        let Some(items) = prepare_res.get("result").and_then(|r| r.as_array()) else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "No hierarchy item found",
            ));
        };

        if items.is_empty() {
            return Ok(json!([]));
        }

        // ÉTAPE 2 : Demander les appels sortants pour cet item
        self.request("callHierarchy/outgoingCalls", json!({ "item": items[0] }))
            .await
    }

    pub async fn get_incoming_calls(
        &mut self,
        file_path: &str,
        line: u32,
        col: u32,
    ) -> Result<Value, std::io::Error> {
        let uri = self.path_to_uri(file_path)?;

        // 1. Prepare
        let prepare_res = self
            .request(
                "textDocument/prepareCallHierarchy",
                json!({
                    "textDocument": { "uri": uri },
                    "position": { "line": line, "character": col }
                }),
            )
            .await?;

        let Some(items) = prepare_res.get("result").and_then(|r| r.as_array()) else {
            return Ok(json!([]));
        };

        if items.is_empty() {
            return Ok(json!([]));
        }

        // 2. Incoming
        self.request("callHierarchy/incomingCalls", json!({ "item": items[0] }))
            .await
    }
}
