# openclaw-agent-rs v1.7.4 (edge)

## Highlights

- Hardened official website bridge routing for key providers:
  - ZAI/GLM: added GLM alias-to-candidate fallback set (`glm-5`, `glm-5-air`, `glm-4.5`, `glm-4.5-air`) so bridge retries stay on coherent ZAI models.
  - Qwen 3.5: improved model alias handling (including provider-prefixed IDs) while preserving 3.5 variant fallback chain.
  - Inception/Mercury: added a dedicated guest bridge flow (`auth -> chat create -> completions`) for `mercury-2` and aliases.
- Added coherent reply gating for bridge responses: a 200 response must contain usable assistant output (text or tool calls), otherwise candidate fallback continues.
- Updated runtime defaults for guest bridge operation when API keys are absent:
  - `zai`, `zhipuai`, `zhipuai-coding`, and `inception` now allow keyless bridge fallback.
  - ZAI bridge candidates are pre-seeded with `https://chat.z.ai`.
- Expanded built-in model catalog with direct ZAI entries for GLM routing:
  - `zai/glm-5`
  - `zai/glm-4.5-air`
- Updated provider matrix documentation to reflect bridge coverage and keyless fallback behavior.

## Validation

- `cargo +1.83.0-x86_64-pc-windows-gnu fmt --all -- --check`
- `cargo +1.83.0-x86_64-pc-windows-gnu clippy --all-targets -- -D warnings`
- `cargo +1.83.0-x86_64-pc-windows-gnu test`
- `cargo +1.83.0-x86_64-pc-windows-gnu build --release`
- `./scripts/with-mingw-env.ps1 "cargo +1.83.0-x86_64-pc-windows-gnu test --features sqlite-state"`
- Docker parity smoke: `./scripts/run-docker-parity-smoke.ps1`
- Ubuntu 20.04 (WSL): `cargo +1.83.0 build --release`
