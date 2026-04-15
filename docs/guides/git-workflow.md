# Git Workflow & Collaboration Guide

To maintain a pristine project history, avoid merge conflicts on core logic, and ensure `main` is always production-ready, we follow a strict **Trunk-Based Development** workflow with short-lived Feature Branches.

## 1. Branch Naming Conventions
All branches must be created from `main` and follow this format: `<type>/<kebab-case-description>`

* `feat/` - New features (e.g., `feat/postgres-provisioning`)
* `fix/` - Bug fixes (e.g., `fix/jwt-expiration-bug`)
* `chore/` - Maintenance, dependencies, or configuration (e.g., `chore/update-angular-v17`)
* `docs/` - Documentation updates (e.g., `docs/api-specs-auth`)
* `hotfix/` - Urgent production fixes (e.g., `hotfix/k8s-crash-loop`)

## 2. Conventional Commits
Every commit must follow the [Conventional Commits](https://www.conventionalcommits.org/) specification. This allows us to auto-generate changelogs and easily read the history.

**Format:** `type(scope): description`
* **scope** is optional but recommended (e.g., `api`, `ui`, `k8s`, `db`).
* **description** must be lowercase and imperative (e.g., "add", not "added" or "adds").

**Examples:**
* `feat(db): add postgres instances table`
* `fix(ui): resolve overflow issue on project card`
* `refactor(api): extract jwt generation to shared module`
* `test(api): add coverage for project creation`

## 3. The Daily Workflow (Step-by-Step)

### Starting New Work
Always start fresh from the latest `main`.
```bash
git checkout main
git pull origin main
git checkout -b feat/your-feature-name
```

### Syncing with Main (The Rebase Rule)
**DO NOT** use `git merge main` into your feature branch. It creates ugly merge commits. Instead, keep your branch up-to-date using `rebase`.

If your teammate merges a PR to `main` while you are still working on your branch:
```bash
git fetch origin
git rebase origin/main
```
*If there are conflicts, Git will pause. Fix the files, run `git add .`, and then run `git rebase --continue`.*

Since you rewrote your local history with rebase, you must force push. **Always use `--force-with-lease`** (never `--force` alone, to prevent accidentally overwriting a teammate's commits if you are sharing the branch).
```bash
git push --force-with-lease origin feat/your-feature-name
```

## 4. Pull Request (PR) Protocol
A feature is not done until it is merged into `main`.

1. **Self-Review:** Before opening a PR, review your own diff. Ensure no `console.log()` or `println!()` are left behind.
2. **Draft PRs:** If you need feedback early, open a PR as a "Draft".
3. **Approval:** A PR requires **at least 1 approval** from the other team member.
4. **CI Checks:** All automated tests (TDD) must pass.
5. **Merging:** Use the **Squash and Merge** strategy in GitHub/GitLab. This squashes all your small commits (e.g., "wip", "fix typo") into one clean commit on `main`.

## 5. Emergency Protocol (Hotfixes)
If a critical bug is found in production:
1. Create a branch directly from `main`: `git checkout -b hotfix/critical-bug`
2. Fix the bug and write a test to ensure it doesn't happen again.
3. Open a PR, get an immediate review, and Squash & Merge.
4. The CI/CD pipeline deploys `main` to production immediately.