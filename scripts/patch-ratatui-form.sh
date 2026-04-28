#!/bin/bash
set -e

REPO_ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." &> /dev/null && pwd)

if [ ! -d "$REPO_ROOT/.tmp/ratatui-form" ]; then
    echo ">>> Cloning and patching ratatui-form..."
    mkdir -p "$REPO_ROOT/.tmp"
    git clone https://github.com/DavidLiedle/ratatui-form "$REPO_ROOT/.tmp/ratatui-form"
    sed -i 's/ratatui = "0.29"/ratatui = "0.30"/' "$REPO_ROOT/.tmp/ratatui-form/Cargo.toml"
    sed -i 's/crossterm = "0.28"/crossterm = "0.29"/' "$REPO_ROOT/.tmp/ratatui-form/Cargo.toml"
    rm -rf "$REPO_ROOT/.tmp/ratatui-form/.git"
fi
