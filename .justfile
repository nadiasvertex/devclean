# List recipes
default:
    @just --list

[doc('Build a release binary.')]
[group('build')]
build:
    cargo build --release

[doc('Build and install the release binary to ~/bin.')]
[group('build')]
install: build
    mkdir -p ~/bin
    install -m 755 target/release/devclean ~/bin/devclean

[doc('Raise the Cargo.toml package version by "major", "minor" or "patch".')]
[group('release')]
bump-version part:
    #!/usr/bin/env bash
    set -euo pipefail
    case "{{ part }}" in
      major|minor|patch) ;;
      *) echo "part must be major, minor, or patch (got '{{ part }}')" >&2; exit 1 ;;
    esac
    current=$(sed -nE 's/^version = "([0-9]+\.[0-9]+\.[0-9]+)"$/\1/p' Cargo.toml | head -1)
    if [ -z "$current" ]; then
      echo "Cargo.toml has no 'version = \"X.Y.Z\"' line" >&2
      exit 1
    fi
    IFS=. read -r major minor patch <<< "$current"
    case "{{ part }}" in
      major) major=$((major + 1)); minor=0; patch=0 ;;
      minor) minor=$((minor + 1)); patch=0 ;;
      patch) patch=$((patch + 1)) ;;
    esac
    next="$major.$minor.$patch"
    sed -E -i.bak "s/^version = \"$current\"$/version = \"$next\"/" Cargo.toml
    rm Cargo.toml.bak
    cargo update --offline --quiet -p devclean
    echo "$current -> $next"

[doc('Tag HEAD as v<version> from Cargo.toml and push the tag, which starts the GitHub release build.')]
[group('release')]
release:
    #!/usr/bin/env bash
    set -euo pipefail
    version=$(sed -nE 's/^version = "([0-9]+\.[0-9]+\.[0-9]+)"$/\1/p' Cargo.toml | head -1)
    if [ -z "$version" ]; then
      echo "Cargo.toml has no 'version = \"X.Y.Z\"' line" >&2
      exit 1
    fi
    tag="v$version"
    if git rev-parse -q --verify "refs/tags/$tag" >/dev/null \
       || git ls-remote --exit-code --tags origin "refs/tags/$tag" >/dev/null 2>&1; then
      echo "Tag $tag already exists. Run 'just bump-version <major|minor|patch>' and commit the change first." >&2
      exit 1
    fi
    if ! git diff --quiet HEAD -- Cargo.toml Cargo.lock; then
      echo "Cargo.toml or Cargo.lock has uncommitted changes. Commit the version bump before releasing." >&2
      exit 1
    fi
    git fetch --quiet origin
    if ! git merge-base --is-ancestor HEAD origin/master; then
      echo "HEAD is not on origin/master. Push your commits before releasing." >&2
      exit 1
    fi
    git tag -a "$tag" -m "devclean $tag"
    git push origin "$tag"
    echo "Pushed $tag"
