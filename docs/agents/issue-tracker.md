# Issue tracker: GitHub

Issues and specifications live in https://github.com/andikon/treerails/issues.
Use the gh CLI from this repository; it infers the repository from origin.

## Conventions

- Read: `gh issue view <number> --comments`.
- List: `gh issue list --state open --json number,title,body,labels`.
- Create: `gh issue create --title "..." --body-file <file>`.
- Update a description: `gh issue edit <number> --body-file <file>`.
- Comment: `gh issue comment <number> --body-file <file>`.
- Apply/remove labels: `gh issue edit <number> --add-label "..."` or `--remove-label "..."`.
- Close: `gh issue close <number>`.

Use UTF-8 files for multiline descriptions and comments.

“Publish to the issue tracker” means create a GitHub issue.
“Fetch the relevant ticket” means read the issue and its comments.

## Pull requests as a triage surface

**PRs as a request surface: no.**
