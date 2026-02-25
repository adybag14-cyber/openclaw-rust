# openclaw-agent-rs v1.7.3

## Highlights

- Fixed Telegram fallback model switching so provider/model overrides are applied atomically as `provider/model` and restored cleanly after fallback attempts.
- Fixed `/model` session override persistence by patching only the canonical `model` field and reading back from `session.status`.
- Expanded `/model list` visibility to include built-in + configured catalog entries (instead of only configured overrides).
- Added provider alias normalization for `zaiweb`/`zai-web` -> `zai` in gateway and Telegram paths.
- Added built-in Groq catalog models:
  - `groq/llama-3.3-70b-versatile`
  - `groq/llama-3.1-8b-instant`
- Hardened OAuth provider normalization to reject unsupported providers and added `google` alias for Gemini OAuth (`google-gemini-cli`).
- Enabled keyless bridge default for Inception/Mercury provider runtime (`allow_missing_api_key = true`).

## Validation

- `cargo +1.83.0-x86_64-pc-windows-gnu fmt --all -- --check`
- `cargo +1.83.0-x86_64-pc-windows-gnu clippy --all-targets -- -D warnings`
- `cargo +1.83.0-x86_64-pc-windows-gnu test`
- `cargo +1.83.0-x86_64-pc-windows-gnu build --release`
- `./scripts/with-mingw-env.ps1 "cargo +1.83.0-x86_64-pc-windows-gnu test --features sqlite-state"`
- Docker parity smoke: `./scripts/run-docker-parity-smoke.ps1`
- Ubuntu 20.04 (WSL): `cargo +1.83.0 build --release`
