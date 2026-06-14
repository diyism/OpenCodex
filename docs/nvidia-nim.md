# NVIDIA NIM Provider

Codex Open includes a native `nvidia-nim` provider for NVIDIA NIM models that
expose an OpenAI-compatible chat-completions API.

## What Works

- API-key authentication with `NVIDIA_API_KEY`.
- Hosted NVIDIA endpoint at `https://integrate.api.nvidia.com/v1`.
- Optional custom endpoint through `CODEX_NVIDIA_NIM_BASE_URL`.
- Remote model catalog loading from `/v1/models`.
- Streaming chat completions from `/v1/chat/completions`.
- Tool calls through OpenAI-compatible chat `tools`.
- Thinking display when a model streams `delta.reasoning_content`.

## Requirements

- A built local Codex Open binary.
- An NVIDIA API key with access to the model you want to use.
- Network access to NVIDIA hosted NIM or your private NIM endpoint.

## Build Locally

From the repository root:

```powershell
cd codex-rs
$env:CARGO_HOME="$(Resolve-Path ..)\.cargo-local"
$env:CARGO_TARGET_DIR="$(Resolve-Path .)\target-local"
cargo build -j 1 -p codex-cli
```

The local Windows binary will be:

```text
codex-rs\target-local\debug\codex.exe
```

Using a local target directory keeps this build separate from any global Codex
installation.

## Run With NVIDIA NIM

PowerShell:

```powershell
cd D:\path\to\codex\codex-rs

$env:CODEX_HOME="D:\path\to\codex\.codex-local"
$env:NVIDIA_API_KEY="nvapi-your-key"

& ".\target-local\debug\codex.exe" `
  -c 'model_provider="nvidia-nim"' `
  -c 'model="z-ai/glm-5.1"'
```

Bash:

```bash
cd /path/to/codex/codex-rs

export CODEX_HOME="/path/to/codex/.codex-local"
export NVIDIA_API_KEY="nvapi-your-key"

./target-local/debug/codex \
  -c 'model_provider="nvidia-nim"' \
  -c 'model="z-ai/glm-5.1"'
```

Important: `model_provider` is always `nvidia-nim`. Do not set it to the model
owner name. For example, use:

```text
model_provider = "nvidia-nim"
model = "z-ai/glm-5.1"
```

## List Available NIM Models

After setting `NVIDIA_API_KEY`, ask Codex to load the provider's model catalog:

```powershell
& ".\target-local\debug\codex.exe" debug models `
  -c 'model_provider="nvidia-nim"'
```

Filter for a vendor or family:

```powershell
& ".\target-local\debug\codex.exe" debug models `
  -c 'model_provider="nvidia-nim"' | Select-String "glm|z-ai|nemotron|deepseek"
```

Availability depends on what NVIDIA exposes for your API key and region.

## Self-Hosted or Private NIM

Set `CODEX_NVIDIA_NIM_BASE_URL` to the `/v1` base URL:

```powershell
$env:CODEX_NVIDIA_NIM_BASE_URL="https://your-nim-host.example.com/v1"
```

Then run Codex with the same provider configuration:

```powershell
& ".\target-local\debug\codex.exe" `
  -c 'model_provider="nvidia-nim"' `
  -c 'model="your-org/your-model"'
```

The endpoint must support:

- `GET /v1/models`
- `POST /v1/chat/completions`
- Bearer-token authentication or a compatible auth layer

## Thinking Display

Some NIM-hosted models stream reasoning/thinking content with a field like:

```json
{
  "choices": [
    {
      "delta": {
        "reasoning_content": "..."
      }
    }
  ]
}
```

Codex Open maps that field into Codex's existing Thinking display. If the model
does not stream reasoning content, no thinking box is shown.

## Speed Tips

Provider latency depends on the selected model, current load, network path, and
how much thinking the model performs. For faster local workflows:

- Start Codex in the project directory instead of a broad workspace root.
- Prefer `rg`, `rg --files`, and `git ls-files` over `Get-ChildItem -Recurse`
  for large directory searches.
- Use a smaller or lower-latency NIM model when the task does not need extended
  reasoning.
- Keep `CODEX_HOME` local during testing so a broken global Codex database does
  not block the local build.
- Use `cargo build -j 1` on Windows machines where parallel builds exhaust
  memory or pagefile space.

Example targeted launch:

```powershell
& "D:\path\to\codex\codex-rs\target-local\debug\codex.exe" `
  -C "D:\Whitebox\Federated-Learning-with-IPFS" `
  -c 'model_provider="nvidia-nim"' `
  -c 'model="z-ai/glm-5.1"'
```

## Troubleshooting

### 404 on `/v1/responses`

NVIDIA NIM chat models use chat completions. This build routes `nvidia-nim` to:

```text
POST /v1/chat/completions
```

If you see a `/v1/responses` request, rebuild the local binary and confirm you
are running the local executable.

### `Model provider 'z-ai' not found`

`z-ai` is part of the model slug, not the provider name. Use:

```powershell
-c 'model_provider="nvidia-nim"' -c 'model="z-ai/glm-5.1"'
```

### Damaged Global Codex Database

Set `CODEX_HOME` to a local folder for this build:

```powershell
$env:CODEX_HOME="D:\path\to\codex\.codex-local"
```

This avoids reading or modifying the global Codex state under your user profile.

### Slow Directory Search

Avoid recursive scans over large roots such as `D:\Whitebox` unless that scope
is required. Use a targeted project path or a faster search command:

```powershell
rg --files D:\Whitebox | rg "Federated-Learning"
```

## Requesting More Models or Providers

Open a Provider or Model Request issue and include:

- Provider name and official docs.
- Desired model ids.
- Base URL and auth method.
- Streaming format.
- Tool-call support.
- Reasoning/thinking field names, if any.
- A minimal curl or SDK example from official docs.
