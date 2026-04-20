use git2::Repository;

pub struct GitReader {
    repo: Repository,
}

impl GitReader {
    pub fn new() -> Result<Self, git2::Error> {
        let repository = Repository::discover(".").expect("Cannot open repository");
        Ok(Self { repo: repository })
    }

    pub fn get_head(&self) -> Result<String, git2::Error> {
        let head = self.repo.head()?;
        Ok(head.shorthand().unwrap_or("HEAD").to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::Repository;
    use tempfile::TempDir;

    fn setup_test_repo() -> TempDir {
        let tmp_dir = TempDir::new().unwrap();
        // Initialise un vrai dépôt git dans le dossier temporaire
        Repository::init(tmp_dir.path()).unwrap();
        tmp_dir
    }

    #[test]
    fn test_git_reader_new_ok() {
        let tmp_dir = setup_test_repo();

        // On change le répertoire de travail pour le test
        std::env::set_current_dir(tmp_dir.path()).unwrap();

        let reader = GitReader::new();
        assert!(reader.is_ok());
    }

    #[test]
    fn test_get_head_on_empty_repo() {
        let tmp_dir = setup_test_repo();
        std::env::set_current_dir(tmp_dir.path()).unwrap();

        let reader = GitReader::new().unwrap();
        // Sur un dépôt vide sans commit, HEAD n'existe pas encore
        assert!(reader.get_head().is_err());
    }
}
