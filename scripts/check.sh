#!/usr/bin/env bash
# Full local quality gate: format, lint, test, cross-platform compile check.
set -euo pipefail
cd "$(dirname "$0")/.."
. scripts/env.sh
echo "== fmt";    cargo fmt --all -- --check
echo "== clippy"; cargo clippy --workspace --all-targets -- -D warnings
echo "== test";   cargo test --workspace -q
# Crates without C code can be checked for other OSes from Windows.
# cs-db (bundled SQLite C source), cs-exec and the CLI need a target C compiler: checked on CI later.
for t in x86_64-unknown-linux-gnu x86_64-apple-darwin aarch64-apple-darwin; do
  echo "== check $t"
  cargo check -q --target "$t" --all-targets --features cs-core/pure     -p cs-platform -p cs-model -p cs-testgen -p cs-scan -p cs-meta -p cs-date -p cs-core -p cs-plan -p cs-image
done
echo "== audit (local RustSec DB; refresh with: cargo audit)"
cargo audit -q --no-fetch
echo "ALL CHECKS PASSED"
