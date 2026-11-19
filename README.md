# devclean

Finds and reclaims disk space used by development artifacts. `scan` reports and changes nothing.
`clean` shows the same findings in a picker and runs only what you select, after a confirmation.

## Categories

- `build`: build output and dependency folders, detected by marker files (`target/` next to
  `Cargo.toml`, `bin/`/`obj/` next to a `.csproj`, `node_modules/` next to `package.json`,
  Python virtualenvs and caches, Gradle, CMake, Terraform, Swift, Zig). A folder inside a git work
  tree is skipped unless git ignores it.
- `worktree`: git worktrees that are missing on disk, or clean and merged into the default branch,
  whose upstream branch was deleted, or idle for 30 days (`--older-than`). Removal uses
  `git worktree remove`, which keeps the branch and refuses if there are local changes.
- `git-gc`: repos with many loose objects, many packs, or garbage files. Runs `git gc`.
- `cache`: machine-wide tool caches (NuGet, npm, Yarn, pnpm, uv, pip, Go, Cargo, Gradle, Maven,
  Homebrew, Xcode, Playwright, JetBrains). Uses the tool's own clean command when installed.
- `docker`: `docker system prune`, and `docker volume prune` as a separate item.

## Usage

```sh
just install                      # builds and copies to ~/bin
devclean scan ~/meps                       # report
devclean scan ~/meps --only build --older-than 60
devclean clean ~/meps --no-global          # pick and clean, skipping caches and Docker
devclean clean ~/meps --dry-run            # show the commands without running them
```

Sizes are on-disk estimates. Docker's estimate includes unused tagged images, which `prune`
without `-a` keeps.
