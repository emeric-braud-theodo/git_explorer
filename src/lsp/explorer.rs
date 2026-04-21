use super::client::LspClient;
use super::protocol::{Node, ProjectGraph};
use std::fs;
use walkdir::WalkDir;

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

        // 1. Utiliser des chemins ABSOLUS pour matcher le LSP
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

        // 2. Récupérer les Nodes
        for file_path in &files {
            let file_str = file_path.to_string_lossy().to_string();
            if let Ok(symbols_res) = self.client.get_symbols(&file_str).await {
                if let Some(symbols) = symbols_res.get("result").and_then(|r| r.as_array()) {
                    self.extract_symbols_recursive(&file_str, symbols, &mut graph);
                }
            }
        }

        // 3. Récupérer les Edges (Outgoing + Incoming)
        let node_ids: Vec<String> = graph.nodes.keys().cloned().collect();
        for id in node_ids {
            let node = graph.nodes.get(&id).unwrap().clone();

            // Outgoing
            if let Ok(res) = self
                .client
                .get_outgoing_calls(&node.file, node.line, node.col)
                .await
            {
                if let Some(calls) = res.get("result").and_then(|r| r.as_array()) {
                    for c in calls {
                        let to = &c["to"];
                        let to_uri = to["uri"].as_str().unwrap_or("");
                        if to_uri.contains(&node.file.split('/').nth(2).unwrap_or("src")) {
                            let to_line = to["range"]["start"]["line"].as_u64().unwrap_or(0) as u32;
                            let to_col =
                                to["range"]["start"]["character"].as_u64().unwrap_or(0) as u32;
                            let to_file = to_uri.trim_start_matches("file://").to_string();
                            graph.edges.push((
                                node.id.clone(),
                                format!("{}:{}:{}", to_file, to_line, to_col),
                            ));
                        }
                    }
                }
            }
            // Tu peux ajouter les incoming ici de la même façon
        }

        Ok(graph)
    }

    fn extract_symbols_recursive(
        &self,
        file: &str,
        symbols: &[serde_json::Value],
        graph: &mut ProjectGraph,
    ) {
        for s in symbols {
            // Gestion hybride des formats LSP (selectionRange ou location)
            let range = s
                .get("selectionRange")
                .or_else(|| s.get("location").and_then(|l| l.get("range")))
                .and_then(|r| r.get("start"));

            if let Some(start) = range {
                let line = start["line"].as_u64().unwrap_or(0) as u32;
                let col = start["character"].as_u64().unwrap_or(0) as u32;
                let id = format!("{}:{}:{}", file, line, col);

                graph.nodes.insert(
                    id.clone(),
                    Node {
                        id,
                        name: s["name"].as_str().unwrap_or("?").to_string(),
                        file: file.to_string(),
                        line,
                        col,
                        kind: format!("{:?}", s["kind"]),
                    },
                );
            }

            // Si c'est hiérarchique, on descend dans les enfants (children)
            if let Some(children) = s.get("children").and_then(|c| c.as_array()) {
                self.extract_symbols_recursive(file, children, graph);
            }
        }
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
        dot.push_str("  node [shape=box, style=rounded, fontname=\"Arial\"];\n");

        for node in graph.nodes.values() {
            // Nettoyage du nom pour éviter les problèmes de caractères spéciaux
            let clean_name = node.name.replace('\"', "\\\"");
            dot.push_str(&format!("  \"{}\" [label=\"{}\"];\n", node.id, clean_name));
        }

        for (src, target) in &graph.edges {
            dot.push_str(&format!("  \"{}\" -> \"{}\";\n", src, target));
        }

        dot.push_str("}\n");
        std::fs::write(".neurogit/graph.dot", dot)
    }
}
