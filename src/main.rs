extern crate git2;

use git2::Repository;

fn main() {
    let repo = match Repository::open (".") {
        Ok(repo) => repo,
        Err(_) => panic!("you have to be inside of a git repository to use git-submit"),
    };
    match repo.revwalk() {
        Ok(revwalk) => println!("{}", revwalk.count()),
        Err(_) => panic!("can't create revwalk"),
    };
}
