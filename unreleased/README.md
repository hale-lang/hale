# CHANGELOG fragments

One file per pull request, named by its number: `unreleased/<pr-number>.md`. It holds the change's CHANGELOG entry, worded exactly as it would read under `## Unreleased` in `CHANGELOG.md` (a `### heading` line and its bullets, or bullets alone). Open the PR first, then add the file under its number.

Every fragment declares its breaking changes ([STABILITY.md](../STABILITY.md)): a line reading exactly `Breaking: none`, or a `### Breaking` heading with one bullet per break and its migration. `scripts/changelog-check.sh` checks it in CI.

```markdown
### Fixed
- `hale check` refuses ...

Breaking: none
```

```markdown
### Breaking
- `old_form` is refused; write `new_form` instead.

### Changed
- ...
```

At release, `scripts/changelog-fold.sh vX.Y.Z "headline"` folds the fragments, in PR order, under the next version's heading in `CHANGELOG.md`, gathers their Breaking bullets under one `### Breaking` heading at the top of the version, and removes them. This README stays.
