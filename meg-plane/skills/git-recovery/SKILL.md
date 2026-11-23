---
name: git-recovery
description: Recover lost, detached or orphaned git commits and complete a conflicted merge.
triggers: [lost commit, cannot find, can't find, reflog, checkout master, merge conflict, detached head, orphaned, unrecoverable, restore lost, merge conflict, "merge them into"]
required_tools: [run_command, read_file]
---

## Procedure Guidelines

Use this when work has disappeared from the working tree but the user is confident they made
it. The commits are usually NOT lost - git keeps them until garbage collection, and `git reflog`
is the index that says where they went.

### 1. Read the reflog before concluding anything

```
git reflog --date=iso
```

The reflog records every position `HEAD` has held, including commits that no branch points at.
Look for an entry whose message is a COMMIT rather than a `checkout: moving from X to Y` - that
is the work the user is looking for, and the hash on that line is it.

Report the candidate hash to the user before acting on it. There is usually more than one and
they are choosing which.

### 2. Look at the commit, do not merge it blind

```
git show <hash> --stat
git diff HEAD <hash>
```

`git show` is frequently refused by a sandbox, and the diff is the fallback that answers the
same question. If the diff is empty the commit is not the work; keep going rather than merging
it.

### 3. Merge, and expect a conflict

```
git merge <hash>
```

A conflict is the NORMAL outcome when the work touched a file that has since changed. It is not
a failure and it is not a reason to abort. Read the conflicted file before touching it.

### 4. Excise the conflict markers - correctly

A conflicted file has three marker lines:

```
<<<<<<< HEAD
ours
=======
theirs
>>>>>>> <hash>
```

Keep the side that is the user's work and delete the markers AND the other side. The mistake
to avoid is deleting only the markers, which leaves BOTH bodies in the file and produces
content that matches neither commit.

To take the incoming side wholesale, which is right when the commit being merged IS the lost
work:

```
git checkout --theirs <file>
```

To take the current branch's side, `--ours`. Both are only correct when the whole file should
come from one side; a genuine content conflict needs a real edit.

### 5. Verify, then commit the merge

```
git status
git diff --check
git add <file>
git commit -m "merge lost commit"
```

`git commit` is what clears `MERGE_HEAD`. Until it runs, the repository is mid-merge and the
merge is not done no matter what the files look like.

### 6. Confirm against evidence, not belief

`git log --oneline -5` and `git status` should show the merge commit and a clean tree. A run
that ends while `MERGE_HEAD` still exists has not finished, however complete the conflict
markers look.
