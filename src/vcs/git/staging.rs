use git2::Repository;
use std::path::Path;

use crate::error::Result;

pub fn stage_file(repo: &Repository, path: &Path) -> Result<()> {
    let mut index = repo.index()?;
    if repo
        .workdir()
        .is_some_and(|workdir| workdir.join(path).exists())
    {
        index.add_path(path)?;
    } else {
        index.remove_path(path)?;
    }
    index.write()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn create_initial_commit(repo: &Repository, file_name: &str, content: &str) {
        fs::write(repo.workdir().unwrap().join(file_name), content)
            .expect("failed to write initial file");

        let mut index = repo.index().expect("failed to open index");
        index
            .add_path(Path::new(file_name))
            .expect("failed to add file to index");
        index.write().expect("failed to write index");

        let tree_id = index.write_tree().expect("failed to write tree");
        let tree = repo.find_tree(tree_id).expect("failed to find tree");
        let sig = git2::Signature::now("Test User", "test@example.com")
            .expect("failed to create signature");

        repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
            .expect("failed to create commit");
    }

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

    #[test]
    fn stage_file_removes_deleted_file_from_index() {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let repo = Repository::init(temp_dir.path()).expect("failed to init repo");
        create_initial_commit(&repo, "deleted.txt", "goodbye\n");

        fs::remove_file(temp_dir.path().join("deleted.txt")).expect("failed to delete file");

        stage_file(&repo, Path::new("deleted.txt")).unwrap();

        let reopened_repo = Repository::open(temp_dir.path()).expect("failed to reopen repo");
        let index = reopened_repo.index().expect("failed to reopen index");
        assert!(index.get_path(Path::new("deleted.txt"), 0).is_none());

        let head_tree = reopened_repo
            .head()
            .expect("failed to find HEAD")
            .peel_to_tree()
            .expect("failed to find HEAD tree");
        let diff = reopened_repo
            .diff_tree_to_index(Some(&head_tree), Some(&index), None)
            .expect("failed to diff HEAD against index");
        let delta = diff.deltas().next().expect("expected staged deletion");
        assert_eq!(delta.status(), git2::Delta::Deleted);
    }
}
