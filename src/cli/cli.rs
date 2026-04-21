use crate::git_reader::git_reader::GitReader;
use crate::lsp::client::LspClient;
use colored::*;
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;
use serde_json::Value;

/// Représente les intentions de l'utilisateur
enum Command {
    Exit,
    GitHead,
    GitCommits,
    GitDiff,
    LspStart,
    LspStop,
    LspSymbols(String),
    LspReferences(String, u32, u32),
    LspCalls(String, u32, u32),
    Unknown(String),
}

impl Command {
    /// Parse les arguments de la ligne de commande en une variante d'Enum
    fn parse(args: Vec<&str>) -> Self {
        match args.as_slice() {
            ["exit"] | ["quit"] => Self::Exit,
            ["head"] => Self::GitHead,
            ["commits"] => Self::GitCommits,
            ["diff"] => Self::GitDiff,
            ["lsp", "start"] => Self::LspStart,
            ["lsp", "stop"] => Self::LspStop,

            // Gestion des références (Conversion humaine 1-indexed vers LSP 0-indexed)
            ["lsp", "references", path, line, col] | ["references", path, line, col] => {
                let l = line.parse::<u32>().unwrap_or(1).saturating_sub(1);
                let c = col.parse::<u32>().unwrap_or(1).saturating_sub(1);
                Self::LspReferences(path.to_string(), l, c)
            }
            ["lsp", "calls", path, line, col] | ["calls", path, line, col] => {
                let l = line.parse::<u32>().unwrap_or(1).saturating_sub(1);
                let c = col.parse::<u32>().unwrap_or(1).saturating_sub(1);
                Self::LspCalls(path.to_string(), l, c)
            }

            ["lsp", "symbols", path] | ["symbols", path] => Self::LspSymbols(path.to_string()),
            _ => Self::Unknown(args.join(" ")),
        }
    }
}

pub struct CLI {
    git_reader: GitReader,
    lsp_client: Option<LspClient>,
    editor: DefaultEditor,
}

impl CLI {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        // 1. Définir le chemin du dossier
        let config_dir = ".neurogit";
        let history_path = format!("{}/history.txt", config_dir);

        // 2. Créer le répertoire (ne fait rien s'il existe déjà)
        std::fs::create_dir_all(config_dir)?;

        let mut editor = DefaultEditor::new()?;

        // 3. Charger l'historique
        let _ = editor.load_history(&history_path);

        Ok(Self {
            git_reader: GitReader::new()?,
            lsp_client: None,
            editor,
        })
    }

    /// Boucle principale de lecture
    pub async fn listen(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        loop {
            // Remplacement du prompt manuel par rustyline
            let readline = self
                .editor
                .readline(&"neurogit> ".green().bold().to_string());

            match readline {
                Ok(line) => {
                    let input = line.trim();
                    if input.is_empty() {
                        continue;
                    }

                    // Ajoute la commande à l'historique (pour les flèches haut/bas)
                    let _ = self.editor.add_history_entry(input);

                    let args: Vec<&str> = input.split_whitespace().collect();
                    let cmd = Command::parse(args);

                    if let Command::Exit = cmd {
                        if let Some(client) = self.lsp_client.take() {
                            let _ = client.stop().await;
                        }
                        // Utilise le même chemin relatif
                        let _ = self.editor.save_history(".neurogit/history.txt");
                        println!("{}", "Goodbye!".yellow());
                        break;
                    }

                    self.dispatch(cmd).await;
                }
                Err(ReadlineError::Interrupted) => {
                    // Gère le Ctrl-C
                    println!("CTRL-C");
                    break;
                }
                Err(ReadlineError::Eof) => {
                    // Gère le Ctrl-D
                    println!("CTRL-D");
                    break;
                }
                Err(err) => {
                    println!("Error: {:?}", err);
                    break;
                }
            }
        }
        Ok(())
    }

    /// Dispatcher : distribue l'exécution selon la commande
    async fn dispatch(&mut self, cmd: Command) {
        match cmd {
            Command::GitHead => self.exec_git_head(),
            Command::GitCommits => self.exec_git_commits(),
            Command::GitDiff => self.exec_git_diff(),

            Command::LspStart => self.exec_lsp_start().await,

            Command::LspStop => {
                if let Some(client) = self.lsp_client.take() {
                    if let Err(e) = client.stop().await {
                        self.print_error("LSP Stop", e);
                    } else {
                        println!("{}", "LSP server stopped.".yellow());
                    }
                } else {
                    println!("{}", "LSP server is not running.".red());
                }
            }

            Command::LspSymbols(path) => self.handle_lsp_symbols(&path).await,

            Command::LspReferences(path, line, col) => {
                self.handle_lsp_references(&path, line, col).await
            }
            Command::LspCalls(path, line, col) => {
                self.handle_lsp_calls(&path, line, col).await;
            }

            Command::Unknown(s) => println!("{} {}", "Unknown command:".red(), s),
            Command::Exit => (),
        }
    }

    // --- EXECUTEURS GIT ---

    fn exec_git_head(&self) {
        match self.git_reader.get_head() {
            Ok(h) => println!("HEAD is at: {}", h.cyan()),
            Err(e) => self.print_error("Git", e),
        }
    }

    fn exec_git_commits(&self) {
        match self.git_reader.list_commits() {
            Ok(list) => {
                println!("{}", "--- Recent Commits ---".yellow().bold());
                for c in list.iter().take(10) {
                    println!(" {} {}", "•".blue(), c);
                }
            }
            Err(e) => self.print_error("Git", e),
        }
    }

    fn exec_git_diff(&self) {
        let repo = self.git_reader.get_repo();
        match repo.head().and_then(|h| h.peel_to_commit()) {
            Ok(commit) => match self.git_reader.get_commit_diff(&commit) {
                Ok(d) => {
                    if d.is_empty() {
                        println!("No changes in this commit.");
                    } else {
                        println!("{}", d);
                    }
                }
                Err(e) => self.print_error("Diff", e),
            },
            Err(_) => println!("{}", "No commits found to diff.".red()),
        }
    }

    // --- EXECUTEURS LSP ---

    async fn exec_lsp_start(&mut self) {
        if self.lsp_client.is_some() {
            println!("{}", "LSP server is already running.".yellow());
            return;
        }

        println!("{}", "Starting rust-analyzer...".dimmed());
        // LspClient::new() lance automatiquement le processus et l'initialisation
        match LspClient::new("rust-analyzer", ".").await {
            Ok(client) => {
                self.lsp_client = Some(client);
                println!("{}", "LSP server ready.".green());
            }
            Err(e) => self.print_error("LSP Start", e),
        }
    }

    async fn handle_lsp_symbols(&mut self, path: &str) {
        let Some(client) = &mut self.lsp_client else {
            println!("{}", "Error: LSP not started. Type 'lsp start'.".red());
            return;
        };

        println!("{} {}", "Analyzing".bright_black(), path.blue().bold());

        match client.get_symbols(path).await {
            Ok(response) => {
                if let Some(symbols) = response.get("result").and_then(|r| r.as_array()) {
                    if symbols.is_empty() {
                        println!("No symbols found.");
                        return;
                    }

                    println!("\n{:<25} {:<10}", "SYMBOL".bold(), "KIND".bold());
                    println!("{}", "------------------------------------------".dimmed());

                    for s in symbols {
                        let name = s["name"].as_str().unwrap_or("?");
                        let kind = match s["kind"].as_u64() {
                            Some(1) => "File".white(),
                            Some(2) => "Module".blue(),
                            Some(5) => "Class".magenta(),
                            Some(6) => "Method".green(),
                            Some(11) => "Interface".cyan(),
                            Some(12) => "Function".green(),
                            Some(13) => "Variable".yellow(),
                            Some(23) => "Struct".magenta(),
                            _ => "Other".normal(),
                        };
                        println!("{:<25} {:<10}", name, kind);
                    }
                    println!();
                } else if let Some(error) = response.get("error") {
                    eprintln!("{} {}", "LSP Error:".red(), error["message"]);
                }
            }
            Err(e) => self.print_error("LSP Request", e),
        }
    }

    async fn handle_lsp_references(&mut self, path: &str, line: u32, col: u32) {
        let Some(client) = &mut self.lsp_client else {
            println!("{}", "Error: LSP not started. Type 'lsp start'.".red());
            return;
        };

        println!(
            "{} {} at {}:{}",
            "Finding references for".bright_black(),
            path.blue(),
            line + 1, // Affichage humain pour confirmer
            col + 1
        );

        match client.get_references(path, line, col).await {
            Ok(response) => {
                match response.get("result") {
                    Some(Value::Array(locations)) => {
                        if locations.is_empty() {
                            println!("{}", "No references found.".yellow());
                            return;
                        }

                        println!(
                            "\n{} references found:",
                            locations.len().to_string().green()
                        );
                        for loc in locations {
                            let uri = loc["uri"].as_str().unwrap_or("?");

                            if uri.contains(".rustup") || uri.contains(".cargo") {
                                continue;
                            }

                            let range = &loc["range"];
                            let start_line = range["start"]["line"].as_u64().unwrap_or(0);
                            let display_path = uri.trim_start_matches("file://");

                            println!(
                                " {} {}:{}",
                                "→".cyan(),
                                display_path.underline(),
                                start_line + 1
                            );
                        }
                        println!();
                    }
                    Some(Value::Null) | None => {
                        println!("{}", "No references found at this position.".yellow());
                    }
                    _ => {
                        println!("{}", "Unexpected LSP response format.".red());
                    }
                }

                if let Some(error) = response.get("error") {
                    eprintln!("{} {}", "LSP Error:".red(), error["message"]);
                }
            }
            Err(e) => self.print_error("LSP References", e),
        }
    }

    async fn handle_lsp_calls(&mut self, path: &str, line: u32, col: u32) {
        let Some(client) = &mut self.lsp_client else {
            return;
        };

        match client.get_outgoing_calls(path, line, col).await {
            Ok(response) => {
                if let Some(calls) = response.get("result").and_then(|r| r.as_array()) {
                    println!(
                        "\n{} calls found inside this symbol:",
                        calls.len().to_string().green()
                    );
                    for call in calls {
                        // Dans outgoingCalls, la cible est dans "to"
                        let to = &call["to"];
                        let name = to["name"].as_str().unwrap_or("?");
                        let uri = to["uri"].as_str().unwrap_or("");
                        let line = to["range"]["start"]["line"].as_u64().unwrap_or(0);

                        if !uri.contains(".rustup") {
                            println!(
                                " {} {} ({}:{})",
                                "→".cyan(),
                                name.bold(),
                                uri.split('/').last().unwrap_or(""),
                                line + 1
                            );
                        }
                    }
                }
            }
            Err(e) => self.print_error("LSP Calls", e),
        }
    }

    // --- HELPERS ---

    fn print_error<E: std::fmt::Display>(&self, context: &str, err: E) {
        eprintln!("{} {}: {}", "Error".red().bold(), context, err);
    }
}
