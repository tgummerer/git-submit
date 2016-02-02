extern crate core;
extern crate git2;
extern crate tempdir;

use git2::{Branch, Error, Oid, Reference, Repository};
use std::env;
use std::process::Command;

fn revs_to_send(repo: &Repository) -> Result<Vec<Oid>, Error> {
    let mut revwalk = try!(repo.revwalk());
    try!(revwalk.push_head());
    let ref_oids = try!(branches(&repo)).iter().map(|x| x.target().unwrap()).collect::<Vec<_>>();
    Ok(revwalk.take_while(|x| !ref_oids.contains(x)).collect())
}

fn branches(repo: &Repository) -> Result<Vec<Reference>, Error> {
    let refs = try!(repo.references());
    let head = repo.head().unwrap();
    Ok(refs.filter(|x| x.is_branch() && x != &head).collect())
}

fn current_branch<'a>(repo: &'a Repository) -> Result<Branch<'a>, Error> {
    let branches = try!(repo.branches(None));
    // TODO(tg): This won't be very nice if we have multiple branches pointing to head.
    // We should error out in such a case.
    for branch in branches {
        let (branch_unwrap, _) = branch;
        if branch_unwrap.is_head() {
            return Ok(branch_unwrap);
        }
    }
    Err(Error::from_str("no branch pointing to HEAD"))
}

fn set_path(repo: &Repository) {
    let repo_root = repo.path();
    env::set_current_dir(repo_root).unwrap();
}

fn format_patches(revs: Vec<Oid>, branch_name: &str, version: u32) {
    let mut command = Command::new("git");
    command.arg("format-patch");
    command.arg("-o");
    command.arg(format!("output-{}", branch_name));
    if revs.len() >= 3 {
        command.arg("--cover-letter");
    }
    if version > 1 {
        command.arg(format!("-v{}", version));
    }
    command.arg(format!("{}~..{}", revs[revs.len() - 1], revs[0]));
    let output = command.output().unwrap_or_else(|e| panic!("error: {}", e));
    if !output.status.success() {
        panic!("format-patch failed");
    }
}

fn find_version(repo: &Repository, branch_name: &str) -> Result<u32, Error> {
    let tags = try!(repo.tag_names(Some(format!("{}-v*", branch_name).as_str())));
    let mut max = 1;
    for tag in tags.iter() {
        match tag {
            Some(tag) => {
                match tag.replace(format!("{}-v", branch_name).as_str(), "").parse::<u32>() {
                    Ok(num) => {
                        if num >= max {
                            max = num + 1;
                        }
                    },
                    Err(_) => ()
                }
            },
            None => (),
        }
    }
    Ok(max)
}

fn tag_version(repo: &Repository, branch_name: &str, version: u32) -> Result<(), Error> {
    let branch = try!(repo.revparse_single(branch_name));
    try!(repo.tag_lightweight(format!("{}-v{}", branch_name, version).as_str(), &branch, true));
    Ok(())
}

fn main() {
    let repo = Repository::discover(".").unwrap();
    set_path(&repo);
    let revs = revs_to_send(&repo).unwrap();
    let branch = current_branch(&repo).unwrap();
    let branch_name = match branch.name() {
        Ok(None) => panic!("branch name not valid"),
        Ok(Some(name)) => name,
        Err(e) => panic!("error: {}", e),
    };
    let version = find_version(&repo, branch_name).unwrap();
    format_patches(revs, branch_name, version);
    tag_version(&repo, branch_name, version).unwrap();
}

#[cfg(test)]
mod tests {
    use super::{branches, current_branch, find_version, format_patches, revs_to_send, set_path,
                tag_version};

    use git2::{Error, Repository, Signature, Tree};
    use std::fs::{self, File};
    use std::io::{self, Write};
    use std::path::Path;
    use tempdir::TempDir;

    fn init_test_repo(path: &str) -> Result<(), Error> {
        let repo = try!(Repository::init(path));
        set_path(&repo);

        let sig = Signature::now("A U Thor", "author@example.net").unwrap();

        let tree1 = new_tree(&repo, "1", None);
        let oid1 = repo.commit(Some("HEAD"), &sig, &sig, "commit 1", &tree1, &[]).unwrap();
        let tree2 = new_tree(&repo, "2", Some(&tree1));
        let commit1 = repo.find_commit(oid1).unwrap();
        let oid2 = repo.commit(Some("HEAD"), &sig, &sig, "commit 2", &tree2, &[&commit1])
            .unwrap();
        let commit2 = repo.find_commit(oid2).unwrap();
        try!(repo.commit(Some("HEAD"), &sig, &sig, "commit 3",
                         &new_tree(&repo, "3", Some(&tree2)), &[&commit2]));
        try!(repo.branch("test", &commit1, false));
        Ok(())
    }

    fn write_file(file: &Path) -> Result<(), io::Error> {
        let mut f = try!(File::create(file));
        try!(f.write_all(b"Hello it's me!"));
        try!(f.sync_all());
        Ok(())
    }

    fn new_tree<'a>(repo: &'a Repository, filename: &str, tree: Option<&Tree>) -> Tree<'a> {
        let pathbuf = repo.workdir().unwrap().join(filename);
        let file = pathbuf.as_path();
        write_file(file).unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new(filename)).unwrap();
        let oid = index.write_tree_to(repo).unwrap();

        let mut builder = repo.treebuilder(tree).unwrap();
        builder.insert(filename, oid, 0o100644).unwrap();
        repo.find_tree(builder.write().unwrap()).unwrap()
    }

    #[test]
    fn test_branches() {
        let tempdir = Box::new(TempDir::new("git-submit").unwrap());
        let repo_path = tempdir.path().to_str().unwrap();
        init_test_repo(repo_path).unwrap();
        let repo = Repository::open(repo_path).unwrap();

        let bs = branches(&repo).unwrap();
        assert_eq!(bs.len(), 1);
        assert!(bs[0].is_branch());
        assert_eq!(bs[0].name(), Some("refs/heads/test"));

        fs::remove_dir_all(repo_path).unwrap();
    }

    #[test]
    fn test_revs_to_send() {
        let tempdir = Box::new(TempDir::new("git-submit").unwrap());
        let repo_path = tempdir.path().to_str().unwrap();
        init_test_repo(repo_path).unwrap();
        let repo = Repository::open(repo_path).unwrap();

        let revs = revs_to_send(&repo).unwrap();
        assert_eq!(revs.len(), 2);

        fs::remove_dir_all(repo_path).unwrap();
    }

    #[test]
    fn test_current_branch() {
        let tempdir = Box::new(TempDir::new("git-submit").unwrap());
        let repo_path = tempdir.path().to_str().unwrap();
        init_test_repo(repo_path).unwrap();
        let repo = Repository::open(repo_path).unwrap();

        let branch = current_branch(&repo).unwrap();

        let branch_name = branch.name().unwrap();
        assert_eq!(branch_name, Some("master"));

        fs::remove_dir_all(repo_path).unwrap();
    }

    #[test]
    fn test_format_patches() {
        let tempdir = Box::new(TempDir::new("git-submit").unwrap());
        let repo_path = tempdir.path().to_str().unwrap();
        init_test_repo(repo_path).unwrap();
        let repo = Repository::open(repo_path).unwrap();
        set_path(&repo);

        let revs = revs_to_send(&repo).unwrap();
        format_patches(revs, "master", 1);

        let patch_files = fs::read_dir(format!("{}/.git/output-master", repo_path)).unwrap();
        assert_eq!(patch_files.count(), 2);

        fs::remove_dir_all(repo_path).unwrap();
    }

    #[test]
    fn test_find_correct_version() {
        let tempdir = Box::new(TempDir::new("git-submit").unwrap());
        let repo_path = tempdir.path().to_str().unwrap();
        init_test_repo(repo_path).unwrap();
        let repo = Repository::open(repo_path).unwrap();

        assert_eq!(find_version(&repo, "master").unwrap(), 1);

        let master = repo.revparse_single("master").unwrap();
        repo.tag_lightweight("master-v1", &master, false).unwrap();
        assert_eq!(find_version(&repo, "master").unwrap(), 2);

        fs::remove_dir_all(repo_path).unwrap();
    }

    #[test]
    fn test_tag_version() {
        let tempdir = Box::new(TempDir::new("git-submit").unwrap());
        let repo_path = tempdir.path().to_str().unwrap();
        init_test_repo(repo_path).unwrap();
        let repo = Repository::open(repo_path).unwrap();

        tag_version(&repo, "master", 1).unwrap();
        let tag = repo.find_reference("refs/tags/master-v1").unwrap();
        assert!(tag.is_tag());
        assert_eq!(find_version(&repo, "master").unwrap(), 2);

        fs::remove_dir_all(repo_path).unwrap();
    }
}
