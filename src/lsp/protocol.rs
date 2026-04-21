use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct Node {
    pub id: String, // "file:line:col"
    pub name: String,
    pub file: String,
    pub line: u32,
    pub col: u32,
    pub kind: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ProjectGraph {
    pub nodes: HashMap<String, Node>,
    pub edges: Vec<(String, String)>, // (SourceID, TargetID)
}
