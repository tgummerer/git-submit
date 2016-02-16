# git-submit

This utility makes contributing workflows that use mailing lists,
especially those that use gmane to monitor them.

I originally got the idea from
[Thomas Rast](https://github.com/trast/git/wiki/Todo-items).  I tried
to implement it in git-send-email, but wasn't too happy with the
result.  Also some of the features I wanted did not really fit into
git itself, so I decided to write my own small thing.

If you are looking for something more graphical, Roberto Tyley's
[submitgit](https://github.com/rtyley/submitgit) is probably a good
alternative for you.

# How do I install it?

As this is written in [Rust](https://github.com/rust-lang/rust), it
needs [Cargo](https://crates.io) to be built.

Just run `cargo build` in the repo, and copy `target/debug/git_submit`
to anywhere in your `$PATH` or `$(git --exec-path)` as `git-submit`.
Then you can call it as `git submit`.

# How do I use it?

```
git submit [--to=<email>] [--cc=<email>] [--in-reply-to=<message-id>]
```

* `--to=<email>`
  Specify the email addresses, to which the patch series should be sent
  to.  Can be specified multiple times.

* `--cc=<email>`
  Specify the email addresses, to which  the patch series should be
  cc'd to.  Can be specified multiple times.

* `--in-reply-to=<message-id>`
  Specify the message id to which the patch series replies to.
  Automatically adds the email addresses it can get from the specified
  message-id from gmane to send-email.

# How does it work?

`git submit` offloads as much work from submitting a patch series as
possible, while still giving some control to you.

 1) When run, `git submit` starts from the current HEAD, and walks
    back the history until it encounters a commit that is the tip of
    another branch.  All the revisions that `git submit` walked
    through, excluding the tip of the branch that is encountered.

 2) If a reply-to option is given, `git submit` tries to get the to
    and cc addresses from the specified mail from gmane and add them
    to the list given using the `--to` and `--cc` arguments.

 3) `git format-patch` is called on all the revisions found in 1).

 4) `git submit` walks through the list of all patches and opens the
    editor specified by the `$EDITOR` environment variable for each of
    them, so the you can modify the patches.  This can be used to edit
    the cover letter (which is created for all patch series of 3
    patches or longer), commit message, comments on the commit, or
    even the patch itself (be careful with this though!)

 5) The current branch is re-built from the modified patches.  This
    way whatever you changed in the previous step will be kept in the
    history, and you can keep iterating on that.  Should `git am`
    however fail to apply a patch because of modifications that were
    made before, the branch will be restored to the previous state and
    the changes from before are lost.

 6) A lightweight tag is created with the name $currentbranch-vn,
    where x stands for the nth iteration of the patch series (the nth
    time `git submit` was successfully invoked normally).  This is
    used by `git submit` to keep track of the version of the patch
    series and can be used by you to keep track of the changes you
    made.

 7) The emails are sent to the recipients you specified and the ones
    `git submit` got from the message on gmane if `--in-reply-to` was
    specified.

 8) Time to celebrate :beer: :tada: (or to start writing more code).

# Warning

There may be bugs that will cause unexpected results.  If you
encounter such a bug, please file an issue.  You can also email me if
that's more up your alley.

# Apparently it's good to have this

Copyright (c) 2016, Thomas Gummerer

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in
all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
THE SOFTWARE.
