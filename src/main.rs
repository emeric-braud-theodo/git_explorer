mod cli;
mod git_reader;
mod lsp;
use cli::cli::CLI;

#[tokio::main] // N'oublie pas l'attribut tokio sur le main
async fn main() {
    // On ajoute .expect() pour extraire le CLI du Result
    let mut cli = CLI::new().expect("Échec de l'initialisation de la CLI");

    let _ = cli.listen().await;
}
