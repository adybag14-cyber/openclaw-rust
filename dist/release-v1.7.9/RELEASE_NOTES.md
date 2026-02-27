# OpenClaw Rust v1.7.9

## Highlights
- Added ChatGPT browser-session OAuth capture flow for OpenAI provider usage without API keys.
- Added `scripts/chatgpt-browser-auth.mjs` (Playwright-first with Puppeteer fallback).
- Added `/auth wait openai` browser capture + OAuth completion path.
- Added OpenAI OAuth runtime override path to route keyless authenticated sessions through ChatGPT website bridge.
- Added official ChatGPT website bridge invocation path (`chatgpt.com/backend-api/conversation`) with SSE-to-OpenAI response shaping.
- Added model catalog entry for `gpt-5.2-thinking-extended`.

## Validation
- `cargo +1.83.0-x86_64-pc-windows-gnu test`
- `cargo +1.83.0-x86_64-pc-windows-gnu clippy --all-targets -- -D warnings`
- `cargo +1.83.0-x86_64-pc-windows-gnu build --release`
- `./scripts/with-mingw-env.ps1 "cargo +1.83.0-x86_64-pc-windows-gnu test --features sqlite-state"`
- `./scripts/with-mingw-env.ps1 "cargo +1.83.0-x86_64-pc-windows-gnu clippy --all-targets --features sqlite-state -- -D warnings"`
- `./scripts/with-mingw-env.ps1 "cargo +1.83.0-x86_64-pc-windows-gnu build --release --features sqlite-state"`
- `wsl -d Ubuntu-20.04 -- bash -lc 'source $HOME/.cargo/env && cd /mnt/c/Users/Ady/Documents/openclaw-rust && CARGO_TARGET_DIR=target-linux-ubuntu20 cargo +1.83.0 build --release'`
- `node --check scripts/chatgpt-browser-auth.mjs`
