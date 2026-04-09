# LLM Module

Current backends are limited to:

- `openai` with `request_format = "chat_completions"` or `"responses"`
- `openai_codex`
- `anthropic`
- `groq`
- `openrouter`
- `ollama`

## File Map

| File | Role |
|------|------|
| `mod.rs` | Provider factory and provider-chain assembly |
| `config.rs` | Resolved LLM config types |
| `error.rs` | Shared `LlmError` type |
| `provider.rs` | `LlmProvider` trait and shared request/response types |
| `disabled.rs` | Placeholder provider used when no backend is configured |
| `oauth_helpers.rs` | Shared OAuth callback listener helpers |
| `openai_codex_provider.rs` | Dedicated OpenAI Codex provider |
| `openai_codex_session.rs` | OpenAI Codex auth/session management |
| `anthropic_oauth.rs` | Anthropic OAuth-backed provider variant |
| `rig_adapter.rs` | Adapter for registry-backed providers |
| `retry.rs` | Retry wrapper |
| `smart_routing.rs` | Cheap/primary routing wrapper |
| `recording.rs` | Request recording wrapper |
| `response_cache.rs` | Cached provider wrapper |

## Notes

- Desktop startup must tolerate an unconfigured backend. `DisabledLlmProvider` is used until onboarding completes.
- Registry-backed providers come only from `providers.json`.
- OpenAI Codex is intentionally separate from the registry-backed providers.
