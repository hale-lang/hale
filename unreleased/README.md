# CHANGELOG fragments

One file per pull request, named by its number: `unreleased/<pr-number>.md`. It holds the change's CHANGELOG entry, worded exactly as it would read under `## Unreleased` in `CHANGELOG.md` (a `### heading` line and its bullets, or bullets alone). Open the PR first, then add the file under its number.

At release, `scripts/changelog-fold.sh vX.Y.Z "headline"` folds the fragments, in PR order, under the next version's heading in `CHANGELOG.md` and removes them. This README stays.
