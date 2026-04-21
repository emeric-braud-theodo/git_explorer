pub struct LspClient {
    transport: LspTransport,
    capabilities: ServerCapabilities,
}

impl LspClient {
    pub async fn get_symbols(&mut self, path: &str) -> Result<Vec<Symbol>> {
        self.transport.send_notification("textDocument/didOpen", ...).await?;
        
        // La méthode request retourne maintenant un objet typé, pas du JSON brut
        let response: SymbolResponse = self.transport.request("textDocument/documentSymbol", ...).await?;
        
        Ok(response.result)
    }
}