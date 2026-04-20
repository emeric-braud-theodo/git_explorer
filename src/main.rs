mod cli;
mod git_reader;
use cli::cli::CLI;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = CLI::new()?;
    cli.listen();
    Ok(())
}
