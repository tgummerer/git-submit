extern crate core;
extern crate git2;
extern crate tempdir;

use git2::{Branch, Oid, Reference, Repository};
use std::env;
use std::process::Command;

fn revs_to_send(repo: &Repository) -> Vec<Oid> {
    let mut revwalk = match repo.revwalk() {
        Ok(revwalk) => revwalk,
        Err(_) => panic!("can't create revwalk"),
    };
    if let Err(e) = revwalk.push_head() {
        panic!("error: {}", e);
    };
    let ref_oids = branches(&repo).iter().map(|x| x.target().unwrap()).collect::<Vec<_>>();
    revwalk.take_while(|x| !ref_oids.contains(x)).collect()
}

fn branches(repo: &Repository) -> Vec<Reference> {
    let refs = match repo.references() {
        Ok(refs) => refs,
        Err(e) => panic!("{}", e),
    };
    let head = repo.head().unwrap();
    refs.filter(|x| x.is_branch() && x != &head).collect()
}

fn current_branch<'a>(repo: &'a Repository) -> Result<Branch<'a>, &'static str> {
    let branches = match repo.branches(None) {
        Ok(branches) => branches,
        Err(e) => panic!("error: {}", e),
    };
    // TODO(tg): This won't be very nice if we have multiple branches pointing to head.
    // We should error out in such a case.
    for branch in branches {
        let (branch_unwrap, _) = branch;
        if branch_unwrap.is_head() {
            return Ok(branch_unwrap);
        }
    }
    Err("no branch pointing to HEAD")
}

fn set_path(repo: &Repository) {
    let repo_root = repo.path();
    if let Err(e) = env::set_current_dir(repo_root) {
        panic!("error: {}", e);
    }
}

fn format_patches(revs: Vec<Oid>, branch_name: &str) {
    let mut command = Command::new("git");
    command.arg("format-patch");
    command.arg("-o");
    command.arg(format!("output-{}", branch_name));
    if revs.len() >= 3 {
        command.arg("--cover-letter");
    }
    command.arg(format!("{}~..{}", revs[revs.len() - 1], revs[0]));
    let output = command.output().unwrap_or_else(|e| panic!("error: {}", e));
    if !output.status.success() {
        panic!("format-patch failed");
    }
}

fn main() {
    let repo = match Repository::discover(".") {
        Ok(repo) => repo,
        Err(_) => panic!("you have to be inside of a git repository to use git-submit"),
    };
    set_path(&repo);
    let revs = revs_to_send(&repo);
    let branch = match current_branch(&repo) {
        Ok(branch) => branch,
        Err(e) => panic!("error: {}", e),
    };
    let branch_name = match branch.name() {
        Ok(None) => panic!("branch name not valid"),
        Ok(Some(name)) => name,
        Err(e) => panic!("error: {}", e),
    };
    format_patches(revs, branch_name);
}

#[cfg(test)]
mod tests {
    use super::{branches, current_branch, format_patches, revs_to_send, set_path};

    use git2::{Repository, Signature, Tree};
    use std::fs::{self, File};
    use std::io::Write;
    use std::path::Path;
    use tempdir::TempDir;

    // TODO(tg): make sure to clean up the repo we created if something goes wrong.
    fn init_test_repo(path: &str) {
        let repo = match Repository::init(path) {
            Ok(repo) => repo,
            Err(e) => panic!("error: {}", e),
        };
        set_path(&repo);

        let sig = Signature::now("A U Thor", "author@example.net").unwrap();

        let tree1 = new_tree(&repo, "1", None);
        let oid1 = repo.commit(Some("HEAD"), &sig, &sig, "commit 1", &tree1, &[]).unwrap();
        let tree2 = new_tree(&repo, "2", Some(&tree1));
        let commit1 = repo.find_commit(oid1).unwrap();
        let oid2 = repo.commit(Some("HEAD"), &sig, &sig, "commit 2", &tree2, &[&commit1])
            .unwrap();
        let commit2 = repo.find_commit(oid2).unwrap();
        if let Err(e) = repo.commit(Some("HEAD"), &sig, &sig, "commit 3",
                                &new_tree(&repo, "3", Some(&tree2)), &[&commit2]) {
            panic!("error: {}", e);
        }
        if let Err(e) = repo.branch("test", &commit1, false) {
            panic!("error: {}", e);
        };
    }

    fn write_file(file: &Path) {
        let mut f = match File::create(file) {
            Ok(f) => f,
            Err(e) => panic!("error: {}", e),
        };
        if let Err(e) = f.write_all(b"Hello it's me!") {
            panic!("error: {}", e);
        };
        if let Err(e) = f.sync_all() {
            panic!("error: {}", e);
        };
    }

    fn new_tree<'a>(repo: &'a Repository, filename: &str, tree: Option<&Tree>) -> Tree<'a> {
        let path = match repo.workdir() {
            Some(path) => path.join(filename),
            None => panic!("repository has to have a worktree"),
        };
        let file = path.as_path();
        write_file(file);
        let mut index = match repo.index() {
            Ok(index) => index,
            Err(e) => panic!("error: {}", e),
        };
        if let Err(e) = index.add_path(Path::new(filename)) {
            panic!("error: {}", e);
        }
        let oid = match index.write_tree_to(repo) {
            Ok(oid) => oid,
            Err(e) => panic!("error: {}", e),
        };

        let mut builder = match repo.treebuilder(tree) {
            Ok(builder) => builder,
            Err(e) => panic!("error: {}", e),
        };
        if let Err(e) = builder.insert(filename, oid, 0o100644) {
            panic!("error: {}", e);
        }
        match builder.write() {
            Ok(oid) => {
                match repo.find_tree(oid) {
                    Ok(tree) => tree,
                    Err(e) => panic!("error: {}", e),
                }
            },
            Err(e) => panic!("error: {}", e),
        }
    }

    #[test]
    fn test_branches() {
        let tempdir = Box::new(match TempDir::new("git-submit") {
            Ok(tmp) => tmp,
            Err(e) => panic!("error: {}", e),
        });
        let repo_path = match tempdir.path().to_str() {
            Some(dir) => dir,
            None => panic!("error: path isn't valid utf-8"),
        };
        init_test_repo(repo_path);
        let repo = match Repository::open(repo_path) {
            Ok(repo) => repo,
            Err(e) => panic!("error: {}", e),
        };

        let bs = branches(&repo);
        assert_eq!(bs.len(), 1);
        assert!(bs[0].is_branch());
        assert_eq!(bs[0].name(), Some("refs/heads/test"));

        if let Err(e) = fs::remove_dir_all(repo_path) {
            panic!("error: {}", e);
        }
    }

    #[test]
    fn test_revs_to_send() {
        let tempdir = Box::new(match TempDir::new("git-submit") {
            Ok(tmp) => tmp,
            Err(e) => panic!("error: {}", e),
        });
        let repo_path = match tempdir.path().to_str() {
            Some(dir) => dir,
            None => panic!("error: path isn't valid utf-8"),
        };
        init_test_repo(repo_path);
        let repo = match Repository::open(repo_path) {
            Ok(repo) => repo,
            Err(e) => panic!("error: {}", e),
        };

        let revs = revs_to_send(&repo);
        assert_eq!(revs.len(), 2);

        if let Err(e) = fs::remove_dir_all(repo_path) {
            panic!("error: {}", e);
        }
    }

    #[test]
    fn test_current_branch() {
        let tempdir = Box::new(match TempDir::new("git-submit") {
            Ok(tmp) => tmp,
            Err(e) => panic!("error: {}", e),
        });
        let repo_path = match tempdir.path().to_str() {
            Some(dir) => dir,
            None => panic!("error: path isn't valid utf-8"),
        };
        init_test_repo(repo_path);
        let repo = match Repository::open(repo_path) {
            Ok(repo) => repo,
            Err(e) => panic!("error: {}", e),
        };

        let branch = match current_branch(&repo) {
            Ok(branch) => branch,
            Err(e) => panic!("error: {}", e),
        };

        match branch.name() {
            Ok(name) => assert_eq!(name, Some("master")),
            Err(e) => panic!("error: {}", e),
        };

        if let Err(e) = fs::remove_dir_all(repo_path) {
            panic!("error: {}", e);
        }
    }

    #[test]
    fn test_format_patches() {
        let tempdir = Box::new(match TempDir::new("git-submit") {
            Ok(tmp) => tmp,
            Err(e) => panic!("error: {}", e),
        });
        let repo_path = match tempdir.path().to_str() {
            Some(dir) => dir,
            None => panic!("error: path isn't valid utf-8"),
        };
        init_test_repo(repo_path);
        let repo = match Repository::open(repo_path) {
            Ok(repo) => repo,
            Err(e) => panic!("error: {}", e),
        };
        set_path(&repo);

        format_patches(revs_to_send(&repo), "master");

        let patch_files = match fs::read_dir(format!("{}/.git/output-master", repo_path)) {
            Ok(files) => files,
            Err(e) => panic!("error: {}", e),
        };
        assert_eq!(patch_files.count(), 2);

        if let Err(e) = fs::remove_dir_all(repo_path) {
            panic!("error: {}", e);
        }
    }
}
