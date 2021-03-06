extern crate core;
extern crate email;
extern crate getopts;
extern crate git2;
extern crate hyper;
extern crate regex;
extern crate tempdir;

use email::Address;
use email::Mailbox;
use email::MimeMessage;
use getopts::Options;
use git2::{Branch, Error, ObjectType, Oid, Reference, Repository, ResetType, StatusOptions};
use git2::build::CheckoutBuilder;
use hyper::Client;
use regex::Regex;
use std::env;
use std::fs;
use std::io::{self, Read};
use std::process::{Command, Stdio};
use std::str;

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
    let repo_root = repo.workdir();
    env::set_current_dir(repo_root.unwrap()).unwrap();
}

fn format_patches(revs: &Vec<Oid>, branch_name: &str, version: u32) {
    let mut command = Command::new("git");
    command.arg("format-patch");
    command.arg("-o");
    command.arg(format!("output-{}", branch_name).replace("/", "_"));
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

fn send_emails(repo: &Repository, branch_name: &str, in_reply_to: Option<String>,
               to: Vec<String>, cc: Vec<String>) -> Result<(), io::Error> {
    let mut command = Command::new("git");
    command.arg("send-email");
    if to.is_empty() && cc.is_empty() {
        return Err(io::Error::new(io::ErrorKind::Other,
                                  "please specify at least one address"));
    }
    if !to.is_empty() {
        for addr in to {
            command.arg(format!("--to={}", addr));
        }
    }
    if !cc.is_empty() {
        for addr in cc {
            command.arg(format!("--cc={}", addr));
        }
    }
    if in_reply_to.is_some() {
        command.arg(format!("--in-reply-to={}", in_reply_to.unwrap()));
    }

    let path = repo.workdir().unwrap();
    let patch_files = try!(fs::read_dir(format!("{}/output-{}/", path.to_str().unwrap_or("./"),
                                                branch_name.replace("/", "_"))));
    let mut file_list = Vec::new();
    for file in patch_files {
        let f = try!(file);
        if f.path().to_str().is_some() {
            let st = f.path();
            file_list.push(st)
        }
    }
    file_list.sort();
    for file in file_list {
        command.arg(file);
    }
    let output = try!(command.output());
    println!("{}", str::from_utf8(output.stdout.as_slice()).unwrap());
    println!("{}", str::from_utf8(output.stderr.as_slice()).unwrap());
    Ok(())
}

fn edit_patches(repo: &Repository, branch_name: &str) -> Result<(), io::Error> {
    let path = repo.workdir().unwrap();
    let patch_files = try!(fs::read_dir(format!("{}/output-{}/", path.to_str().unwrap_or("./"),
                                                branch_name.replace("/", "_"))));
    let mut file_list = Vec::new();
    for file in patch_files {
        let f = try!(file);
        if f.path().to_str().is_some() {
            let st = f.path();
            file_list.push(st)
        } else {
            return Err(io::Error::new(io::ErrorKind::Other, "path is not valid utf-8"));
        }
    }
    file_list.sort();

    for file in file_list {
        let editor = match env::var("EDITOR") {
            Ok(editor) => editor,
            Err(_) => return Err(io::Error::new(io::ErrorKind::Other,
                                                "EDITOR environment variable has to be set")),
        };
        let mut editor_split = editor.split(" ");
        let mut command = Command::new(editor_split.next().unwrap());
        for es in editor_split {
            command.arg(es);
        }
        command.arg(file);
        command.stdout(Stdio::inherit());
        try!(command.output());
    }
    Ok(())
}

fn is_clean(repo: &Repository) -> Result<bool, Error> {
    let statuses = try!(repo.statuses(Some(&mut StatusOptions::new())));
    Ok(statuses.len() == 0)
}

fn rebuild_branch(repo: &Repository, original_revs: &Vec<Oid>, branch_name: &str)
                  -> Result<(), Error> {
    let obj = try!(repo.revparse_single(format!("{}~", original_revs[original_revs.len() - 1])
                                        .as_str()));
    try!(repo.reset(&obj, ResetType::Hard, Some(&mut CheckoutBuilder::new())));
    let path = repo.workdir().unwrap();
    let patch_files = match fs::read_dir(format!("{}/output-{}/", path.to_str().unwrap_or("./"),
                                                 branch_name.replace("/", "_"))) {
        Ok(files) => files,
        Err(_) => return Err(Error::from_str("could not read patch files")),
    };
    let re = Regex::new("(v[0-9]+-)?0000.*?").unwrap();

    let mut file_list = Vec::new();
    for file in patch_files {
        let f = match file {
            Ok(file) => file,
            Err(e) => return Err(Error::from_str(format!("{}", e).as_str())),
        };
        if f.path().to_str().is_some() {
            let st = f.path();
            file_list.push(st)
        } else {
            return Err(Error::from_str("path is not valid utf-8"));
        }
    }
    file_list.sort();

    for file in file_list {
        match file.to_str() {
            Some(filename) => if re.is_match(filename) {
                continue;
            },
            None => continue,
        };
        let mut command = Command::new("git");
        command.arg("am");
        command.arg("--3way");
        command.arg(file.to_str().unwrap());
        match command.output() {
            Ok(output) => if !output.status.success() {
                return Err(Error::from_str("git am unsuccessful"));
            },
            Err(_) => return Err(Error::from_str("git am failed")),
        };
    }
    Ok(())
}

fn remove_patches(repo: &Repository, branch_name: &str) {
    fs::remove_dir_all(format!("{}/output-{}/", repo.workdir().unwrap().to_str().unwrap_or("./"),
                               branch_name.replace("/", "_"))).unwrap();
}

fn remove_tag(repo: &Repository, branch_name: &str, version: u32) {
    repo.tag_delete(format!("{}-v{}", branch_name, version).as_str()).unwrap();
}

fn format_addr(mb: Mailbox) -> String {
    match mb.name {
        Some(name) => format!("{} <{}>", name, mb.address),
        None => mb.address,
    }
}

fn find_addresses(command_line: Vec<String>, reply_to: Option<String>, addr_type: String)
           -> Result<Vec<String>, hyper::error::Error> {
    let mut addresses = command_line;
    match reply_to {
        Some(r) => {
            let client = Client::new();
            let article_res =
                try!(client.get(format!("http://mid.gmane.org/{}", r).as_str()).send());
            let mut raw_res = try!(client.get(
                format!("{}/raw", article_res.url.serialize()).as_str()).send());

            let mut body = String::new();
            raw_res.read_to_string(&mut body).unwrap();
            let header_map = MimeMessage::parse(body.as_str()).unwrap().headers;

            for addr in header_map.get_value::<Vec<Address>>(addr_type).unwrap() {
                match addr {
                    Address::Mailbox(mb) => addresses.push(format_addr(mb)),
                    Address::Group(_, g) => {
                        for mb in g {
                            addresses.push(format_addr(mb));
                        }
                    },
                };
            };
        },
        None => (),
    };
    Ok(addresses)
}

fn main() {
    let args: Vec<String> = env::args().collect();

    let mut opts = Options::new();
    opts.optmulti("", "to", "set to addresses", "to");
    opts.optmulti("", "cc", "set cc addresses", "cc");
    opts.optopt("", "in-reply-to", "reply to message-id", "message-id");
    opts.optflag("h", "help", "print this help menu");
    let matches = match opts.parse(&args[1..]) {
        Ok(m) => m,
        Err(e) => panic!("error: {}", e),
    };
    if matches.opt_present("h") {
        print!("{}", opts.usage(&format!("usage: [options]")));
        return;
    }
    let to_only: Vec<String> = find_addresses(matches.opt_strs("to"),
                                              matches.opt_str("in-reply-to"),
                                              String::from("To")).unwrap();
    // Add the from address to the to list as well.
    let to: Vec<String> = find_addresses(to_only, matches.opt_str("in-reply-to"),
                                         String::from("From")).unwrap();
    let cc: Vec<String> = find_addresses(matches.opt_strs("cc"),
                                         matches.opt_str("in-reply-to"),
                                         String::from("Cc")).unwrap();

    let repo = Repository::discover(".").unwrap();
    match is_clean(&repo) {
        Ok(clean) => if !clean {
            panic!("git-submit can't be run with changes in the working tree");
        },
        Err(e) => panic!("error: {}", e),
    }
    set_path(&repo);
    let revs = revs_to_send(&repo).unwrap();
    let branch = current_branch(&repo).unwrap();
    let branch_name = match branch.name() {
        Ok(None) => panic!("branch name not valid"),
        Ok(Some(name)) => name,
        Err(e) => panic!("error: {}", e),
    };
    let version = find_version(&repo, branch_name).unwrap();
    if version > 1 && matches.opt_str("in-reply-to").is_none() {
        panic!("This is version {} of the patch series, --in-reply-to=<previous-message-id> should be used",
               version);
    }
    format_patches(&revs, branch_name, version);
    edit_patches(&repo, branch_name).unwrap();
    let head = repo.head().unwrap();
    if let Err(e) = rebuild_branch(&repo, &revs, branch_name) {
        repo.reset(&head.peel(ObjectType::Any).unwrap(), ResetType::Hard,
                   Some(&mut CheckoutBuilder::new())).unwrap();
        remove_patches(&repo, branch_name);
        panic!("error: {}", e);
    };
    if let Err(e) = tag_version(&repo, branch_name, version) {
        remove_patches(&repo, branch_name);
        panic!("error: {}", e);
    };
    if let Err(e) = send_emails(&repo, branch_name, matches.opt_str("in-reply-to"), to, cc) {
        remove_tag(&repo, branch_name, version);
        panic!("error: {}", e);
    };
    remove_patches(&repo, branch_name);
}

#[cfg(test)]
mod tests {
    use super::{branches, current_branch, edit_patches, find_addresses, find_version,
                format_addr, format_patches, remove_patches, remove_tag, revs_to_send,
                set_path, tag_version};

    use email::Mailbox;
    use git2::{Error, Repository, Signature, Tree};
    use std::env;
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
        format_patches(&revs, "master", 1);

        let patch_files = fs::read_dir(format!("{}/output-master", repo_path)).unwrap();
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

    #[test]
    fn test_remove_patches() {
        let tempdir = Box::new(TempDir::new("git-submit").unwrap());
        let repo_path = tempdir.path().to_str().unwrap();
        init_test_repo(repo_path).unwrap();
        let repo = Repository::open(repo_path).unwrap();
        set_path(&repo);

        let revs = revs_to_send(&repo).unwrap();
        format_patches(&revs, "master", 1);
        remove_patches(&repo, "master");
        let files = fs::read_dir(format!("{}/output-master", repo_path));
        assert!(files.is_err());

        fs::remove_dir_all(repo_path).unwrap();
    }

    #[test]
    fn test_remove_tag() {
        let tempdir = Box::new(TempDir::new("git-submit").unwrap());
        let repo_path = tempdir.path().to_str().unwrap();
        init_test_repo(repo_path).unwrap();
        let repo = Repository::open(repo_path).unwrap();

        tag_version(&repo, "master", 1).unwrap();
        let tag = repo.find_reference("refs/tags/master-v1").unwrap();
        assert!(tag.is_tag());
        assert_eq!(find_version(&repo, "master").unwrap(), 2);
        remove_tag(&repo, "master", 1);
        let tag_result = repo.find_reference("refs/tags/master-v1");
        assert!(tag_result.is_err());

        fs::remove_dir_all(repo_path).unwrap();
    }

    #[test]
    fn test_edit_patches() {
        let tempdir = Box::new(TempDir::new("git-submit").unwrap());
        let repo_path = tempdir.path().to_str().unwrap();
        init_test_repo(repo_path).unwrap();
        let repo = Repository::open(repo_path).unwrap();
        set_path(&repo);

        let revs = revs_to_send(&repo).unwrap();
        format_patches(&revs, "master", 1);
        env::set_var("EDITOR", "truncate --size=0");
        edit_patches(&repo, "master").unwrap();
        let patch_files = fs::read_dir(format!("{}/output-master", repo_path)).unwrap();
        for file in patch_files {
            assert_eq!(file.unwrap().metadata().unwrap().len(), 0);
        }

        fs::remove_dir_all(repo_path).unwrap();
    }

    #[test]
    fn test_find_address_name() {
        let mb = Mailbox::new_with_name(String::from("Test Name"),
                                        String::from("test@example.com"));
        assert_eq!(format_addr(mb), String::from("Test Name <test@example.com>"));
    }

    #[test]
    fn test_find_address_no_name() {
        let mb = Mailbox::new(String::from("test@example.com"));
        assert_eq!(format_addr(mb), String::from("test@example.com"));
    }

    #[test]
    fn test_find_addresses_command_line() {
        assert_eq!(find_addresses(vec!(String::from("test@example.com"),
                                       String::from("snd@example.com")), None,
                                  String::from("To")).unwrap(),
                   vec!(String::from("test@example.com"),
                        String::from("snd@example.com")));
    }

    #[test]
    fn test_find_to_mail() {
        assert_eq!(find_addresses(Vec::new(),
                                  Some(String::from(
                                      "1453136238-19448-1-git-send-email-t.gummerer@gmail.com")),
                                  String::from("To")).unwrap(),
                   vec!(String::from("git@vger.kernel.org")));
    }

    #[test]
    fn test_find_cc_mail() {
        assert_eq!(find_addresses(Vec::new(),
                                  Some(String::from(
                                      "1453136238-19448-1-git-send-email-t.gummerer@gmail.com")),
                                  String::from("Cc")).unwrap(),
                   vec!(String::from("peff@peff.net"),
                        String::from("bturner@atlassian.com"),
                        String::from("gitster@pobox.com"),
                        String::from("pedrorijo91@gmail.com"),
                        String::from("Thomas Gummerer <t.gummerer@gmail.com>")));
    }

    #[test]
    fn test_find_combined_command_line_to_mail() {
        assert_eq!(find_addresses(vec!(String::from("test@example.com")),
                                  Some(String::from(
                                      "1453136238-19448-1-git-send-email-t.gummerer@gmail.com")),
                                  String::from("To")).unwrap(),
                   vec!(String::from("test@example.com"),
                        String::from("git@vger.kernel.org")));
    }

    #[test]
    fn test_add_from_addresses() {
        let to_only = find_addresses(vec!(String::from("test@example.com")),
                                     Some(String::from(
                                         "1453136238-19448-1-git-send-email-t.gummerer@gmail.com")),
                                     String::from("To")).unwrap();
        assert_eq!(to_only, vec!(String::from("test@example.com"),
                                 String::from("git@vger.kernel.org")));
        assert_eq!(find_addresses(to_only,
                                  Some(String::from(
                                      "1453136238-19448-1-git-send-email-t.gummerer@gmail.com")),
                                  String::from("From")).unwrap(),
                   vec!(String::from("test@example.com"),
                        String::from("git@vger.kernel.org"),
                        String::from("Thomas Gummerer <t.gummerer@gmail.com>")));
    }
}
