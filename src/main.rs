extern crate core;
extern crate git2;
extern crate tempdir;

use git2::{Oid, Reference, Repository};

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

fn main() {
    let repo = match Repository::discover(".") {
        Ok(repo) => repo,
        Err(_) => panic!("you have to be inside of a git repository to use git-submit"),
    };
    let revs = revs_to_send(&repo);
    for rev in revs {
        println!("{}", rev);
    }
}

#[cfg(test)]
mod tests {
    use super::{branches, revs_to_send};

    use git2::{Oid, Repository, Signature, Tree};
    use std::fs;
    use tempdir::TempDir;

    // TODO(tg): make sure to clean up the repo we created if something goes wrong.
    fn init_test_repo(path: &str) {
        let repo = match Repository::init(path) {
            Ok(repo) => repo,
            Err(e) => panic!("error: {}", e),
        };

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

    fn new_tree<'a>(repo: &'a Repository, filename: &str, tree: Option<&Tree>) -> Tree<'a> {
        let mut builder = match repo.treebuilder(tree) {
            Ok(builder) => builder,
            Err(e) => panic!("error: {}", e),
        };
        let st = (0..40).map(|_| filename).collect::<String>();
        let oid = Oid::from_str(st.as_str()).unwrap();

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
}
