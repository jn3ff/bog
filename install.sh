#!/usr/bin/env bash
set -e
cargo build --release
cargo install --path . --force

echo ""
echo "Installed: bog"
echo "Run 'bog --help' to get started."
echo "Run 'bog orchestrate --help' for multi-agent orchestration."
