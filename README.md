    $ wget https://github.com/diyism/OpenCodex/releases/download/linux-v0.0.0-20260614/codex.tar.gz
    $ tar xvf codex.tar.gz
    $ NVIDIA_API_KEY="nvapi-..." ./codex -c 'model_provider="nvidia-nim"' -c 'model="z-ai/glm-5.1"'
    $ NVIDIA_API_KEY="nvapi-..." ./codex -c 'model_provider="nvidia-nim"' -c 'model="deepseek-ai/deepseek-v4-pro"'
    $ NVIDIA_API_KEY="nvapi-..." ./codex -c 'model_provider="nvidia-nim"' -c 'model="moonshotai/kimi-k2.6,"'
    $ NVIDIA_API_KEY="nvapi-..." ./codex -c 'model_provider="nvidia-nim"' -c 'model="minimaxai/minimax-m3"'
    $ NVIDIA_API_KEY="nvapi-..." ./codex -c 'model_provider="nvidia-nim"' -c 'model="google/diffusiongemma-26b-a4b-it"'

============================================================

# OpenCodex

OpenCodex is a local-first, community-driven build of the Codex CLI focused on
open model provider support. It keeps the familiar terminal coding-agent
workflow while adding provider integrations that can be used without changing a
global Codex installation.

The first release feature is native NVIDIA NIM support. More providers and
quality-of-life features will be added from real community requests.

## Community

OpenCodex is meant to be practical and community-shaped. If something breaks,
open an issue. If a provider, model, or workflow is missing, open a feature
request. If the request is clear, useful, and backed by official docs where
needed, it can be added.

Bring the details that make the work actionable:

- What you tried.
- What failed or what you want added.
- Provider/model docs when the request involves an external API.
- Exact model ids, endpoints, and error messages when available.

## Highlights

- Local Codex CLI build that can run side-by-side with a global Codex install.
- Built-in `nvidia-nim` model provider.
- NVIDIA NIM model discovery through the `/v1/models` endpoint.
- NVIDIA NIM chat-completions support through `/v1/chat/completions`.
- Streaming assistant output for NIM chat models.
- Thinking/reasoning display when a NIM model streams `reasoning_content`.
- Provider-specific shell guidance to prefer fast workspace search tools such as
  `rg`, `rg --files`, and `git ls-files`.

Future provider and feature integrations will be driven through GitHub issues.
Open a provider request when you want another model host or API surface added.

## Quick Start

Build the local binary from the Rust workspace:

```powershell
cd codex-rs
$env:CARGO_HOME="$(Resolve-Path ..)\.cargo-local"
$env:CARGO_TARGET_DIR="$(Resolve-Path .)\target-local"
cargo build -j 1 -p codex-cli
```

Run the local binary without touching your global Codex data:

```powershell
$env:CODEX_HOME="$(Resolve-Path ..)\.codex-local"
$env:NVIDIA_API_KEY="nvapi-your-key"

& ".\target-local\debug\codex.exe" `
  -c 'model_provider="nvidia-nim"' `
  -c 'model="z-ai/glm-5.1"'
```

On macOS or Linux, use the same config values with your local binary path:

```bash
cd codex-rs
export CODEX_HOME="$(cd .. && pwd)/.codex-local"
export NVIDIA_API_KEY="nvapi-your-key"

./target-local/debug/codex \
  -c 'model_provider="nvidia-nim"' \
  -c 'model="z-ai/glm-5.1"'
```

`model_provider` must be `nvidia-nim`. The model id is the full NIM model slug,
for example `z-ai/glm-5.1`.

## NVIDIA NIM

OpenCodex uses NVIDIA NIM's OpenAI-compatible API shape:

- Model catalog: `GET /v1/models`
- Chat inference: `POST /v1/chat/completions`
- Authentication: `Authorization: Bearer <NVIDIA_API_KEY>`

Default hosted endpoint:

```text
https://integrate.api.nvidia.com/v1
```

Override the endpoint for a self-hosted or private NIM deployment:

```powershell
$env:CODEX_NVIDIA_NIM_BASE_URL="https://your-nim-host.example.com/v1"
```

Detailed setup and troubleshooting are in
[`docs/nvidia-nim.md`](docs/nvidia-nim.md).

## Release Status

This release is provider-focused. NVIDIA NIM is the first supported external
provider. Additional providers, model controls, and provider-specific features
will be added based on community requests and tested integrations.

See [`RELEASE_NOTES.md`](RELEASE_NOTES.md) for the current release notes.

## Request a Feature

Use GitHub issues for requests:

- Provider/model request: use the Provider or Model Request issue template.
- Bug report: include `codex --version`, platform, provider, model id, and the
  exact command you ran.
- Feature request: describe the workflow, expected behavior, and links to any
  official API documentation.

Useful details for provider requests:

- Provider name and official API docs.
- Base URL and authentication method.
- Whether the provider supports chat completions, responses, tools, streaming,
  and reasoning/thinking fields.
- Example model ids you want supported.

## Development

Common local commands:

```powershell
cd codex-rs
$env:CARGO_HOME="$(Resolve-Path ..)\.cargo-local"
$env:CARGO_TARGET_DIR="$(Resolve-Path .)\target-local"

cargo fmt --package codex-core --package codex-model-provider
cargo check -j 1 -p codex-core -p codex-model-provider
cargo test -j 1 -p codex-model-provider
cargo build -j 1 -p codex-cli
```

Use `-j 1` on machines where parallel Rust builds exhaust memory or pagefile
space.

## License

This repository is licensed under the [Apache-2.0 License](LICENSE). Codex is an
open source project originally released by OpenAI; OpenCodex adds community
provider integration work on top of that codebase.
