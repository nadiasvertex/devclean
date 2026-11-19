# devclean

devclean finds disk space used by development work and helps you reclaim it. It looks for build
output, stale git worktrees, git repositories that need compacting, tool caches, and unused Docker
data. `devclean scan` reports what it finds and changes nothing. `devclean clean` lets you choose
what to remove and asks before it removes anything.

devclean runs on macOS and Linux. Most cache locations it knows about are the macOS ones.

## Install

Download the archive for your platform from the
[releases page](https://github.com/nadiasvertex/devclean/releases), extract it, and put
`devclean` somewhere on your `PATH`:

```sh
tar -xzf devclean-v0.1.0-aarch64-apple-darwin.tar.gz
mv devclean-v0.1.0-aarch64-apple-darwin/devclean ~/bin/
```

On macOS, a downloaded binary may be blocked by Gatekeeper. Clear the quarantine flag with
`xattr -d com.apple.quarantine ~/bin/devclean`.

To build from source, install [rustup](https://rustup.rs), then run `just install` (copies the binary to
`~/bin`) or `cargo install --path .`. The repository pins its Rust version in `rust-toolchain.toml`,
and rustup installs that version on first build.

## Quick start

```sh
devclean scan ~/src            # see what can be reclaimed
devclean clean ~/src --dry-run # see the commands clean would run
devclean clean ~/src           # choose items and clean them
```

## Commands

### scan

`devclean scan [ROOTS]...` searches each root directory and prints findings grouped by category,
largest first, with an estimated total. With no roots it searches the current directory. Tool
caches and Docker are machine-wide, so they are reported regardless of the roots.

Each row shows the estimated size, the age in days (time since the newest file changed, or since
the last commit for worktrees), the kind of item, and its path. Some rows have a second line that
explains why the item was flagged or what cleaning it does.

Each category lists its 15 largest items. Pass `--all` to list everything.

### clean

`devclean clean [ROOTS]...` runs the same scan, then shows a checklist. Use the arrow keys to move,
space to check or uncheck a row, `a` to toggle all rows, and enter to continue. devclean shows the
total and asks for confirmation before it runs anything. Press escape to cancel.

Build folders are grouped into one row per project, meaning the git repository or worktree that
contains them, so thousands of `bin/` and `obj/` folders become a short list. These rows start
checked: uncheck the projects whose build output you want to keep. All other rows (worktrees,
`git gc`, caches, and Docker) start unchecked, so check the ones you want.

To keep a project out of the list entirely, use `--exclude`:

```sh
devclean clean ~/src --exclude '~/src/active-project/**' --exclude '**/vendor/**'
```

After cleaning, devclean prints each command as it runs it and reports the space reclaimed. It
exits with status 1 if any item failed.

`clean` accepts two options in addition to the scan options:

- `--dry-run` prints the command for each selected item and runs nothing.
- `--yes` skips the checklist and the confirmation and cleans every finding. Combine it with
  `--only` and the other filters to keep the selection narrow.

### Options

These options apply to both `scan` and `clean`.

| Option | Effect |
| --- | --- |
| `--only <list>` | Limit to these categories, comma-separated: `build`, `worktree`, `git-gc`, `cache`, `docker`. |
| `--no-global` | Skip tool caches and Docker. |
| `--older-than <days>` | Only include items untouched for at least this many days. Also sets the idle threshold for worktrees. |
| `--min-size <MiB>` | Hide build folders and caches smaller than this. The default is 10. |
| `--exclude <glob>` | Skip findings whose path matches the glob. Repeatable. A leading `~/` means your home directory. Quote the pattern so the shell does not expand it. `dir/**` matches everything under `dir` but not `dir` itself. |
| `--all` | List every finding instead of the largest 15 per category. |

## What devclean finds

### Build output (`build`)

devclean recognizes build and dependency folders by the project files next to them:

| Folder | Recognized when |
| --- | --- |
| `target` | next to `Cargo.toml` (Rust) or `pom.xml` (Maven) |
| `bin`, `obj` | next to a `.csproj`, `.fsproj`, or `.vbproj` file |
| `node_modules`, `dist`, `build`, `out`, `coverage` | next to `package.json` |
| `.next`, `.nuxt`, `.svelte-kit`, `.turbo`, `.parcel-cache`, `.angular`, `.vite` | next to `package.json` |
| `.venv`, `venv`, `env` | the folder contains `pyvenv.cfg` |
| `__pycache__`, `.pytest_cache`, `.mypy_cache`, `.ruff_cache`, `.tox` | always |
| `.gradle`, `build` | next to a Gradle build or settings file |
| `build`, `cmake-build-*` | next to `CMakeLists.txt`, or any `cmake-build-*` folder |
| `.terraform` | next to a `.tf` file |
| `.build` | next to `Package.swift` |
| `.zig-cache`, `zig-cache`, `zig-out` | next to `build.zig` |

Inside a git repository, a folder is only reported if git ignores it. This keeps devclean from
offering a folder that holds tracked files. Cleaning deletes the folder. Rebuilding or reinstalling
dependencies recreates it.

### Git worktrees (`worktree`)

devclean lists the linked worktrees of every repository it finds and reports a worktree when one
of these is true:

- Its directory no longer exists. Cleaning runs `git worktree prune`.
- It has no uncommitted changes and its branch is merged into the remote default branch, its
  upstream branch was deleted, or it has had no commits for 30 days (or `--older-than`). Cleaning
  runs `git worktree remove`.

Removing a worktree keeps its branch, so committed work is not lost. Git refuses to remove a
worktree that has uncommitted or untracked files. Locked worktrees are never reported.

devclean does not fetch. Deleted upstream branches are detected only after a
`git fetch --prune`.

### Git repositories (`git-gc`)

devclean reports a repository when it has 1,000 or more loose objects, 10 or more pack files, or
garbage files. Cleaning runs `git gc`. The size shown counts loose objects and garbage; the actual
savings after repacking vary.

### Tool caches (`cache`)

devclean reports caches for Cargo, npm, Yarn, pnpm, NuGet, Gradle, Maven, Homebrew, Go, pip, uv,
Playwright, Xcode DerivedData, iOS simulators, Terraform, and JetBrains IDEs. When the tool is
installed, cleaning uses its own command, such as `npm cache clean --force` or
`dotnet nuget locals all --clear`. Otherwise devclean deletes the cache folder. Tools download what
they need again on next use.

Some caches have side effects, which the report notes. The Maven repository also holds artifacts
from `mvn install`, and `brew cleanup` also removes old installed versions.

### Docker (`docker`)

When the Docker daemon is running, devclean reads `docker system df` and reports two items:

- `docker system prune` removes stopped containers, unused networks, dangling images, and build
  cache. The size shown includes unused tagged images, which this command keeps, so the actual
  savings may be smaller.
- `docker volume prune` removes unused anonymous volumes. These can hold local database data, so
  this is a separate item.

## Sizes

Sizes are on-disk usage. The estimated total counts a folder once when it sits inside another
finding, such as build output inside a stale worktree.

## Releasing

Raise the version, commit and push the change, then run `just release`:

```sh
just bump-version minor   # or major, or patch; updates Cargo.toml and Cargo.lock
git commit -am "Bump version to 0.2.0"
git push
just release              # tags HEAD as v0.2.0 and pushes the tag
```

`just release` stops if the tag already exists, if the version change is not committed, or if HEAD
has not been pushed to `master`. The pushed tag starts the release workflow, which tests the code,
builds archives for macOS and Linux on arm64 and x86_64, and publishes them with SHA-256 checksums
on a GitHub release.

## License

MIT
