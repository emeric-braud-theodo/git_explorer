use lsp_types::{DocumentSymbolResponse, Location};
use serde::{Deserialize, Serialize};

/// Structure de base d'un message JSON-RPC (Réponse)
#[derive(Debug, Deserialize)]
pub struct LspResponse<T> {
    pub jsonrpc: String,
    pub id: Option<u64>,
    pub result: Option<T>,
    pub error: Option<LspError>,
}

/// Structure pour les erreurs retournées par le serveur
#[derive(Debug, Deserialize)]
pub struct LspError {
    pub code: i64,
    pub message: String,
}

/// On peut définir des alias pour rendre le code du Client plus lisible
pub type SymbolResponse = LspResponse<DocumentSymbolResponse>;
pub type ReferencesResponse = LspResponse<Vec<Location>>;

/// Si tu veux simplifier l'affichage des symboles, tu peux créer ta propre struct
/// "aplatie" pour éviter de naviguer dans l'objet DocumentSymbol complexe.
#[derive(Debug, Serialize, Deserialize)]
pub struct SimpleSymbol {
    pub name: String,
    pub kind: String,
    pub line: u32,
    pub col: u32,
}
