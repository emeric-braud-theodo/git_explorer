use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
pub struct Node {
    pub id: String, // format: "file.rs:line:col"
    pub name: String,
    pub file: String,
    pub line: u32,
    pub col: u32,
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct Graph {
    pub nodes: HashMap<String, Node>,
    pub calls: Vec<(String, String)>, // (ID source, ID cible)
}
