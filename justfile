set positional-arguments

# default recipe to display help information
default:
  @just --list

# Fixes the formatting of the workspace
fmt-fix:
  cargo +nightly fmt --all

# Check the formatting of the workspace
fmt-check:
  cargo +nightly fmt --all -- --check

# Lint the workspace
lint: fmt-check
  cargo +nightly clippy --workspace --all --all-features --all-targets -- -D warnings

# Build the workspace
build *args='':
  cargo build --workspace --all $@

# Run Rust tests
test *args='':
  cargo nextest run --workspace --all --all-features $@

# Run Lua tests (requires nvim with plenary.nvim installed)
test-lua:
  nvim --headless -c "PlenaryBustedDirectory tests/ {minimal_init = 'tests/minimal_init.lua'}"
