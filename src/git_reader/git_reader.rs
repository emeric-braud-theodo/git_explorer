use git2::Repository;

pub struct GitReader {
    repo: Repository,
}

impl GitReader {
    pub fn new() -> Result<Self, git2::Error> {
        let repository = Repository::discover(".")?;
        Ok(Self { repo: repository })
    }

    pub fn get_repo(&self) -> &Repository {
        &self.repo
    }

    pub fn get_head(&self) -> Result<String, git2::Error> {
        let head = self.repo.head()?;
        Ok(head.shorthand().unwrap_or("HEAD").to_string())
    }

    pub fn list_commits(&self) -> Result<Vec<String>, git2::Error> {
        let mut revwalk = self.repo.revwalk()?;
        revwalk.push_head()?;
        revwalk.set_sorting(git2::Sort::TIME)?;
        let mut messages = Vec::new();

        for id in revwalk {
            let oid = id?;
            let commit = self.repo.find_commit(oid)?;

            let message = commit.summary().unwrap_or("(no message)").to_string();
            messages.push(message);
        }
        Ok(messages)
    }

    pub fn get_file_content_at(
        &self,
        path: &str,
        commit: &git2::Commit,
    ) -> Result<String, git2::Error> {
        let tree = commit.tree()?;
        let entry = tree.get_path(std::path::Path::new(path))?;

        let object = entry.to_object(&self.repo)?;
        let blob = object
            .as_blob()
            .ok_or_else(|| git2::Error::from_str("Element is not a file (blob)"))?;

        let content = std::str::from_utf8(blob.content())
            .map_err(|_| git2::Error::from_str("File is not UTF-8"))?;

        Ok(content.to_string())
    }

    pub fn get_commit_diff(&self, commit: &git2::Commit) -> Result<String, git2::Error> {
        // 1. Récupérer le commit HEAD
        let current_tree = commit.tree()?;

        // 2. Récupérer l'arbre du parent (s'il existe)
        let parent_tree = if commit.parent_count() > 0 {
            Some(commit.parent(0)?.tree()?)
        } else {
            None
        };

        // 3. Créer le diff entre l'ancien arbre et le nouveau
        // Si pas de parent (premier commit), on compare avec "rien" (None)
        let diff = self
            .repo
            .diff_tree_to_tree(parent_tree.as_ref(), Some(&current_tree), None)?;

        // 4. Formater le diff en texte (format standard patch)
        let mut diff_text = String::new();
        diff.print(git2::DiffFormat::Patch, |_delta, _hunk, line| {
            let content = std::str::from_utf8(line.content()).unwrap_or("");
            match line.origin() {
                '+' => diff_text.push('+'),
                '-' => diff_text.push('-'),
                ' ' => diff_text.push(' '),
                _ => {}
            }
            diff_text.push_str(content);
            true // continuer l'itération
        })?;

        Ok(diff_text)
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
