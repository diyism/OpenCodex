# Release Notes

## Codex Open NVIDIA NIM Preview

This release introduces the first provider-focused Codex Open build. The main
goal is to let users run a local Codex CLI against NVIDIA NIM models while
keeping the local build separate from any global Codex installation.

### Added

- Built-in `nvidia-nim` model provider.
- NVIDIA NIM hosted endpoint default:
  `https://integrate.api.nvidia.com/v1`.
- Optional `CODEX_NVIDIA_NIM_BASE_URL` override for private or self-hosted NIM
  deployments.
- `NVIDIA_API_KEY` authentication.
- Remote model discovery from NIM-compatible `/v1/models`.
- Chat-completions inference through `/v1/chat/completions`.
- Streaming assistant output for NIM chat models.
- Thinking display when a model streams `delta.reasoning_content`.
- NIM-specific performance guidance that encourages faster workspace search
  commands such as `rg`, `rg --files`, and `git ls-files`.

### Fixed

- NVIDIA NIM no longer routes to `/v1/responses`, which returns `404 Not Found`
  for hosted NIM chat models.
- Provider selection is explicit: use `model_provider="nvidia-nim"` and the full
  model slug in `model`, for example `model="z-ai/glm-5.1"`.

### Known Limits

- Provider-specific model controls such as custom thinking budgets are not yet
  exposed as first-class Codex config.
- Model availability depends on the NVIDIA account, region, and endpoint.
- Tool behavior depends on each NIM model's OpenAI-compatible tool-call support.
- Image generation, web search, and OpenAI-hosted provider features are disabled
  for `nvidia-nim`.

### Upgrade Notes

Use a local `CODEX_HOME` for this build if you do not want to touch global Codex
state:

```powershell
$env:CODEX_HOME="D:\path\to\codex\.codex-local"
```

Build and run the local binary:

```powershell
cd codex-rs
$env:CARGO_HOME="$(Resolve-Path ..)\.cargo-local"
$env:CARGO_TARGET_DIR="$(Resolve-Path .)\target-local"
cargo build -j 1 -p codex-cli

$env:NVIDIA_API_KEY="nvapi-your-key"
& ".\target-local\debug\codex.exe" `
  -c 'model_provider="nvidia-nim"' `
  -c 'model="z-ai/glm-5.1"'
```

### Requesting Future Work

Additional providers and model-specific controls will be prioritized from real
requests. Open a Provider or Model Request issue with official docs, model ids,
auth details, streaming format, tool support, and reasoning/thinking fields.
