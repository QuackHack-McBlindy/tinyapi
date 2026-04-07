#!/usr/bin/env bash
set -euo pipefail


CRATE_NAME=$(grep '^name =' Cargo.toml | head -1 | sed 's/name = "\(.*\)"/\1/')
echo "Releasing crate: $CRATE_NAME"


if ! command -v cargo &> /dev/null; then
    echo "cargo not found"
    exit 1
fi

if ! git diff --quiet; then
    echo "🚫 Working directory is not clean. Commit or stash changes first."
    exit 1
fi



current_version=$(grep '^version =' Cargo.toml | sed 's/version = "\(.*\)"/\1/')
echo "Current version: $current_version"

IFS='.' read -r major minor patch <<< "$current_version"
new_patch=$((patch + 1))
new_version="$major.$minor.$new_patch"
echo "New version: $new_version"


cp Cargo.toml Cargo.toml.bak
cp README.md README.md.bak


if [[ "$OSTYPE" == "darwin"* ]]; then
    sed -i '' "s/^version = \".*\"/version = \"$new_version\"/" Cargo.toml
else
    sed -i "s/^version = \".*\"/version = \"$new_version\"/" Cargo.toml
fi


if [[ "$OSTYPE" == "darwin"* ]]; then
    sed -i '' "s/$CRATE_NAME = \".*\"/$CRATE_NAME = \"$new_version\"/" README.md
else
    sed -i "s/$CRATE_NAME = \".*\"/$CRATE_NAME = \"$new_version\"/" README.md
fi

echo "Updated version to $new_version"


git add Cargo.toml README.md
git commit -m "chore: bump version to $new_version"
git tag "v$new_version"


if cargo publish; then
    echo "✅ Published successfully"
    git push origin main --tags
    echo "🚀 Released version $new_version"
else
    echo "❌ cargo publish failed. Rolling back..."

    mv Cargo.toml.bak Cargo.toml
    mv README.md.bak README.md


    git reset --hard HEAD~1
    git tag -d "v$new_version"

    echo "Rollback complete. Version unchanged."
    exit 1
fi

rm -f Cargo.toml.bak README.md.bak
