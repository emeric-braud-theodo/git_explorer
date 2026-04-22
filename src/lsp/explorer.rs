use super::client::LspClient;
use super::protocol::{Node, ProjectGraph};
use serde_json::Value;
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

        // 1. Extraction des Nodes
        for file_path in &files {
            let file_str = file_path.to_string_lossy().to_string();
            if let Ok(symbols_res) = self.client.get_symbols(&file_str).await {
                if let Some(symbols) = symbols_res.get("result").and_then(|r| r.as_array()) {
                    // On utilise une boucle plate ou une récursion qui ne bloque pas les lifetimes
                    self.extract_recursive_internal(&file_str, symbols, &mut graph, None)
                        .await;
                }
            }
        }

        // 2. Extraction des Edges (Appels)
        // On clone les IDs pour pouvoir itérer sans bloquer graph.nodes
        let nodes: Vec<Node> = graph.nodes.values().cloned().collect();
        for node in nodes {
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
                            let target_range = to
                                .get("selectionRange")
                                .or(to.get("range"))
                                .expect("No range found");

                            let to_line =
                                target_range["start"]["line"].as_u64().unwrap_or(0) as u32;
                            let to_col =
                                target_range["start"]["character"].as_u64().unwrap_or(0) as u32;
                            let to_file = to_uri.trim_start_matches("file://").to_string();

                            graph.edges.push((
                                node.id.clone(),
                                format!("{}:{}:{}", to_file, to_line, to_col),
                            ));
                        }
                    }
                }
            }
        }

        Ok(graph)
    }

    // Utilisation d'une fonction interne pour éviter le conflit de lifetime 'a
    // On utilise BoxFuture manuellement pour la récursion
    async fn extract_recursive_internal(
        &mut self,
        file: &str,
        symbols: &[Value],
        graph: &mut ProjectGraph,
        parent_id: Option<String>,
    ) {
        for s in symbols {
            if let Some(current_id) = self.get_id_from_symbol(file, s) {
                let name = s["name"].as_str().unwrap_or("?").to_string();
                let kind_u64 = s["kind"].as_u64().unwrap_or(0);

                let parts: Vec<&str> = current_id.split(':').collect();
                let line = parts[1].parse().unwrap_or(0);
                let col = parts[2].parse().unwrap_or(0);

                graph.nodes.insert(
                    current_id.clone(),
                    Node {
                        id: current_id.clone(),
                        name: name.clone(),
                        file: file.to_string(),
                        line,
                        col,
                        kind: format!("{}", kind_u64),
                    },
                );

                if let Some(p_id) = parent_id.clone() {
                    graph.edges.push((p_id, current_id.clone()));
                }

                // Lien Impl -> Struct
                if kind_u64 == 26 || name.starts_with("impl") {
                    if let Ok(def_res) = self.client.get_definition(file, line, col + 5).await {
                        if let Some(target_id) = self.parse_definition_to_id(def_res) {
                            graph.edges.push((target_id, current_id.clone()));
                        }
                    }
                }

                if let Some(children) = s.get("children").and_then(|c| c.as_array()) {
                    // Pour éviter E0499, on utilise une récursion bridée ou on déplace le traitement
                    // Ici, on appelle récursivement sur les enfants
                    // Comme c'est asynchrone, on boxe la future pour le compilateur
                    let sub_file = file.to_string();
                    let sub_children = children.to_vec();
                    let sub_parent = Some(current_id.clone());

                    // Appel récursif (on utilise Box::pin pour stabiliser la récursion async)
                    Box::pin(self.extract_recursive_internal(
                        &sub_file,
                        &sub_children,
                        graph,
                        sub_parent,
                    ))
                    .await;
                }
            }
        }
    }

    fn get_id_from_symbol(&self, file: &str, s: &Value) -> Option<String> {
        let start = s
            .get("selectionRange")
            .and_then(|sr| sr.get("start"))
            .or_else(|| {
                s.get("location")
                    .and_then(|l| l.get("range").and_then(|r| r.get("start")))
            })?;
        Some(format!("{}:{}:{}", file, start["line"], start["character"]))
    }

    fn parse_definition_to_id(&self, res: Value) -> Option<String> {
        let loc = if res["result"].is_array() {
            res["result"].get(0)
        } else {
            res.get("result")
        }?;
        let uri = loc["uri"].as_str()?;
        let path = uri.trim_start_matches("file://");
        let start = &loc["range"]["start"];
        Some(format!("{}:{}:{}", path, start["line"], start["character"]))
    }

    // ... export_to_dot et save_to_disk sont identiques ...
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
        dot.push_str("  edge [fontsize=8, color=\"#555555\"];\n\n");

        for node in graph.nodes.values() {
            let clean_name = node.name.replace('\"', "\\\"");
            let color = match node.kind.as_str() {
                "23" | "5" => "lightblue",
                "12" | "6" => "lightgreen",
                "2" => "lightgrey",
                "26" => "lightyellow",
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

    pub fn export_to_cypher(&self, graph: &ProjectGraph) -> String {
        let mut cypher = String::new();

        // 1. On définit une contrainte d'unicité (Optionnel mais recommandé)
        cypher.push_str("CREATE CONSTRAINT IF NOT EXISTS FOR (n:Node) REQUIRE n.id IS UNIQUE;\n");

        // 2. Création des Nœuds
        for node in graph.nodes.values() {
            // Nettoyage systématique
            let clean_id = node.id.replace('\\', "/").replace('\'', "\\'");
            let clean_name = node.name.replace('\"', "\\\"").replace('\'', "\\'");
            let clean_file = node.file.replace('\\', "/").replace('\'', "\\'");

            cypher.push_str(&format!(
                "MERGE (n:Node {{id: '{}'}}) SET n.name = '{}', n.kind = '{}', n.file = '{}';\n",
                clean_id, clean_name, node.kind, clean_file
            ));
        }

        // 3. Création des Relations
        for (src, target) in &graph.edges {
            // On applique EXACTEMENT le même traitement aux IDs ici
            let clean_src = src.replace('\\', "/").replace('\'', "\\'");
            let clean_target = target.replace('\\', "/").replace('\'', "\\'");

            // On utilise MERGE pour les nœuds ET la relation
            // Si l'ID match parfaitement avec un nœud créé au dessus, il l'utilisera.
            // Sinon, il créera un nouveau nœud (ce qui permet de voir les appels vers 'std')
            cypher.push_str(&format!(
                "MERGE (a:Node {{id: '{}'}}) \
             MERGE (b:Node {{id: '{}'}}) \
             MERGE (a)-[:CALLS]->(b);\n",
                clean_src, clean_target
            ));
        }
        cypher
    }
}
