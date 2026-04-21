use super::client::LspClient;
use super::protocol::{Node, ProjectGraph};
use futures::Future;
use serde_json::Value;
use std::fs;
use std::pin::Pin;
use walkdir::WalkDir;

// Type alias pour gérer la Future récursive boxée
type AsyncStep<'a> = Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

pub struct LspExplorer<'a> {
    client: &'a mut LspClient,
}

impl<'a> LspExplorer<'a> {
    pub fn new(client: &'a mut LspClient) -> Self {
        Self { client }
    }

    pub async fn build_full_graph(&mut self) -> Result<ProjectGraph, Box<dyn std::error::Error>> {
        let mut graph = ProjectGraph::default();
        let mut files = Vec::new();

        let root_dir = std::env::current_dir()?;
        for entry in WalkDir::new(root_dir.join("src"))
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if entry.path().extension().map_or(false, |ext| ext == "rs") {
                files.push(entry.path().to_path_buf());
            }
        }

        println!("🔍 Scanning {} files...", files.len());

        // 1. Extraction des Nodes (Symboles)
        for file_path in &files {
            let file_str = file_path.to_string_lossy().to_string();
            if let Ok(symbols_res) = self.client.get_symbols(&file_str).await {
                if let Some(symbols) = symbols_res.get("result").and_then(|r| r.as_array()) {
                    self.extract_symbols_recursive(&file_str, symbols, &mut graph, None)
                        .await;
                }
            }
        }

        // 2. Extraction des Edges (Appels logiques)
        // IMPORTANT: On travaille sur une copie des nodes pour éviter les problèmes de borrow checker
        let nodes: Vec<Node> = graph.nodes.values().cloned().collect();

        for node in nodes {
            // On interroge le LSP sur la position exacte du NOM de la fonction
            if let Ok(res) = self
                .client
                .get_outgoing_calls(&node.file, node.line, node.col)
                .await
            {
                if let Some(calls) = res.get("result").and_then(|r| r.as_array()) {
                    for c in calls {
                        let to = &c["to"];
                        let to_uri = to["uri"].as_str().unwrap_or("");

                        if to_uri.contains("/src/") {
                            // On récupère la SelectionRange de la cible si disponible, sinon la Range
                            // Le protocole CallHierarchyOutgoingCall contient un CallHierarchyItem dans "to"
                            // Cet item a une 'selectionRange' qui est l'ID unique de notre cible
                            let target_range = to
                                .get("selectionRange")
                                .or(to.get("range"))
                                .expect("No range found for call target");

                            let to_line =
                                target_range["start"]["line"].as_u64().unwrap_or(0) as u32;
                            let to_col =
                                target_range["start"]["character"].as_u64().unwrap_or(0) as u32;
                            let to_file = to_uri.trim_start_matches("file://").to_string();

                            let target_id = format!("{}:{}:{}", to_file, to_line, to_col);

                            // On ajoute le lien seulement si la cible existe dans nos nodes
                            // (évite de lier vers des fonctions de librairies externes filtrées)
                            graph.edges.push((node.id.clone(), target_id));
                        }
                    }
                }
            }
        }

        Ok(graph)
    }

    fn get_id_from_symbol(&self, file: &str, s: &Value) -> Option<String> {
        let start = s
            .get("selectionRange")
            .and_then(|sr| sr.get("start"))
            .or_else(|| {
                s.get("location")
                    .and_then(|l| l.get("range").and_then(|r| r.get("start")))
            })?;

        let line = start["line"].as_u64().unwrap_or(0);
        let col = start["character"].as_u64().unwrap_or(0);

        Some(format!("{}:{}:{}", file, line, col))
    }

    fn extract_symbols_recursive(
        &'a self,
        file: &'a str,
        symbols: &'a [Value],
        graph: &'a mut ProjectGraph,
        parent_id: Option<String>,
    ) -> AsyncStep<'a> {
        Box::pin(async move {
            for s in symbols {
                if let Some(current_id) = self.get_id_from_symbol(file, s) {
                    let name = s["name"].as_str().unwrap_or("?").to_string();
                    let kind_str = format!("{}", s["kind"].as_u64().unwrap_or(0));

                    let parts: Vec<&str> = current_id.split(':').collect();
                    let line = parts[1].parse().unwrap_or(0);
                    let col = parts[2].parse().unwrap_or(0);

                    graph.nodes.insert(
                        current_id.clone(),
                        Node {
                            id: current_id.clone(),
                            name,
                            file: file.to_string(),
                            line,
                            col,
                            kind: kind_str,
                        },
                    );

                    if let Some(p_id) = parent_id.clone() {
                        graph.edges.push((p_id, current_id.clone()));
                    }

                    if let Some(children) = s.get("children").and_then(|c| c.as_array()) {
                        self.extract_symbols_recursive(
                            file,
                            children,
                            graph,
                            Some(current_id.clone()),
                        )
                        .await;
                    }
                }
            }
        })
    }

    pub fn save_to_disk(&self, graph: &ProjectGraph) -> std::io::Result<()> {
        let path = ".neurogit/graph.json";
        let _ = fs::create_dir_all(".neurogit");
        let data = serde_json::to_string_pretty(graph)?;
        fs::write(path, data)?;
        Ok(())
    }

    pub fn export_to_dot(&self, graph: &ProjectGraph) -> std::io::Result<()> {
        let mut dot = String::from("digraph NeuroGraph {\n");
        dot.push_str("  rankdir=LR;\n");
        dot.push_str("  node [shape=box, style=rounded, fontname=\"Arial\", fontsize=10];\n");
        dot.push_str("  edge [fontsize=8, color=\"#555555\"];\n");

        for node in graph.nodes.values() {
            let clean_name = node.name.replace('\"', "\\\"");

            // Mapping des couleurs par KIND (LSP Standard)
            // 23: Struct, 12: Function, 6: Method, 2: Module, 5: Class
            let color = match node.kind.as_str() {
                "23" | "5" => "lightblue",  // Structs / Classes
                "12" | "6" => "lightgreen", // Functions / Methods
                "2" => "lightgrey",         // Modules
                _ => "white",
            };

            dot.push_str(&format!(
                "  \"{}\" [label=\"{}\\n(kind:{})\", fillcolor=\"{}\", style=\"filled,rounded\"];\n",
                node.id, clean_name, node.kind, color
            ));
        }

        let mut seen_edges = std::collections::HashSet::new();
        for (src, target) in &graph.edges {
            if seen_edges.insert((src, target)) {
                dot.push_str(&format!("  \"{}\" -> \"{}\";\n", src, target));
            }
        }

        dot.push_str("}\n");
        std::fs::write(".neurogit/graph.dot", dot)
    }
}
