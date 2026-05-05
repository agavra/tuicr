use git2::Repository;
use std::path::Path;

use crate::error::Result;

pub fn stage_file(repo: &Repository, path: &Path) -> Result<()> {
    let mut index = repo.index()?;
    index.add_path(path)?;
    index.write()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn stage_file_adds_to_index() {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let repo = Repository::init(temp_dir.path()).expect("failed to init repo");

        let file_path = temp_dir.path().join("test.txt");
        fs::write(&file_path, "hello\n").unwrap();

        stage_file(&repo, Path::new("test.txt")).unwrap();

        let index = repo.index().unwrap();
        assert!(index.get_path(Path::new("test.txt"), 0).is_some());
    }
}
