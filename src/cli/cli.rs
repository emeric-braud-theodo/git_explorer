use crate::git_reader::git_reader::GitReader;
use crate::lsp::server::Server;
use colored::*;
use std::io::{self, Write};

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
            ["lsp", "references", path, line, col] | ["references", path, line, col] => {
                let l = line.parse().unwrap_or(0);
                let c = col.parse().unwrap_or(0);
                Self::LspReferences(path.to_string(), l, c)
            }
            ["lsp", "symbols", path] | ["symbols", path] => Self::LspSymbols(path.to_string()),
            _ => Self::Unknown(args.join(" ")),
        }
    }
}

pub struct CLI {
    git_reader: GitReader,
    lsp_server: Server,
}

impl CLI {
    pub fn new() -> Result<Self, git2::Error> {
        Ok(Self {
            git_reader: GitReader::new()?,
            lsp_server: Server::new(),
        })
    }

    /// Boucle principale de lecture
    pub async fn listen(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        loop {
            let input = self.prompt()?;
            let args: Vec<&str> = input.split_whitespace().collect();

            if args.is_empty() {
                continue;
            }

            let cmd = Command::parse(args);

            // On gère la sortie immédiatement
            if let Command::Exit = cmd {
                self.lsp_server.stop().await?;
                println!("{}", "Goodbye!".yellow());
                break;
            }

            // On délègue le reste au dispatcher
            self.dispatch(cmd).await;
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
                let _ = self.lsp_server.stop().await;
            }
            Command::LspSymbols(path) => self.handle_lsp_symbols(&path).await,
            Command::LspReferences(path, line, col) => {
                self.handle_lsp_references(&path, line, col).await
            }
            Command::Unknown(s) => println!("{} {}", "Unknown command:".red(), s),
            Command::Exit => (), // Déjà géré dans listen
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
        println!("{}", "Starting rust-analyzer...".dimmed());
        match self.lsp_server.start(".", "rust-analyzer").await {
            Ok(_) => println!("{}", "LSP server ready.".green()),
            Err(e) => self.print_error("LSP Start", e),
        }
    }

    async fn handle_lsp_symbols(&mut self, path: &str) {
        println!("{} {}", "Analyzing".bright_black(), path.blue().bold());

        match self.lsp_server.get_symbols(path).await {
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
        println!(
            "{} {} at {}:{}",
            "Finding references for".bright_black(),
            path.blue(),
            line,
            col
        );

        // On suppose que tu as ajouté la méthode get_references dans ton Server.rs
        match self.lsp_server.get_references(path, line, col).await {
            Ok(response) => {
                if let Some(locations) = response.get("result").and_then(|r| r.as_array()) {
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
                        let range = &loc["range"];
                        let start_line = range["start"]["line"].as_u64().unwrap_or(0);

                        // Nettoyage de l'URI pour l'affichage (file:///path -> /path)
                        let display_path = uri.trim_start_matches("file://");
                        println!(
                            " {} {}:{}",
                            "→".cyan(),
                            display_path.underline(),
                            start_line
                        );
                    }
                    println!();
                } else if let Some(error) = response.get("error") {
                    eprintln!("{} {}", "LSP Error:".red(), error["message"]);
                }
            }
            Err(e) => self.print_error("LSP References", e),
        }
    }

    // --- HELPERS ---

    fn prompt(&self) -> io::Result<String> {
        print!("{}", "neurogit> ".green().bold());
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        Ok(input.trim().to_string())
    }

    fn print_error<E: std::fmt::Display>(&self, context: &str, err: E) {
        eprintln!("{} {}: {}", "Error".red().bold(), context, err);
    }
}
