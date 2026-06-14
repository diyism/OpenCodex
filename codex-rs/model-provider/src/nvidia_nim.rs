use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use codex_api::ApiError;
use codex_api::Provider;
use codex_api::ReqwestTransport;
use codex_api::SharedAuthProvider;
use codex_api::TransportError;
use codex_api::map_api_error;
use codex_client::HttpTransport;
use codex_login::AuthManager;
use codex_login::CodexAuth;
use codex_login::default_client::build_reqwest_client;
use codex_model_provider_info::ModelProviderInfo;
use codex_models_manager::manager::ModelsEndpointClient;
use codex_models_manager::manager::OpenAiModelsManager;
use codex_models_manager::manager::SharedModelsManager;
use codex_models_manager::manager::StaticModelsManager;
use codex_models_manager::model_info::BASE_INSTRUCTIONS;
use codex_protocol::account::ProviderAccount;
use codex_protocol::config_types::ReasoningSummary;
use codex_protocol::error::CodexErr;
use codex_protocol::error::Result;
use codex_protocol::openai_models::ConfigShellToolType;
use codex_protocol::openai_models::InputModality;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::openai_models::ModelVisibility;
use codex_protocol::openai_models::ModelsResponse;
use codex_protocol::openai_models::TruncationPolicyConfig;
use codex_protocol::openai_models::WebSearchToolType;
use http::Method;
use http::header::ETAG;
use serde::Deserialize;
use tokio::time::timeout;

use crate::auth::auth_manager_for_provider;
use crate::auth::resolve_provider_auth;
use crate::provider::ModelProvider;
use crate::provider::ProviderAccountResult;
use crate::provider::ProviderAccountState;
use crate::provider::ProviderCapabilities;

const MODELS_REFRESH_TIMEOUT: Duration = Duration::from_secs(5);
const NVIDIA_NIM_MODELS_CACHE_FILE: &str = "nvidia_nim_models_cache.json";
const NVIDIA_NIM_DEFAULT_CONTEXT_WINDOW: i64 = 128_000;
const NVIDIA_NIM_TOOL_OUTPUT_TOKEN_LIMIT: i64 = 10_000;
const NVIDIA_NIM_BASE_INSTRUCTIONS_APPENDIX: &str = r#"

Performance note for local shell work: prefer fast search/index tools such as `rg`,
`rg --files`, and `git ls-files` before broad recursive PowerShell scans. Narrow
searches to the current project or workspace path, and avoid scanning a drive root
unless the user specifically asks for that scope."#;

/// Runtime provider for NVIDIA NIM OpenAI-compatible endpoints.
#[derive(Clone, Debug)]
pub(crate) struct NvidiaNimModelProvider {
    info: ModelProviderInfo,
    auth_manager: Option<Arc<AuthManager>>,
}

impl NvidiaNimModelProvider {
    pub(crate) fn new(
        provider_info: ModelProviderInfo,
        _auth_manager: Option<Arc<AuthManager>>,
    ) -> Self {
        let auth_manager = auth_manager_for_provider(/*auth_manager*/ None, &provider_info);
        Self {
            info: provider_info,
            auth_manager,
        }
    }
}

#[async_trait::async_trait]
impl ModelProvider for NvidiaNimModelProvider {
    fn info(&self) -> &ModelProviderInfo {
        &self.info
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            namespace_tools: false,
            image_generation: false,
            web_search: false,
        }
    }

    fn auth_manager(&self) -> Option<Arc<AuthManager>> {
        self.auth_manager.clone()
    }

    async fn auth(&self) -> Option<CodexAuth> {
        match self.auth_manager.as_ref() {
            Some(auth_manager) => auth_manager.auth().await,
            None => None,
        }
    }

    fn account_state(&self) -> ProviderAccountResult {
        let account = self
            .info
            .env_key
            .as_deref()
            .and_then(|env_key| std::env::var(env_key).ok())
            .filter(|api_key| !api_key.trim().is_empty())
            .map(|_| ProviderAccount::ApiKey);

        Ok(ProviderAccountState {
            account,
            requires_openai_auth: false,
        })
    }

    async fn api_provider(&self) -> Result<Provider> {
        let auth = self.auth().await;
        self.info()
            .to_api_provider(auth.as_ref().map(CodexAuth::auth_mode))
    }

    async fn api_auth(&self) -> Result<SharedAuthProvider> {
        let auth = self.auth().await;
        resolve_provider_auth(auth.as_ref(), self.info())
    }

    fn models_manager(
        &self,
        codex_home: PathBuf,
        config_model_catalog: Option<ModelsResponse>,
    ) -> SharedModelsManager {
        match config_model_catalog {
            Some(model_catalog) => Arc::new(StaticModelsManager::new(
                self.auth_manager.clone(),
                model_catalog,
            )),
            None => {
                let endpoint = Arc::new(NvidiaNimModelsEndpoint::new(
                    self.info.clone(),
                    self.auth_manager.clone(),
                ));
                Arc::new(OpenAiModelsManager::new_with_base_catalog(
                    codex_home,
                    endpoint,
                    self.auth_manager.clone(),
                    ModelsResponse::default(),
                    NVIDIA_NIM_MODELS_CACHE_FILE,
                    /*use_remote_models_only*/ true,
                ))
            }
        }
    }
}

#[derive(Debug)]
struct NvidiaNimModelsEndpoint {
    provider_info: ModelProviderInfo,
    auth_manager: Option<Arc<AuthManager>>,
}

impl NvidiaNimModelsEndpoint {
    fn new(provider_info: ModelProviderInfo, auth_manager: Option<Arc<AuthManager>>) -> Self {
        Self {
            provider_info,
            auth_manager,
        }
    }

    async fn auth(&self) -> Option<CodexAuth> {
        match self.auth_manager.as_ref() {
            Some(auth_manager) => auth_manager.auth().await,
            None => None,
        }
    }
}

#[async_trait::async_trait]
impl ModelsEndpointClient for NvidiaNimModelsEndpoint {
    fn has_command_auth(&self) -> bool {
        self.provider_info.has_command_auth()
    }

    fn supports_remote_model_catalog(&self) -> bool {
        true
    }

    async fn uses_codex_backend(&self) -> bool {
        false
    }

    async fn list_models(&self, _client_version: &str) -> Result<(Vec<ModelInfo>, Option<String>)> {
        let auth = self.auth().await;
        let auth_mode = auth.as_ref().map(CodexAuth::auth_mode);
        let api_provider = self.provider_info.to_api_provider(auth_mode)?;
        let api_auth = resolve_provider_auth(auth.as_ref(), &self.provider_info)?;

        let request = api_provider.build_request(Method::GET, "models");
        let request = api_auth
            .apply_auth(request)
            .await
            .map_err(TransportError::from)
            .map_err(ApiError::Transport)
            .map_err(map_api_error)?;

        let transport = ReqwestTransport::new(build_reqwest_client());
        let response = timeout(MODELS_REFRESH_TIMEOUT, transport.execute(request))
            .await
            .map_err(|_| CodexErr::Timeout)?
            .map_err(ApiError::Transport)
            .map_err(map_api_error)?;
        let etag = response
            .headers
            .get(ETAG)
            .and_then(|value| value.to_str().ok())
            .map(ToString::to_string);

        Ok((parse_nvidia_nim_models(&response.body)?, etag))
    }
}

#[derive(Debug, Deserialize)]
struct OpenAiModelListResponse {
    data: Vec<OpenAiModel>,
}

#[derive(Debug, Deserialize)]
struct OpenAiModel {
    id: String,
    owned_by: Option<String>,
}

fn parse_nvidia_nim_models(body: &[u8]) -> Result<Vec<ModelInfo>> {
    if let Ok(ModelsResponse { models }) = serde_json::from_slice::<ModelsResponse>(body) {
        return Ok(models);
    }

    let OpenAiModelListResponse { data } = serde_json::from_slice(body)?;
    let mut seen = HashSet::new();
    let mut models = Vec::new();
    for model in data {
        let id = model.id.trim();
        if id.is_empty() || !seen.insert(id.to_string()) {
            continue;
        }
        let priority = i32::try_from(models.len()).unwrap_or(i32::MAX);
        models.push(nvidia_nim_model_info(
            id,
            model.owned_by.as_deref(),
            priority,
        ));
    }
    Ok(models)
}

fn nvidia_nim_model_info(slug: &str, owned_by: Option<&str>, priority: i32) -> ModelInfo {
    let description = owned_by
        .filter(|owner| !owner.trim().is_empty())
        .map(|owner| format!("NVIDIA NIM model owned by {owner}"))
        .unwrap_or_else(|| "NVIDIA NIM model".to_string());

    ModelInfo {
        slug: slug.to_string(),
        display_name: slug.to_string(),
        description: Some(description),
        default_reasoning_level: None,
        supported_reasoning_levels: Vec::new(),
        shell_type: ConfigShellToolType::ShellCommand,
        visibility: ModelVisibility::List,
        supported_in_api: true,
        priority,
        additional_speed_tiers: Vec::new(),
        service_tiers: Vec::new(),
        availability_nux: None,
        upgrade: None,
        base_instructions: format!("{BASE_INSTRUCTIONS}{NVIDIA_NIM_BASE_INSTRUCTIONS_APPENDIX}"),
        model_messages: None,
        supports_reasoning_summaries: false,
        default_reasoning_summary: ReasoningSummary::Auto,
        support_verbosity: false,
        default_verbosity: None,
        apply_patch_tool_type: None,
        web_search_tool_type: WebSearchToolType::Text,
        truncation_policy: TruncationPolicyConfig::tokens(NVIDIA_NIM_TOOL_OUTPUT_TOKEN_LIMIT),
        supports_parallel_tool_calls: false,
        supports_image_detail_original: false,
        context_window: Some(NVIDIA_NIM_DEFAULT_CONTEXT_WINDOW),
        max_context_window: Some(NVIDIA_NIM_DEFAULT_CONTEXT_WINDOW),
        auto_compact_token_limit: None,
        effective_context_window_percent: 95,
        experimental_supported_tools: Vec::new(),
        input_modalities: vec![InputModality::Text],
        used_fallback_model_metadata: false,
        supports_search_tool: false,
    }
}

#[cfg(test)]
mod tests {
    use codex_models_manager::manager::RefreshStrategy;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use wiremock::Mock;
    use wiremock::MockServer;
    use wiremock::ResponseTemplate;
    use wiremock::matchers::method;
    use wiremock::matchers::path;

    use super::*;

    #[test]
    fn parses_standard_openai_models_response() {
        let body = br#"{
            "object": "list",
            "data": [
                {"id": "nvidia/llama-3.3-nemotron-super-49b-v1.5", "object": "model", "owned_by": "nvidia"},
                {"id": "openai/gpt-oss-120b", "object": "model", "owned_by": "openai"},
                {"id": "openai/gpt-oss-120b", "object": "model", "owned_by": "openai"}
            ]
        }"#;

        let models = parse_nvidia_nim_models(body).expect("models should parse");

        assert_eq!(models.len(), 2);
        assert_eq!(models[0].slug, "nvidia/llama-3.3-nemotron-super-49b-v1.5");
        assert_eq!(models[0].visibility, ModelVisibility::List);
        assert_eq!(models[0].shell_type, ConfigShellToolType::ShellCommand);
        assert!(!models[0].supports_parallel_tool_calls);
        assert_eq!(models[1].slug, "openai/gpt-oss-120b");
    }

    #[test]
    fn provider_capabilities_disable_openai_hosted_features() {
        let provider = NvidiaNimModelProvider::new(
            ModelProviderInfo::create_nvidia_nim_provider(),
            /*auth_manager*/ None,
        );

        assert_eq!(
            provider.capabilities(),
            ProviderCapabilities {
                namespace_tools: false,
                image_generation: false,
                web_search: false,
            }
        );
    }

    #[tokio::test]
    async fn models_manager_fetches_standard_openai_models_endpoint() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "object": "list",
                "data": [
                    {"id": "nvidia/nemotron-3-nano-30b-a3b", "object": "model", "owned_by": "nvidia"}
                ]
            })))
            .mount(&server)
            .await;

        let mut provider_info = ModelProviderInfo::create_nvidia_nim_provider();
        provider_info.base_url = Some(format!("{}/v1", server.uri()));
        provider_info.env_key = None;
        let provider = NvidiaNimModelProvider::new(provider_info, /*auth_manager*/ None);
        let manager = provider.models_manager(
            std::env::temp_dir().join(format!("codex-nvidia-nim-test-{}", std::process::id())),
            /*config_model_catalog*/ None,
        );

        let catalog = manager.raw_model_catalog(RefreshStrategy::Online).await;

        assert_eq!(catalog.models.len(), 1);
        assert_eq!(catalog.models[0].slug, "nvidia/nemotron-3-nano-30b-a3b");
    }
}
