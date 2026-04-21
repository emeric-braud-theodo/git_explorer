use crate::git_reader::git_reader::GitReader;
use crate::lsp::server::Server; // Assure-toi que le chemin correspond à ton organisation
use colored::*;
use std::io::{self, Write}; // Optionnel : cargo add colored pour une CLI plus lisible

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

    pub async fn listen(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        loop {
            print!("{}", "neurogit> ".green().bold());
            io::stdout().flush()?;

            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            let command = input.trim();

            let args: Vec<&str> = command.split_whitespace().collect();
            if args.is_empty() {
                continue;
            }

            match args[0] {
                "exit" | "quit" => {
                    self.lsp_server.stop().await?;
                    break;
                }

                // --- COMMANDES GIT ---
                "head" => match self.git_reader.get_head() {
                    Ok(h) => println!("HEAD is at: {}", h.cyan()),
                    Err(e) => eprintln!("{} {}", "Git Error:".red(), e),
                },

                "commits" => match self.git_reader.list_commits() {
                    Ok(list) => {
                        println!("{}", "--- Recent Commits ---".yellow());
                        for c in list.iter().take(10) {
                            println!("• {}", c);
                        }
                    }
                    Err(e) => eprintln!("{} {}", "Git Error:".red(), e),
                },

                "diff" => {
                    // Diff du dernier commit
                    let repo = self.git_reader.get_repo();
                    match repo.head().and_then(|h| h.peel_to_commit()) {
                        Ok(commit) => match self.git_reader.get_commit_diff(&commit) {
                            Ok(d) => println!("{}", d),
                            Err(e) => eprintln!("{} {}", "Diff Error:".red(), e),
                        },
                        Err(_) => println!("No commits found to diff."),
                    }
                }

                // --- COMMANDES LSP ---
                "lsp" => match args.get(1) {
                    Some(&"start") => {
                        println!("Starting rust-analyzer...");
                        if let Err(e) = self.lsp_server.start(".", "rust-analyzer").await {
                            eprintln!("{} {}", "LSP Start Error:".red(), e);
                        }
                    }
                    Some(&"stop") => {
                        self.lsp_server.stop().await?;
                    }
                    Some(&"symbols") => {
                        if let Some(file_path) = args.get(2) {
                            self.handle_lsp_symbols(file_path).await;
                        } else {
                            println!("Usage: lsp symbols <file_path>");
                        }
                    }
                    _ => println!("Usage: lsp [start|stop|symbols <file_path>]"),
                },

                // Alias rapide pour les symboles
                "symbols" => {
                    if let Some(file_path) = args.get(1) {
                        self.handle_lsp_symbols(file_path).await;
                    } else {
                        println!("Usage: symbols <file_path>");
                    }
                }

                _ => println!("Unknown command: {}", args[0]),
            }
        }
        Ok(())
    }

    /// Logique pour traiter et afficher les symboles proprement
    async fn handle_lsp_symbols(&mut self, path: &str) {
        println!("Analyzing {}...", path.blue());
        match self.lsp_server.get_symbols(path).await {
            Ok(response) => {
                // On extrait le résultat de la réponse JSON-RPC
                if let Some(symbols) = response.get("result").and_then(|r| r.as_array()) {
                    if symbols.is_empty() {
                        println!("No symbols found in this file.");
                        return;
                    }

                    println!("{:<20} {:<10}", "SYMBOL".bold(), "KIND".bold());
                    println!("{}", "------------------------------------------".dimmed());

                    for s in symbols {
                        let name = s["name"].as_str().unwrap_or("?");
                        let kind = match s["kind"].as_u64() {
                            Some(12) => "Function".green(),
                            Some(13) => "Variable".yellow(),
                            Some(23) => "Struct".magenta(),
                            Some(2) => "Module".blue(),
                            Some(11) => "Interface".cyan(),
                            _ => "Other".normal(),
                        };
                        println!("{:<20} {:<10}", name, kind);
                    }
                } else if let Some(error) = response.get("error") {
                    eprintln!("{} {}", "LSP Error:".red(), error["message"]);
                }
            }
            Err(e) => eprintln!("{} {}", "Request Error:".red(), e),
        }
    }
}
