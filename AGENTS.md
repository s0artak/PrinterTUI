# AGENTS.md — PrinterTUI

## Verification Gate (Required)
Run after any edit before reporting completion:
```bash
./scripts/verify.sh
```
Runs `cargo clippy --workspace -- -D warnings` and `cargo test --workspace`.
