# Provider Support Matrix (Rust Runtime)

Last audited: 2026-02-24

This matrix is generated from the runtime implementation, not marketing labels:

- `src/gateway.rs`: `normalize_provider_id`, `provider_runtime_defaults`, `provider_runtime_bridge_defaults`, `OAUTH_PROVIDER_CATALOG`
- `src/website_bridge.rs`: `should_use_zai_guest_bridge` and bridge request path selection

Status legend:

- `Built-in default`: runtime has a canonical default `base_url` + API mode and env-var auth lookup.
- `Alias + config`: alias is normalized, but you must set endpoint/auth in config (`models.providers.<id>`).
- `Bridge default`: runtime seeds website bridge hints (`websiteUrl` and/or `bridgeBaseUrls`).
- `OAuth catalog`: appears in `auth.oauth.providers/start/wait/complete/logout/import`.

Runtime security note (`v1.6.2`):

- Provider routing/output now flows through the expanded tool runtime safety stack (`security.wasm`, dynamic WIT registry, credential leak redaction, and SafetyLayer output controls).

## Requested Coverage Matrix

| Provider | Runtime status | Canonical id |
|---|---|---|
| Ollama | Built-in default | `ollama` |
| LM Studio | Built-in default | `lmstudio` |
| vLLM | Built-in default | `vllm` |
| llama.cpp server | Built-in default | `llamacpp` |
| LocalAI | Built-in default | `localai` |
| Hugging Face TGI | Built-in default | `tgi` |
| GPT4All | Built-in default | `gpt4all` |
| KoboldCPP | Built-in default | `koboldcpp` |
| Oobabooga | Built-in default | `oobabooga` |
| Groq | Built-in default | `groq` |
| Google Gemini (AI Studio OpenAI-compat) | Built-in default | `google` |
| DeepSeek | Built-in default | `deepseek` |
| Mistral | Built-in default | `mistral` |
| Fireworks | Built-in default | `fireworks` |
| Together AI | Built-in default | `together` |
| DeepInfra | Built-in default | `deepinfra` |
| Cerebras | Built-in default | `cerebras` |
| SiliconFlow | Built-in default | `siliconflow` |
| SambaNova | Built-in default | `sambanova` |
| Novita | Built-in default | `novita` |
| Hyperbolic | Built-in default | `hyperbolic` |
| Nebius | Built-in default | `nebius` |
| Inference.net | Built-in default | `inference-net` |
| OpenRouter | Built-in default | `openrouter` |
| Inception Mercury | Built-in default | `inception` |
| AIMLAPI | Built-in default | `aimlapi` |
| Vercel AI Gateway | Alias + config | `vercel-ai-gateway` |
| ShareAI | Alias + config | `shareai` |
| Bifrost (Maxim) | Alias + config | `bifrost` |
| OpenAI | Built-in default | `openai` |
| Anthropic (Claude) | Alias + config, OAuth catalog | `anthropic` |
| Azure OpenAI | Alias + config | `azure-openai` |
| Google Vertex AI | Alias + config | `vertex-ai` |
| Amazon Bedrock | Alias + config | `bedrock` |
| Cohere | Built-in default (compat endpoint) | `cohere` |
| xAI Grok | Built-in default | `xai` |
| NVIDIA NIM | Built-in default | `nvidia` |
| GitHub Models | Alias + config | `github-models` |
| OpenCode Zen | Built-in default + bridge default | `opencode` |
| OpenRouter free models | Built-in default + model id support | `openrouter` |

Notes:

- `zai` / `zhipuai` guest website bridge is implemented (`chat.z.ai` path support).
- `qwen-portal` guest website bridge fallback is implemented (`chat.qwen.ai` path support).
- `kimi-coding` has a built-in API endpoint default and OAuth catalog support, but guest bridge is login/session-gated in practice.

## Exhaustive Built-in Runtime Defaults (Canonical IDs)

All entries below resolve to `api_mode = openai-completions` in runtime.

| Canonical provider | Default base URL | API key optional |
|---|---|---|
| `aimlapi` | `https://api.aimlapi.com/v1` | no |
| `byteplus` | `https://api.byteplus.com/v1` | no |
| `cerebras` | `https://api.cerebras.ai/v1` | no |
| `cohere` | `https://api.cohere.com/compatibility/v1` | no |
| `deepinfra` | `https://api.deepinfra.com/v1/openai` | no |
| `deepseek` | `https://api.deepseek.com/v1` | no |
| `fireworks` | `https://api.fireworks.ai/inference/v1` | no |
| `google` | `https://generativelanguage.googleapis.com/v1beta/openai` | no |
| `gpt4all` | `http://127.0.0.1:4891/v1` | yes |
| `groq` | `https://api.groq.com/openai/v1` | no |
| `huggingface` | `https://api-inference.huggingface.co/v1` | no |
| `hyperbolic` | `https://api.hyperbolic.xyz/v1` | no |
| `inference-net` | `https://api.inference.net/v1` | no |
| `inception` | `https://api.inceptionlabs.ai/v1` | no |
| `kimi-coding` | `https://api.kimi.com/coding` | no |
| `koboldcpp` | `http://127.0.0.1:5001/v1` | yes |
| `litellm` | `http://127.0.0.1:4000/v1` | yes |
| `llamacpp` | `http://127.0.0.1:8080/v1` | yes |
| `lmstudio` | `http://127.0.0.1:1234/v1` | yes |
| `localai` | `http://127.0.0.1:8080/v1` | yes |
| `mistral` | `https://api.mistral.ai/v1` | no |
| `moonshot` | `https://api.moonshot.ai/v1` | no |
| `nebius` | `https://api.studio.nebius.com/v1` | no |
| `novita` | `https://api.novita.ai/v3/openai` | no |
| `nvidia` | `https://integrate.api.nvidia.com/v1` | no |
| `ollama` | `http://127.0.0.1:11434/v1` | yes |
| `oobabooga` | `http://127.0.0.1:5000/v1` | yes |
| `openai` | `https://api.openai.com/v1` | no |
| `openai-codex` | `https://api.openai.com/v1` | no |
| `opencode` | `https://opencode.ai/zen/v1` | yes |
| `openrouter` | `https://openrouter.ai/api/v1` | no |
| `perplexity` | `https://api.perplexity.ai` | no |
| `qianfan` | `https://qianfan.baidubce.com/v2` | no |
| `qwen-portal` | `https://portal.qwen.ai/v1` | yes (bridge fallback) |
| `sambanova` | `https://api.sambanova.ai/v1` | no |
| `siliconflow` | `https://api.siliconflow.cn/v1` | no |
| `tgi` | `http://127.0.0.1:8080/v1` | yes |
| `together` | `https://api.together.xyz/v1` | no |
| `vllm` | `http://127.0.0.1:8000/v1` | yes |
| `volcengine` | `https://ark.cn-beijing.volces.com/api/v3` | no |
| `xai` | `https://api.x.ai/v1` | no |
| `zai` | `https://api.z.ai/v1` | no |
| `zhipuai` | `https://open.bigmodel.cn/api/paas/v4` | no |
| `zhipuai-coding` | `https://open.bigmodel.cn/api/coding/paas/v4` | no |

## Alias + Config Required Providers

These canonical IDs are normalized from aliases, but do not have built-in default base URLs in `provider_runtime_defaults`.

| Canonical provider | Example aliases |
|---|---|
| `anthropic` | `claude`, `claude-code`, `claude-cli` |
| `azure-openai` | `azure`, `azure-openai-service` |
| `vertex-ai` | `vertex`, `google-vertex` |
| `bedrock` | `amazon-bedrock` |
| `github-models` | `github-model` |
| `vercel-ai-gateway` | `vercel-ai`, `ai-gateway` |
| `shareai` | `share-ai` |
| `bifrost` | `maxim-bifrost`, `bifrost-maxim` |

## Website Bridge Defaults

| Canonical provider | Default websiteUrl | Default bridge candidates |
|---|---|---|
| `opencode` | `https://opencode.ai` | `https://opencode.ai/zen/v1`, `https://api.opencode.ai/v1` |
| `qwen-portal` | `https://chat.qwen.ai` | `https://chat.qwen.ai` |
| `zai` | `https://chat.z.ai` | none pre-seeded |
| `zhipuai` | `https://chat.z.ai` | none pre-seeded |
| `zhipuai-coding` | `https://chat.z.ai` | none pre-seeded |
| `kimi-coding` | `https://www.kimi.com` | none pre-seeded |
| `minimax-portal` | `https://chat.minimax.io` | none pre-seeded |
| `inception` | `https://chat.inceptionlabs.ai` | `https://api.inceptionlabs.ai/v1` |

Implementation detail:

- Zhipu/Z.ai and Qwen guest bridge fallbacks are explicitly recognized in `src/website_bridge.rs`.
- Kimi/Minimax website hints are surfaced for configured bridge mode, but practical usage is login/session dependent.

## OAuth Provider Catalog (RPC Surface)

`auth.oauth.providers` currently advertises these provider IDs:

- `openai`
- `openai-codex`
- `anthropic`
- `google-gemini-cli`
- `qwen-portal`
- `minimax-portal`
- `kimi-coding`
- `opencode`
- `zhipuai`

## Endpoint Documentation References (Audited)

The following official documentation sets were reviewed in parallel for endpoint/auth alignment:

- OpenAI: https://platform.openai.com/docs/api-reference/chat/create-chat-completion
- Anthropic: https://docs.anthropic.com/en/api/overview
- Azure OpenAI: https://learn.microsoft.com/azure/ai-foundry/openai/latest
- Vertex AI OpenAI compatibility: https://cloud.google.com/vertex-ai/generative-ai/docs/start/openai
- Cohere compatibility API: https://docs.cohere.com/docs/compatibility-api
- OpenRouter: https://openrouter.ai/docs
- Vercel AI Gateway OpenAI compatibility: https://vercel.com/docs/ai-gateway/sdks-and-apis/openai-compat
- Groq OpenAI compatibility: https://console.groq.com/docs/openai
- Google Gemini OpenAI compatibility: https://ai.google.dev/gemini-api/docs/openai
- DeepSeek: https://api-docs.deepseek.com/
- Fireworks OpenAI compatibility: https://docs.fireworks.ai/tools-sdks/openai-compatibility
- Together OpenAI compatibility: https://docs.together.ai/docs/openai-api-compatibility
- NVIDIA NIM API reference: https://docs.api.nvidia.com/nim/reference/llm-apis
- GitHub Models: https://docs.github.com/en/github-models/quickstart
- Ollama API: https://docs.ollama.com/api/introduction
- LM Studio OpenAI endpoints: https://lmstudio.ai/docs/app/api/endpoints/openai
- vLLM OpenAI-compatible server: https://docs.vllm.ai/en/latest/serving/openai_compatible_server/
- GPT4All local API server: https://docs.gpt4all.io/gpt4all_api_server/home.html
- OpenCode Zen docs: https://open-code.ai/docs/en/zen

For providers that are alias-only/config-required in this runtime, these docs should be used to supply `models.providers.<id>.baseUrl`, auth headers, and any provider-specific request defaults.

