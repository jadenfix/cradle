use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Deserializer, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Lane {
    Wasm,
    PythonWasi,
    PythonNative,
    JsWasm,
    JsNative,
    Exec,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BrowserSandboxLevel {
    InstrumentedExternal,
    EphemeralProfile,
    NetworkSuppressed,
    SealedState,
    OsIsolated,
    RemoteIsolated,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BrowserSandboxAvailability {
    Available,
    Planned,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BrowserSandboxControl {
    FreshProfile,
    NoAmbientCredentials,
    EgressPolicy,
    LocalNetworkBlock,
    SealedArtifacts,
    OsProcessIsolation,
    RemoteWorkerIsolation,
    TeardownProof,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct BrowserSandboxProfile {
    pub level: BrowserSandboxLevel,
    pub availability: BrowserSandboxAvailability,
    pub summary: String,
    #[serde(default)]
    #[schema(required = true)]
    pub controls: Vec<BrowserSandboxControl>,
    pub isolation_boundary: String,
    pub privacy_controls: Vec<String>,
    pub egress_controls: Vec<String>,
    pub credential_controls: Vec<String>,
    pub storage_controls: Vec<String>,
    pub encryption_claims: Vec<String>,
    pub non_goals: Vec<String>,
    pub downgrade_reasons: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct BrowserIntegrationContract {
    pub status: BrowserSandboxAvailability,
    pub consumer: String,
    pub endpoint: String,
    pub admission_endpoint: String,
    pub selection_field: String,
    pub required_consumer_behavior: Vec<String>,
    #[serde(default)]
    #[schema(required = true)]
    pub adapter: BrowserAdapterContract,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct BrowserAdapterContract {
    pub version: String,
    pub status: BrowserSandboxAvailability,
    #[schema(required = true)]
    pub launch_endpoint: Option<String>,
    pub handoff_fields: Vec<String>,
    pub required_guard_fields: Vec<String>,
    pub required_completion_proofs: Vec<String>,
    #[serde(default = "browser_adapter_completion_proof_contract")]
    #[schema(required = true)]
    pub completion_proof_contract: Vec<BrowserAdapterCompletionProofRequirement>,
    pub unavailable_reason: String,
}

impl Default for BrowserAdapterContract {
    fn default() -> Self {
        let completion_proof_contract = browser_adapter_completion_proof_contract();
        Self {
            version: "browser-adapter-v1".to_string(),
            status: BrowserSandboxAvailability::Unavailable,
            launch_endpoint: None,
            handoff_fields: vec![
                "request_id".to_string(),
                "issued_at".to_string(),
                "expires_at".to_string(),
                "max_session_seconds".to_string(),
                "adapter_id".to_string(),
                "contract_version".to_string(),
                "requested_level".to_string(),
                "actor".to_string(),
                "sensitivity".to_string(),
                "sensitive_activity_mode".to_string(),
                "target_origins".to_string(),
                "credential_mode".to_string(),
                "artifact_mode".to_string(),
                "requested_controls".to_string(),
                "guard_plan".to_string(),
                "required_completion_proofs".to_string(),
                "completion_proof_contract".to_string(),
                "completion_report_template".to_string(),
                "replay_protection_required".to_string(),
            ],
            required_guard_fields: vec![
                "guard_plan.network.allowed_origins".to_string(),
                "guard_plan.network.deny_private_networks".to_string(),
                "guard_plan.network.deny_localhost".to_string(),
                "guard_plan.network.deny_metadata_endpoints".to_string(),
                "guard_plan.network.require_dns_rebinding_protection".to_string(),
                "guard_plan.network.require_redirect_revalidation".to_string(),
                "guard_plan.network.require_proxy_enforcement".to_string(),
                "guard_plan.network.outbound_network_disabled_without_proxy".to_string(),
                "guard_plan.credentials.mode".to_string(),
                "guard_plan.credentials.ambient_credentials_allowed".to_string(),
                "guard_plan.credentials.user_mediation_required".to_string(),
                "guard_plan.credentials.scoped_secret_channel_required".to_string(),
                "guard_plan.storage.mode".to_string(),
                "guard_plan.storage.plaintext_persistence_allowed".to_string(),
                "guard_plan.storage.explicit_artifact_allowlist_required".to_string(),
                "guard_plan.storage.encryption_required_for_persistence".to_string(),
                "guard_plan.storage.teardown_proof_required".to_string(),
                "guard_plan.suppression.mode".to_string(),
                "guard_plan.suppression.suppress_ambient_browser_state".to_string(),
                "guard_plan.suppression.suppress_ambient_credentials".to_string(),
                "guard_plan.suppression.suppress_unapproved_network".to_string(),
                "guard_plan.suppression.suppress_persistent_artifacts".to_string(),
                "guard_plan.suppression.downgrade_requires_user_approval".to_string(),
                "guard_plan.suppression.required_operator_confirmations".to_string(),
                "guard_plan.required_runtime_guards".to_string(),
            ],
            required_completion_proofs: vec![
                "browser process exited or was killed".to_string(),
                "temporary profile directory removed".to_string(),
                "plaintext artifacts outside the explicit allowlist removed".to_string(),
                "egress proxy log sealed or discarded according to artifact_mode".to_string(),
            ],
            completion_proof_contract,
            unavailable_reason: "no browser adapter launch endpoint is implemented by this daemon"
                .to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct BrowserProfilesResponse {
    pub version: String,
    pub runnable_browser_sessions: bool,
    #[schema(required = true)]
    pub default_level: Option<BrowserSandboxLevel>,
    pub integration: BrowserIntegrationContract,
    pub profiles: Vec<BrowserSandboxProfile>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BrowserSessionActor {
    Agent,
    Human,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BrowserSensitivity {
    Public,
    Sensitive,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BrowserCredentialMode {
    #[default]
    NoCredentials,
    UserMediated,
    ScopedSecrets,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BrowserArtifactMode {
    #[default]
    Discard,
    ExplicitDownloads,
    SealedArtifacts,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BrowserSensitiveActivityMode {
    #[default]
    Standard,
    Private,
    NetworkSuppressed,
    Sealed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct BrowserNetworkGuardPlan {
    /// Validated public origins this browser session may target.
    pub allowed_origins: Vec<String>,
    /// Runtime must deny localhost, loopback, private, shared, link-local, and
    /// metadata-address egress even after DNS resolution, redirects, and proxying.
    pub deny_private_networks: bool,
    pub deny_localhost: bool,
    pub deny_metadata_endpoints: bool,
    pub require_dns_rebinding_protection: bool,
    pub require_redirect_revalidation: bool,
    pub require_proxy_enforcement: bool,
    pub outbound_network_disabled_without_proxy: bool,
}

impl Default for BrowserNetworkGuardPlan {
    fn default() -> Self {
        Self {
            allowed_origins: Vec::new(),
            deny_private_networks: true,
            deny_localhost: true,
            deny_metadata_endpoints: true,
            require_dns_rebinding_protection: true,
            require_redirect_revalidation: true,
            require_proxy_enforcement: true,
            outbound_network_disabled_without_proxy: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct BrowserCredentialGuardPlan {
    pub mode: BrowserCredentialMode,
    pub ambient_credentials_allowed: bool,
    pub user_mediation_required: bool,
    pub scoped_secret_channel_required: bool,
}

impl Default for BrowserCredentialGuardPlan {
    fn default() -> Self {
        Self {
            mode: BrowserCredentialMode::NoCredentials,
            ambient_credentials_allowed: false,
            user_mediation_required: false,
            scoped_secret_channel_required: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct BrowserStorageGuardPlan {
    pub mode: BrowserArtifactMode,
    pub plaintext_persistence_allowed: bool,
    pub explicit_artifact_allowlist_required: bool,
    pub encryption_required_for_persistence: bool,
    pub teardown_proof_required: bool,
}

impl Default for BrowserStorageGuardPlan {
    fn default() -> Self {
        Self {
            mode: BrowserArtifactMode::Discard,
            plaintext_persistence_allowed: false,
            explicit_artifact_allowlist_required: false,
            encryption_required_for_persistence: false,
            teardown_proof_required: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct BrowserSuppressionGuardPlan {
    pub mode: BrowserSensitiveActivityMode,
    pub suppress_ambient_browser_state: bool,
    pub suppress_ambient_credentials: bool,
    pub suppress_unapproved_network: bool,
    pub suppress_persistent_artifacts: bool,
    pub downgrade_requires_user_approval: bool,
    pub required_operator_confirmations: Vec<String>,
}

impl Default for BrowserSuppressionGuardPlan {
    fn default() -> Self {
        Self {
            mode: BrowserSensitiveActivityMode::Standard,
            suppress_ambient_browser_state: false,
            suppress_ambient_credentials: false,
            suppress_unapproved_network: false,
            suppress_persistent_artifacts: false,
            downgrade_requires_user_approval: true,
            required_operator_confirmations: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct BrowserAdmissionGuardPlan {
    pub network: BrowserNetworkGuardPlan,
    pub credentials: BrowserCredentialGuardPlan,
    pub storage: BrowserStorageGuardPlan,
    #[serde(default)]
    #[schema(required = true)]
    pub suppression: BrowserSuppressionGuardPlan,
    pub required_runtime_guards: Vec<String>,
}

impl Default for BrowserAdmissionGuardPlan {
    fn default() -> Self {
        Self {
            network: BrowserNetworkGuardPlan::default(),
            credentials: BrowserCredentialGuardPlan::default(),
            storage: BrowserStorageGuardPlan::default(),
            suppression: BrowserSuppressionGuardPlan::default(),
            required_runtime_guards: vec![
                "fresh server-issued guard plan required before browser launch".to_string(),
            ],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct BrowserAdapterCompletionProofRequirement {
    /// Stable machine-readable proof name. Adapters must echo this in reports.
    #[schema(min_length = 1, max_length = 128)]
    pub proof_id: String,
    /// Human-readable compatibility label retained for older manifest fields.
    #[schema(min_length = 1)]
    pub label: String,
    /// Required evidence field path in BrowserAdapterCompletionReport.
    #[schema(min_length = 1)]
    pub evidence_field: String,
    /// Runtime invariant Beatbox expects before trusting completion.
    #[schema(min_length = 1)]
    pub required_invariant: String,
}

impl Default for BrowserAdapterCompletionProofRequirement {
    fn default() -> Self {
        Self {
            proof_id: "browser_process_terminated".to_string(),
            label: "browser process exited or was killed".to_string(),
            evidence_field: "process_terminated".to_string(),
            required_invariant:
                "browser process is no longer running before completion is reported".to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct BrowserAdapterCompletionReport {
    /// Must match the BrowserAdapterLaunchRequest request_id.
    #[schema(min_length = 1, max_length = 128)]
    pub request_id: String,
    /// Must match the trusted adapter id chosen for launch.
    #[schema(min_length = 1, max_length = 128)]
    pub adapter_id: String,
    #[schema(min_length = 1, max_length = 128)]
    pub contract_version: String,
    pub process_terminated: bool,
    pub temporary_profile_removed: bool,
    pub plaintext_artifacts_removed: bool,
    pub egress_log_sealed_or_discarded: bool,
    #[schema(max_items = 64)]
    pub sealed_artifact_handles: Vec<String>,
    #[schema(max_items = 64)]
    pub proof_ids: Vec<String>,
    #[schema(max_items = 64)]
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BrowserAdapterCompletionValidationDecision {
    Rejected,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct BrowserAdapterCompletionValidationResponse {
    pub decision: BrowserAdapterCompletionValidationDecision,
    pub report_shape_complete: bool,
    pub verified_on_production_path: bool,
    pub trusted_for_sensitive_work: bool,
    pub request_id: String,
    pub adapter_id: String,
    pub contract_version: String,
    pub missing_proof_ids: Vec<String>,
    pub unexpected_proof_ids: Vec<String>,
    pub failed_evidence_fields: Vec<String>,
    pub required_completion_proofs: Vec<String>,
    pub completion_proof_contract: Vec<BrowserAdapterCompletionProofRequirement>,
    pub reasons: Vec<String>,
    pub required_next_steps: Vec<String>,
    pub adapter_contract: BrowserAdapterContract,
}

impl Default for BrowserAdapterCompletionReport {
    fn default() -> Self {
        let adapter = BrowserAdapterContract::default();
        Self {
            request_id: "browser-adapter-launch-template-v1".to_string(),
            adapter_id: "tempo-conformance-adapter-v1".to_string(),
            contract_version: adapter.version.clone(),
            process_terminated: true,
            temporary_profile_removed: true,
            plaintext_artifacts_removed: true,
            egress_log_sealed_or_discarded: true,
            sealed_artifact_handles: Vec::new(),
            proof_ids: adapter
                .completion_proof_contract
                .into_iter()
                .map(|proof| proof.proof_id)
                .collect(),
            notes: vec![
                "template only; not evidence of a real browser session".to_string(),
                "production completion must verify these booleans on the teardown path".to_string(),
            ],
        }
    }
}

fn browser_adapter_completion_proof_contract() -> Vec<BrowserAdapterCompletionProofRequirement> {
    vec![
        BrowserAdapterCompletionProofRequirement {
            proof_id: "browser_process_terminated".to_string(),
            label: "browser process exited or was killed".to_string(),
            evidence_field: "process_terminated".to_string(),
            required_invariant:
                "browser process is no longer running before completion is reported".to_string(),
        },
        BrowserAdapterCompletionProofRequirement {
            proof_id: "temporary_profile_removed".to_string(),
            label: "temporary profile directory removed".to_string(),
            evidence_field: "temporary_profile_removed".to_string(),
            required_invariant: "fresh profile directory is removed before completion is trusted"
                .to_string(),
        },
        BrowserAdapterCompletionProofRequirement {
            proof_id: "plaintext_artifacts_removed".to_string(),
            label: "plaintext artifacts outside the explicit allowlist removed".to_string(),
            evidence_field: "plaintext_artifacts_removed".to_string(),
            required_invariant:
                "non-allowlisted plaintext browser artifacts are removed or never persisted"
                    .to_string(),
        },
        BrowserAdapterCompletionProofRequirement {
            proof_id: "egress_log_sealed_or_discarded".to_string(),
            label: "egress proxy log sealed or discarded according to artifact_mode".to_string(),
            evidence_field: "egress_log_sealed_or_discarded".to_string(),
            required_invariant:
                "network logs follow the requested discard or sealed-artifact storage posture"
                    .to_string(),
        },
    ]
}

fn browser_adapter_completion_report_template_for_launch(
    request_id: &str,
    adapter_id: Option<&str>,
    contract_version: &str,
    completion_proof_contract: &[BrowserAdapterCompletionProofRequirement],
) -> BrowserAdapterCompletionReport {
    BrowserAdapterCompletionReport {
        request_id: request_id.to_string(),
        adapter_id: adapter_id
            .unwrap_or("adapter-id-bound-at-registration")
            .to_string(),
        contract_version: contract_version.to_string(),
        process_terminated: true,
        temporary_profile_removed: true,
        plaintext_artifacts_removed: true,
        egress_log_sealed_or_discarded: true,
        sealed_artifact_handles: Vec::new(),
        proof_ids: completion_proof_contract
            .iter()
            .map(|proof| proof.proof_id.clone())
            .collect(),
        notes: vec![
            "template only; not evidence of a real browser session".to_string(),
            "production completion must verify these booleans on the teardown path".to_string(),
        ],
    }
}

pub const BROWSER_ADAPTER_LAUNCH_LEASE_SECONDS: u64 = 10 * 60;

pub fn browser_adapter_launch_template_issued_at() -> String {
    "1970-01-01T00:00:00Z".to_string()
}

pub fn browser_adapter_launch_template_expires_at() -> String {
    "1970-01-01T00:10:00Z".to_string()
}

pub fn browser_adapter_launch_default_max_session_seconds() -> u64 {
    BROWSER_ADAPTER_LAUNCH_LEASE_SECONDS
}

pub fn browser_adapter_launch_default_replay_protection_required() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, utoipa::ToSchema)]
pub struct BrowserAdapterLaunchRequest {
    /// Server-issued request identifier for adapter logs and completion proofs.
    #[schema(min_length = 1, max_length = 128)]
    pub request_id: String,
    /// RFC3339 timestamp for when this envelope was issued. Discovery and
    /// conformance templates use a deterministic placeholder; live launch-plan
    /// preflights use server time.
    pub issued_at: String,
    /// RFC3339 timestamp after which a future launcher must reject this
    /// envelope instead of attempting adapter execution.
    pub expires_at: String,
    /// Maximum browser session lifetime represented by this envelope.
    #[schema(minimum = 1)]
    pub max_session_seconds: u64,
    /// Adapter identifier for the envelope. Runtime launch requests use the
    /// trusted registered adapter id; conformance fixtures may carry a sample
    /// id and still do not imply adapter registration, trust, or launchability.
    /// Null in discovery templates before an adapter is selected.
    #[schema(required = true, min_length = 1, max_length = 128)]
    pub adapter_id: Option<String>,
    pub contract_version: String,
    pub requested_level: BrowserSandboxLevel,
    pub actor: BrowserSessionActor,
    pub sensitivity: BrowserSensitivity,
    #[serde(default)]
    #[schema(required = true)]
    pub sensitive_activity_mode: BrowserSensitiveActivityMode,
    pub target_origins: Vec<String>,
    pub credential_mode: BrowserCredentialMode,
    pub artifact_mode: BrowserArtifactMode,
    pub requested_controls: Vec<BrowserSandboxControl>,
    pub guard_plan: BrowserAdmissionGuardPlan,
    pub required_completion_proofs: Vec<String>,
    #[serde(default = "browser_adapter_completion_proof_contract")]
    #[schema(required = true)]
    pub completion_proof_contract: Vec<BrowserAdapterCompletionProofRequirement>,
    #[serde(default = "BrowserAdapterCompletionReport::default")]
    #[schema(required = true)]
    pub completion_report_template: BrowserAdapterCompletionReport,
    pub same_user_capability_required: bool,
    pub endpoint_network_policy_binding_required: bool,
    /// Future launchers must reject reused request_id values for this adapter.
    pub replay_protection_required: bool,
    pub notes: Vec<String>,
}

impl<'de> Deserialize<'de> for BrowserAdapterLaunchRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            request_id: String,
            #[serde(default = "browser_adapter_launch_template_issued_at")]
            issued_at: String,
            #[serde(default = "browser_adapter_launch_template_expires_at")]
            expires_at: String,
            #[serde(default = "browser_adapter_launch_default_max_session_seconds")]
            max_session_seconds: u64,
            #[serde(default)]
            adapter_id: Option<String>,
            contract_version: String,
            requested_level: BrowserSandboxLevel,
            actor: BrowserSessionActor,
            sensitivity: BrowserSensitivity,
            #[serde(default)]
            sensitive_activity_mode: BrowserSensitiveActivityMode,
            #[serde(default)]
            target_origins: Vec<String>,
            credential_mode: BrowserCredentialMode,
            artifact_mode: BrowserArtifactMode,
            #[serde(default)]
            requested_controls: Vec<BrowserSandboxControl>,
            guard_plan: BrowserAdmissionGuardPlan,
            #[serde(default)]
            required_completion_proofs: Vec<String>,
            #[serde(default = "browser_adapter_completion_proof_contract")]
            completion_proof_contract: Vec<BrowserAdapterCompletionProofRequirement>,
            #[serde(default)]
            completion_report_template: Option<BrowserAdapterCompletionReport>,
            same_user_capability_required: bool,
            endpoint_network_policy_binding_required: bool,
            #[serde(default = "browser_adapter_launch_default_replay_protection_required")]
            replay_protection_required: bool,
            #[serde(default)]
            notes: Vec<String>,
        }

        let wire = Wire::deserialize(deserializer)?;
        let completion_report_template = wire.completion_report_template.unwrap_or_else(|| {
            browser_adapter_completion_report_template_for_launch(
                &wire.request_id,
                wire.adapter_id.as_deref(),
                &wire.contract_version,
                &wire.completion_proof_contract,
            )
        });

        Ok(Self {
            request_id: wire.request_id,
            issued_at: wire.issued_at,
            expires_at: wire.expires_at,
            max_session_seconds: wire.max_session_seconds,
            adapter_id: wire.adapter_id,
            contract_version: wire.contract_version,
            requested_level: wire.requested_level,
            actor: wire.actor,
            sensitivity: wire.sensitivity,
            sensitive_activity_mode: wire.sensitive_activity_mode,
            target_origins: wire.target_origins,
            credential_mode: wire.credential_mode,
            artifact_mode: wire.artifact_mode,
            requested_controls: wire.requested_controls,
            guard_plan: wire.guard_plan,
            required_completion_proofs: wire.required_completion_proofs,
            completion_proof_contract: wire.completion_proof_contract,
            completion_report_template,
            same_user_capability_required: wire.same_user_capability_required,
            endpoint_network_policy_binding_required: wire.endpoint_network_policy_binding_required,
            replay_protection_required: wire.replay_protection_required,
            notes: wire.notes,
        })
    }
}

impl Default for BrowserAdapterLaunchRequest {
    fn default() -> Self {
        let adapter = BrowserAdapterContract::default();
        let target_origins = vec!["https://example.com".to_string()];
        let mut guard_plan = BrowserAdmissionGuardPlan::default();
        guard_plan.network.allowed_origins = target_origins.clone();
        Self {
            request_id: "browser-adapter-launch-template-v1".to_string(),
            issued_at: browser_adapter_launch_template_issued_at(),
            expires_at: browser_adapter_launch_template_expires_at(),
            max_session_seconds: BROWSER_ADAPTER_LAUNCH_LEASE_SECONDS,
            adapter_id: None,
            contract_version: adapter.version.clone(),
            requested_level: BrowserSandboxLevel::OsIsolated,
            actor: BrowserSessionActor::Agent,
            sensitivity: BrowserSensitivity::Sensitive,
            sensitive_activity_mode: BrowserSensitiveActivityMode::NetworkSuppressed,
            target_origins,
            credential_mode: BrowserCredentialMode::NoCredentials,
            artifact_mode: BrowserArtifactMode::Discard,
            requested_controls: vec![
                BrowserSandboxControl::FreshProfile,
                BrowserSandboxControl::NoAmbientCredentials,
                BrowserSandboxControl::EgressPolicy,
                BrowserSandboxControl::LocalNetworkBlock,
                BrowserSandboxControl::TeardownProof,
            ],
            guard_plan,
            required_completion_proofs: adapter.required_completion_proofs,
            completion_proof_contract: adapter.completion_proof_contract.clone(),
            completion_report_template: browser_adapter_completion_report_template_for_launch(
                "browser-adapter-launch-template-v1",
                None,
                &adapter.version,
                &adapter.completion_proof_contract,
            ),
            same_user_capability_required: true,
            endpoint_network_policy_binding_required: true,
            replay_protection_required: true,
            notes: vec![
                "template only; not a browser launch grant".to_string(),
                "same-user capability and endpoint network policy must be bound before use"
                    .to_string(),
            ],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct BrowserAdapterHandoff {
    pub contract_version: String,
    #[schema(required = true)]
    pub launch_endpoint: Option<String>,
    pub launchable: bool,
    pub handoff_fields: Vec<String>,
    #[serde(default)]
    #[schema(required = true)]
    pub launch_request_template: BrowserAdapterLaunchRequest,
    pub required_completion_proofs: Vec<String>,
    #[serde(default = "browser_adapter_completion_proof_contract")]
    #[schema(required = true)]
    pub completion_proof_contract: Vec<BrowserAdapterCompletionProofRequirement>,
    pub unavailable_reason: String,
}

impl Default for BrowserAdapterHandoff {
    fn default() -> Self {
        let adapter = BrowserAdapterContract::default();
        Self {
            contract_version: adapter.version,
            launch_endpoint: adapter.launch_endpoint,
            launchable: false,
            handoff_fields: adapter.handoff_fields,
            launch_request_template: BrowserAdapterLaunchRequest::default(),
            required_completion_proofs: adapter.required_completion_proofs,
            completion_proof_contract: adapter.completion_proof_contract,
            unavailable_reason:
                "fresh server-issued browser adapter handoff required before launch".to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct BrowserAdapterManifestRequest {
    /// Stable adapter identifier; non-empty, at most 128 bytes, no surrounding whitespace.
    #[schema(min_length = 1, max_length = 128)]
    pub adapter_id: String,
    /// Browser adapter contract version; non-empty with no surrounding whitespace.
    #[schema(min_length = 1, max_length = 128)]
    pub contract_version: String,
    /// Optional HTTPS launch endpoint string. Only syntax and literal local/private hosts are checked.
    #[schema(required = true, min_length = 1)]
    pub launch_endpoint: Option<String>,
    #[schema(max_items = 64)]
    pub supported_levels: Vec<BrowserSandboxLevel>,
    #[schema(max_items = 64)]
    pub supported_controls: Vec<BrowserSandboxControl>,
    /// Required guard_plan field names; entries must be non-empty without surrounding whitespace.
    #[schema(max_items = 64)]
    pub guard_fields: Vec<String>,
    /// Required completion proof labels; entries must be non-empty without surrounding whitespace.
    #[schema(max_items = 64)]
    pub completion_proofs: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BrowserAdapterValidationDecision {
    Rejected,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BrowserAdapterRegistrationDecision {
    Rejected,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BrowserAdapterLaunchPlanDecision {
    Rejected,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct BrowserAdapterConformanceExpectation {
    pub decision: BrowserAdapterValidationDecision,
    pub manifest_complete: bool,
    pub launchable: bool,
    pub trusted_for_sensitive_work: bool,
    pub endpoint_network_policy_bound: bool,
    pub missing_levels: Vec<BrowserSandboxLevel>,
    pub missing_controls: Vec<BrowserSandboxControl>,
    pub missing_guard_fields: Vec<String>,
    pub missing_completion_proofs: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct BrowserAdapterConformanceCase {
    pub name: String,
    pub manifest: BrowserAdapterManifestRequest,
    pub expected_rest_status: u16,
    #[schema(required = true)]
    pub expected_rest_error_code: Option<String>,
    #[schema(required = true)]
    pub expected_mcp_error_code: Option<i64>,
    pub expected_mcp_error_message_contains: Vec<String>,
    #[schema(required = true)]
    pub expected_validation: Option<BrowserAdapterConformanceExpectation>,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct BrowserAdapterConformanceProfile {
    pub profile_version: String,
    pub field_complete_manifest: BrowserAdapterManifestRequest,
    #[serde(default)]
    #[schema(required = true)]
    pub field_complete_launch_request: BrowserAdapterLaunchRequest,
    pub field_complete_expectation: BrowserAdapterConformanceExpectation,
    pub required_cases: Vec<BrowserAdapterConformanceCase>,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct BrowserAdapterContractResponse {
    pub adapter_contract: BrowserAdapterContract,
    pub conformance_profile: BrowserAdapterConformanceProfile,
    pub required_levels: Vec<BrowserSandboxLevel>,
    pub required_controls: Vec<BrowserSandboxControl>,
    pub launchable: bool,
    pub trusted_for_sensitive_work: bool,
    pub endpoint_network_policy_bound: bool,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct BrowserAdapterCapabilityIssueRequest {
    pub actor: BrowserSessionActor,
    pub sensitivity: BrowserSensitivity,
    /// Optional requested browser suppression posture to bind into a later
    /// launch-plan preflight. When present, the nested admission request must
    /// carry the same sensitive_activity_mode.
    #[serde(default)]
    pub sensitive_activity_mode: Option<BrowserSensitiveActivityMode>,
    /// Optional adapter identifier to bind the capability to. When present, any
    /// consuming registration or launch-plan preflight must use the same
    /// manifest adapter_id.
    #[serde(default)]
    #[schema(min_length = 1, max_length = 128)]
    pub adapter_id: Option<String>,
    /// Optional requested lifetime in seconds. When present, it must be between
    /// 1 and 300 seconds.
    #[serde(default)]
    #[schema(minimum = 1, maximum = 300)]
    pub ttl_seconds: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct BrowserAdapterCapabilityIssueResponse {
    /// Secret one-time capability. Treat as bearer material; Beatbox stores only
    /// a digest and never echoes it from consuming preflight responses.
    #[schema(min_length = 1, max_length = 256)]
    pub same_user_capability: String,
    pub expires_at: String,
    #[schema(minimum = 1, maximum = 300)]
    pub ttl_seconds: u64,
    pub actor: BrowserSessionActor,
    pub sensitivity: BrowserSensitivity,
    #[serde(default)]
    #[schema(required = true)]
    pub sensitive_activity_mode: Option<BrowserSensitiveActivityMode>,
    #[schema(required = true)]
    pub adapter_id: Option<String>,
    pub registration_endpoint: String,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct BrowserAdapterRegistrationRequest {
    pub actor: BrowserSessionActor,
    pub sensitivity: BrowserSensitivity,
    /// Caller-supplied same-user capability for the future local user/session
    /// that would own this adapter. Beatbox can bind only a live one-time
    /// capability issued by its REST control plane, consumes it at most once,
    /// and never echoes it in responses.
    #[schema(min_length = 1, max_length = 256)]
    pub same_user_capability: String,
    pub manifest: BrowserAdapterManifestRequest,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct BrowserAdapterRegistrationResponse {
    pub decision: BrowserAdapterRegistrationDecision,
    pub adapter_id: String,
    pub actor: BrowserSessionActor,
    pub sensitivity: BrowserSensitivity,
    pub registered: bool,
    pub launchable: bool,
    pub trusted_for_sensitive_work: bool,
    pub endpoint_network_policy_bound: bool,
    pub same_user_capability_bound: bool,
    pub manifest_validation: BrowserAdapterManifestResponse,
    pub reasons: Vec<String>,
    pub required_next_steps: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct BrowserAdapterLaunchPlanRequest {
    /// One-time same-user capability issued by the REST control plane. It is
    /// consumed on a matching preflight and never echoed.
    #[schema(min_length = 1, max_length = 256)]
    pub same_user_capability: String,
    pub admission: BrowserAdmissionRequest,
    pub manifest: BrowserAdapterManifestRequest,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct BrowserAdapterLaunchPlanResponse {
    pub decision: BrowserAdapterLaunchPlanDecision,
    pub request_id: String,
    pub adapter_id: String,
    pub actor: BrowserSessionActor,
    pub sensitivity: BrowserSensitivity,
    pub launchable: bool,
    pub trusted_for_sensitive_work: bool,
    pub endpoint_network_policy_bound: bool,
    pub same_user_capability_bound: bool,
    /// True when this daemon recorded the emitted launch request id in its
    /// bounded replay ledger. This is not launch authorization.
    pub replay_protection_bound: bool,
    pub admission: BrowserAdmissionResponse,
    pub manifest_validation: BrowserAdapterManifestResponse,
    pub launch_request: BrowserAdapterLaunchRequest,
    pub completion_validation_endpoint: String,
    pub reasons: Vec<String>,
    pub required_next_steps: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct BrowserAdapterLaunchClaimRequest {
    /// Full server-issued launch request returned by
    /// /v1/browser/adapter/launch/plan. Beatbox compares it against the stored
    /// canonical envelope before marking the request id claimed.
    pub launch_request: BrowserAdapterLaunchRequest,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BrowserAdapterLaunchClaimDecision {
    Claimed,
    Rejected,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct BrowserAdapterLaunchClaimResponse {
    pub decision: BrowserAdapterLaunchClaimDecision,
    pub request_id: String,
    #[schema(required = true)]
    pub adapter_id: Option<String>,
    pub server_issued_launch_request: bool,
    pub canonical_request_matched: bool,
    pub launch_request_unexpired: bool,
    pub launch_request_claim_bound: bool,
    pub launch_request_replay_detected: bool,
    pub launchable: bool,
    pub trusted_for_sensitive_work: bool,
    pub endpoint_network_policy_bound: bool,
    pub reasons: Vec<String>,
    pub required_next_steps: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct BrowserAdapterManifestResponse {
    pub decision: BrowserAdapterValidationDecision,
    pub manifest_complete: bool,
    pub launchable: bool,
    pub trusted_for_sensitive_work: bool,
    pub adapter_id: String,
    #[schema(required = true)]
    pub launch_endpoint: Option<String>,
    pub endpoint_network_policy_bound: bool,
    pub missing_levels: Vec<BrowserSandboxLevel>,
    pub missing_controls: Vec<BrowserSandboxControl>,
    pub missing_guard_fields: Vec<String>,
    pub missing_completion_proofs: Vec<String>,
    pub reasons: Vec<String>,
    pub required_next_steps: Vec<String>,
    pub adapter_contract: BrowserAdapterContract,
    pub conformance_profile: BrowserAdapterConformanceProfile,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct BrowserAdmissionRequest {
    pub requested_level: BrowserSandboxLevel,
    pub actor: BrowserSessionActor,
    pub sensitivity: BrowserSensitivity,
    /// Requested privacy/suppression posture for sensitive browser work. This
    /// is part of the future adapter contract and is still fail-closed until a
    /// production browser launcher enforces it.
    #[serde(default)]
    pub sensitive_activity_mode: BrowserSensitiveActivityMode,
    /// Bare public HTTP(S) origins this browser session is allowed to target.
    /// Entries must contain only scheme, host, and optional port. The runtime
    /// rejects credentials, paths, queries, fragments, localhost, private/LAN
    /// address space, link-local metadata ranges, and more than 16 entries.
    #[serde(default)]
    pub target_origins: Vec<String>,
    /// Credential posture requested for the session. Non-default modes remain
    /// fail-closed until a real browser substrate implements scoped handling.
    #[serde(default)]
    pub credential_mode: BrowserCredentialMode,
    /// Artifact persistence posture requested for the session. Non-default
    /// modes remain fail-closed until storage and sealing are implemented.
    #[serde(default)]
    pub artifact_mode: BrowserArtifactMode,
    #[serde(default)]
    pub required_controls: Vec<BrowserSandboxControl>,
    #[serde(default)]
    pub allow_downgrade: bool,
    #[serde(default)]
    pub task_label: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BrowserAdmissionDecision {
    Accepted,
    Rejected,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct BrowserAdmissionResponse {
    pub decision: BrowserAdmissionDecision,
    pub runnable_browser_sessions: bool,
    pub requested_level: BrowserSandboxLevel,
    #[schema(required = true)]
    pub selected_level: Option<BrowserSandboxLevel>,
    pub actor: BrowserSessionActor,
    pub sensitivity: BrowserSensitivity,
    /// Echo of the requested privacy/suppression posture.
    #[serde(default)]
    #[schema(required = true)]
    pub sensitive_activity_mode: BrowserSensitiveActivityMode,
    /// Echo of the validated target origins from the admission request.
    #[serde(default)]
    #[schema(required = true)]
    pub target_origins: Vec<String>,
    /// Echo of the requested credential posture.
    #[serde(default)]
    #[schema(required = true)]
    pub credential_mode: BrowserCredentialMode,
    /// Echo of the requested artifact persistence posture.
    #[serde(default)]
    #[schema(required = true)]
    pub artifact_mode: BrowserArtifactMode,
    #[serde(default)]
    #[schema(required = true)]
    pub requested_controls: Vec<BrowserSandboxControl>,
    #[serde(default)]
    #[schema(required = true)]
    pub requested_profile_controls: Vec<BrowserSandboxControl>,
    #[serde(default)]
    #[schema(required = true)]
    pub missing_controls: Vec<BrowserSandboxControl>,
    #[serde(default)]
    #[schema(required = true)]
    pub level_satisfies_requested_controls: bool,
    /// Non-fatal intent issues that matter before any future runnable browser
    /// session can be admitted.
    #[serde(default)]
    #[schema(required = true)]
    pub intent_warnings: Vec<String>,
    /// Structured guard posture a future browser adapter must enforce before
    /// this admission request can become runnable. It is a plan, not a claim
    /// that the current daemon enforces browser isolation.
    #[serde(default)]
    #[schema(required = true)]
    pub guard_plan: BrowserAdmissionGuardPlan,
    /// Adapter handoff metadata for future browser launchers. This is
    /// fail-closed while launch_endpoint is null and launchable is false.
    #[serde(default)]
    #[schema(required = true)]
    pub adapter_handoff: BrowserAdapterHandoff,
    pub downgrade_allowed: bool,
    pub reasons: Vec<String>,
    pub required_next_steps: Vec<String>,
    pub profiles_endpoint: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CapabilityLane {
    pub lane: Lane,
    pub available: bool,
    pub substrate: String,
    pub grade: BTreeMap<String, String>,
    pub mechanisms: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CapabilityLimits {
    pub sync_wall_ms: u64,
    pub job_wall_ms: u64,
    pub default_wall_ms: u64,
    pub default_memory_bytes: u64,
    pub default_output_bytes: u64,
    pub max_request_bytes: usize,
    pub max_memory_bytes: u64,
    pub max_output_bytes: u64,
    pub max_fuel: u64,
    pub max_concurrent_sync: usize,
    pub max_concurrent_jobs: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CapabilitiesResponse {
    pub version: String,
    pub lanes: Vec<CapabilityLane>,
    pub limits: CapabilityLimits,
    pub engines: BTreeMap<String, String>,
    pub browser_sandbox: BrowserProfilesResponse,
    /// Aether-compatible payment evidence accepted by the MCP boundary. The
    /// payload header is never echoed; only its companion hash may be surfaced
    /// in MCP response metadata for downstream audit correlation.
    #[serde(default)]
    #[schema(required = true)]
    pub aether_payment: AetherPaymentContextCapabilities,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AetherPaymentContextCapabilities {
    pub payment_header: String,
    pub payment_hash_header: String,
    pub accepted_mcp_headers: Vec<String>,
    pub require_hash_with_payment: bool,
    pub echo_payment_payload: bool,
    pub echo_payment_hash: bool,
    pub max_payment_header_bytes: usize,
}

impl Default for AetherPaymentContextCapabilities {
    fn default() -> Self {
        Self {
            payment_header: "x-payment".to_string(),
            payment_hash_header: "x-aether-payment-hash".to_string(),
            accepted_mcp_headers: vec![
                "x-payment".to_string(),
                "x-aether-payment-hash".to_string(),
            ],
            require_hash_with_payment: true,
            echo_payment_payload: false,
            echo_payment_hash: true,
            max_payment_header_bytes: 8192,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    #[serde(default)]
    pub fs: FsPolicy,
    #[serde(default)]
    pub net: NetPolicy,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub secrets: Vec<Secret>,
    #[serde(default)]
    pub limits: Limits,
    #[serde(default)]
    pub determinism: Determinism,
    #[serde(default)]
    pub double_jail: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct FsPolicy {
    #[serde(default)]
    #[schema(value_type = Option<String>)]
    pub workspace: Option<PathBuf>,
    #[serde(default)]
    pub mounts: Vec<Mount>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Mount {
    #[schema(value_type = String)]
    pub host: PathBuf,
    #[schema(value_type = String)]
    pub guest: PathBuf,
    pub mode: MountMode,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum MountMode {
    Ro,
    Rw,
}

// `Deny` is an empty *struct* variant (`Deny {}`), not a unit variant, because
// serde silently ignores `deny_unknown_fields` on unit variants of an
// internally-tagged enum — `{"kind":"deny","allow_domains":[...]}` would be
// accepted and the extra keys dropped. An empty struct variant is validated, so
// unknown keys on `deny` are rejected like every other variant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum NetPolicy {
    Deny {},
    Proxy {
        #[serde(default)]
        allow_domains: Vec<String>,
        #[serde(default)]
        allow_ports: Vec<u16>,
    },
}

impl Default for NetPolicy {
    fn default() -> Self {
        Self::Deny {}
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Secret {
    pub name: String,
    pub value_ref: String,
    pub expose: SecretExpose,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SecretExpose {
    Env,
    File,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    #[serde(default = "default_wall_ms")]
    pub wall_ms: u64,
    #[serde(default = "default_cpu_ms")]
    pub cpu_ms: u64,
    #[serde(default = "default_memory_bytes")]
    pub memory_bytes: u64,
    #[serde(default = "default_output_bytes")]
    pub output_bytes: u64,
    #[serde(default = "default_pids")]
    pub pids: u32,
    #[serde(default = "default_disk_bytes")]
    pub disk_bytes: u64,
    #[serde(default = "default_fuel")]
    pub fuel: Option<u64>,
}

const fn default_wall_ms() -> u64 {
    5_000
}

const fn default_cpu_ms() -> u64 {
    5_000
}

const fn default_memory_bytes() -> u64 {
    64 * 1024 * 1024
}

const fn default_output_bytes() -> u64 {
    1024 * 1024
}

const fn default_pids() -> u32 {
    1
}

const fn default_disk_bytes() -> u64 {
    64 * 1024 * 1024
}

const fn default_fuel() -> Option<u64> {
    Some(10_000_000)
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            wall_ms: default_wall_ms(),
            cpu_ms: default_cpu_ms(),
            memory_bytes: default_memory_bytes(),
            output_bytes: default_output_bytes(),
            pids: default_pids(),
            disk_bytes: default_disk_bytes(),
            fuel: default_fuel(),
        }
    }
}

// `Off` is an empty struct variant for the same reason as `NetPolicy::Deny`:
// `deny_unknown_fields` is a no-op on unit variants of an internally-tagged
// enum, so `{"kind":"off","seed":5}` would be silently accepted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Determinism {
    Off {},
    Seeded { seed: u64, epoch_ms: u64 },
}

impl Default for Determinism {
    fn default() -> Self {
        Self::Off {}
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ExecuteRequest {
    pub lane: Lane,
    pub source: Source,
    #[serde(default)]
    pub entrypoint: Option<String>,
    #[serde(default)]
    pub input: serde_json::Value,
    #[serde(default)]
    pub stdin: String,
    #[serde(default)]
    pub policy: Policy,
    #[serde(default)]
    pub idempotency_key: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    Canceled,
}

impl JobStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Canceled => "canceled",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CreateJobResponse {
    pub job_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct JobRecord {
    pub job_id: String,
    pub status: JobStatus,
    pub request: ExecuteRequest,
    pub result: Option<ExecutionResult>,
    pub error: Option<ErrorBody>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Source {
    Inline {
        code: String,
    },
    WasmFile {
        #[schema(value_type = String)]
        path: PathBuf,
    },
    WasmWat {
        text: String,
    },
    WasmBytesBase64 {
        bytes: String,
    },
    ModuleRef {
        sha256: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    Ok,
    Error,
    Timeout,
    Oom,
    Killed,
    Denied,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ExecutionResult {
    pub status: ExecutionStatus,
    pub value: serde_json::Value,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stdout_truncated: bool,
    pub stderr: String,
    pub stderr_truncated: bool,
    pub error: Option<ErrorBody>,
    pub metrics: Metrics,
    pub lane: Lane,
    pub deterministic: bool,
    pub inputs_digest: String,
    pub engine_version: String,
    pub beatbox_version: String,
    pub effective_isolation: EffectiveIsolation,
    pub egress: Vec<EgressRecord>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Metrics {
    pub wall_time_ms: u64,
    /// CPU time in milliseconds, when the lane measures it separately from wall
    /// time. The W0 wasm lane does not, so this is `None` there — use `fuel_used`
    /// as the deterministic compute signal rather than treating wall time as CPU.
    pub cpu_time_ms: Option<u64>,
    pub fuel_used: Option<u64>,
    pub peak_memory_bytes: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ErrorBody {
    pub code: String,
    pub message: String,
}

impl ErrorBody {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct EffectiveIsolation {
    pub os: String,
    pub mechanisms: Vec<String>,
    pub landlock_abi: Option<u32>,
    pub downgrades: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct EgressRecord {
    pub domain: String,
    pub port: u16,
    pub bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ErrorResponse {
    pub error: ErrorBody,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_round_trips_without_defaults_expanding_json() -> Result<(), serde_json::Error> {
        let policy = Policy::default();
        let encoded = serde_json::to_string(&policy)?;
        let decoded: Policy = serde_json::from_str(&encoded)?;
        assert_eq!(decoded, policy);
        Ok(())
    }

    #[test]
    fn partial_limits_merge_onto_defaults() -> Result<(), serde_json::Error> {
        let policy: Policy = serde_json::from_str(r#"{"limits": {"wall_ms": 1000}}"#)?;
        assert_eq!(policy.limits.wall_ms, 1000);
        // Untouched fields fall back to their defaults instead of failing to parse.
        assert_eq!(policy.limits.cpu_ms, Limits::default().cpu_ms);
        assert_eq!(policy.limits.memory_bytes, Limits::default().memory_bytes);
        assert_eq!(policy.limits.fuel, Limits::default().fuel);
        Ok(())
    }

    #[test]
    fn unknown_policy_fields_are_rejected() {
        // A typo'd top-level policy key must be an error, not a silent default.
        assert!(serde_json::from_str::<Policy>(r#"{"double_jale": true}"#).is_err());
        // A typo'd limits key must be rejected too.
        assert!(serde_json::from_str::<Policy>(r#"{"limits": {"wall_mss": 1}}"#).is_err());
    }

    #[test]
    fn unknown_fields_on_tagged_enum_unit_variants_are_rejected() -> Result<(), serde_json::Error> {
        // Regression: deny_unknown_fields must also fire on the empty variants
        // (`deny`, `off`), not just the struct-shaped ones.
        assert!(serde_json::from_str::<NetPolicy>(r#"{"kind":"deny"}"#).is_ok());
        assert!(
            serde_json::from_str::<NetPolicy>(r#"{"kind":"deny","allow_domains":["x"]}"#).is_err()
        );
        assert!(serde_json::from_str::<Determinism>(r#"{"kind":"off"}"#).is_ok());
        assert!(serde_json::from_str::<Determinism>(r#"{"kind":"off","seed":5}"#).is_err());
        // Defaults and round-trips still hold with the empty-struct-variant shape.
        assert_eq!(NetPolicy::default(), NetPolicy::Deny {});
        assert_eq!(Determinism::default(), Determinism::Off {});
        assert_eq!(
            serde_json::to_string(&NetPolicy::default())?,
            r#"{"kind":"deny"}"#
        );
        Ok(())
    }

    #[test]
    fn unknown_request_and_source_fields_are_rejected() {
        assert!(serde_json::from_str::<ExecuteRequest>(
            r#"{"lane": "wasm", "source": {"kind": "wasm_wat", "text": "(module)"}, "polcy": {}}"#
        )
        .is_err());
        assert!(
            serde_json::from_str::<Source>(
                r#"{"kind": "wasm_wat", "text": "(module)", "txt": "x"}"#
            )
            .is_err()
        );
    }

    #[test]
    fn browser_profile_response_tolerates_additive_fields() -> Result<(), serde_json::Error> {
        let response: BrowserProfilesResponse = serde_json::from_str(
            r#"{
                "version": "browser-sandbox-v1",
                "runnable_browser_sessions": false,
                "default_level": null,
                "integration": {
                    "status": "planned",
                    "consumer": "tempo",
                    "endpoint": "/v1/browser/profiles",
                    "admission_endpoint": "/v1/browser/admit",
                    "selection_field": "requested_level",
                    "required_consumer_behavior": ["treat unavailable as rejected"],
                    "future_integration_note": true
                },
                "profiles": [
                    {
                        "level": "network_suppressed",
                        "availability": "planned",
                        "summary": "planned profile from an older daemon",
                        "isolation_boundary": "none yet",
                        "privacy_controls": [],
                        "egress_controls": [],
                        "credential_controls": [],
                        "storage_controls": [],
                        "encryption_claims": [],
                        "non_goals": [],
                        "downgrade_reasons": [],
                        "future_profile_note": "ignored"
                    }
                ],
                "future_response_note": "ignored"
            }"#,
        )?;
        assert_eq!(
            response.profiles[0].controls,
            Vec::<BrowserSandboxControl>::new()
        );
        assert_eq!(
            response.integration.adapter.status,
            BrowserSandboxAvailability::Unavailable
        );
        assert_eq!(response.integration.adapter.launch_endpoint, None);
        assert!(
            response
                .integration
                .adapter
                .handoff_fields
                .iter()
                .any(|field| field == "guard_plan")
        );
        assert!(
            BrowserAdapterContract::default()
                .required_guard_fields
                .iter()
                .any(|field| field == "guard_plan.storage.teardown_proof_required")
        );
        assert!(
            BrowserAdapterContract::default()
                .required_guard_fields
                .iter()
                .any(|field| field == "guard_plan.network.outbound_network_disabled_without_proxy")
        );
        Ok(())
    }

    #[test]
    fn browser_admission_response_defaults_additive_control_fields() -> Result<(), serde_json::Error>
    {
        let response: BrowserAdmissionResponse = serde_json::from_str(
            r#"{
                "decision": "rejected",
                "runnable_browser_sessions": false,
                "requested_level": "os_isolated",
                "selected_level": null,
                "actor": "agent",
                "sensitivity": "sensitive",
                "downgrade_allowed": false,
                "reasons": ["no runnable browser sandbox"],
                "required_next_steps": ["implement a browser launcher"],
                "profiles_endpoint": "/v1/browser/profiles",
                "future_response_note": "ignored"
            }"#,
        )?;
        assert_eq!(
            response.requested_controls,
            Vec::<BrowserSandboxControl>::new()
        );
        assert_eq!(
            response.requested_profile_controls,
            Vec::<BrowserSandboxControl>::new()
        );
        assert_eq!(
            response.missing_controls,
            Vec::<BrowserSandboxControl>::new()
        );
        assert_eq!(response.target_origins, Vec::<String>::new());
        assert_eq!(
            response.credential_mode,
            BrowserCredentialMode::NoCredentials
        );
        assert_eq!(response.artifact_mode, BrowserArtifactMode::Discard);
        assert_eq!(
            response.sensitive_activity_mode,
            BrowserSensitiveActivityMode::Standard
        );
        assert_eq!(response.intent_warnings, Vec::<String>::new());
        assert_eq!(
            response.guard_plan.network.allowed_origins,
            Vec::<String>::new()
        );
        assert!(response.guard_plan.network.deny_private_networks);
        assert!(response.guard_plan.network.require_dns_rebinding_protection);
        assert!(response.guard_plan.network.require_redirect_revalidation);
        assert!(response.guard_plan.network.require_proxy_enforcement);
        assert!(
            response
                .guard_plan
                .network
                .outbound_network_disabled_without_proxy
        );
        assert!(!response.guard_plan.credentials.ambient_credentials_allowed);
        assert!(response.guard_plan.storage.teardown_proof_required);
        assert_eq!(
            response.guard_plan.suppression.mode,
            BrowserSensitiveActivityMode::Standard
        );
        assert!(!response.guard_plan.suppression.suppress_unapproved_network);
        assert!(
            response
                .guard_plan
                .required_runtime_guards
                .iter()
                .any(|guard| guard.contains("fresh server-issued guard plan"))
        );
        assert!(!response.adapter_handoff.launchable);
        assert_eq!(response.adapter_handoff.launch_endpoint, None);
        assert!(
            response
                .adapter_handoff
                .handoff_fields
                .iter()
                .any(|field| field == "guard_plan")
        );
        assert!(
            response
                .adapter_handoff
                .launch_request_template
                .same_user_capability_required
        );
        assert!(
            response
                .adapter_handoff
                .launch_request_template
                .endpoint_network_policy_binding_required
        );
        assert!(
            response
                .adapter_handoff
                .completion_proof_contract
                .iter()
                .any(|proof| proof.proof_id == "temporary_profile_removed")
        );
        assert_eq!(
            response
                .adapter_handoff
                .launch_request_template
                .completion_report_template
                .proof_ids,
            response
                .adapter_handoff
                .launch_request_template
                .completion_proof_contract
                .iter()
                .map(|proof| proof.proof_id.clone())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            response
                .adapter_handoff
                .launch_request_template
                .guard_plan
                .network
                .allowed_origins,
            response
                .adapter_handoff
                .launch_request_template
                .target_origins
        );
        assert!(
            response
                .adapter_handoff
                .unavailable_reason
                .contains("fresh server-issued browser adapter handoff")
        );
        assert!(!response.level_satisfies_requested_controls);
        Ok(())
    }

    #[test]
    fn browser_adapter_launch_request_backfills_completion_report_from_sibling_fields()
    -> Result<(), serde_json::Error> {
        let request: BrowserAdapterLaunchRequest = serde_json::from_str(
            r#"{
                "request_id": "old-launch-123",
                "adapter_id": "tempo-old-adapter",
                "contract_version": "browser-adapter-v1",
                "requested_level": "os_isolated",
                "actor": "agent",
                "sensitivity": "sensitive",
                "target_origins": ["https://example.com"],
                "credential_mode": "no_credentials",
                "artifact_mode": "discard",
                "requested_controls": ["teardown_proof"],
                "guard_plan": {
                    "network": {
                        "allowed_origins": ["https://example.com"],
                        "deny_private_networks": true,
                        "deny_localhost": true,
                        "deny_metadata_endpoints": true,
                        "require_dns_rebinding_protection": true,
                        "require_redirect_revalidation": true,
                        "require_proxy_enforcement": true,
                        "outbound_network_disabled_without_proxy": true
                    },
                    "credentials": {
                        "mode": "no_credentials",
                        "ambient_credentials_allowed": false,
                        "user_mediation_required": false,
                        "scoped_secret_channel_required": false
                    },
                    "storage": {
                        "mode": "discard",
                        "plaintext_persistence_allowed": false,
                        "explicit_artifact_allowlist_required": false,
                        "encryption_required_for_persistence": false,
                        "teardown_proof_required": true
                    },
                    "required_runtime_guards": ["fresh guard plan"]
                },
                "required_completion_proofs": ["temporary profile directory removed"],
                "same_user_capability_required": true,
                "endpoint_network_policy_binding_required": true,
                "notes": ["old payload"]
            }"#,
        )?;

        assert_eq!(request.request_id, "old-launch-123");
        assert_eq!(
            request.issued_at,
            browser_adapter_launch_template_issued_at()
        );
        assert_eq!(
            request.expires_at,
            browser_adapter_launch_template_expires_at()
        );
        assert_eq!(
            request.max_session_seconds,
            BROWSER_ADAPTER_LAUNCH_LEASE_SECONDS
        );
        assert!(request.replay_protection_required);
        assert_eq!(request.adapter_id.as_deref(), Some("tempo-old-adapter"));
        assert_eq!(
            request.sensitive_activity_mode,
            BrowserSensitiveActivityMode::Standard
        );
        assert_eq!(
            request.guard_plan.suppression.mode,
            BrowserSensitiveActivityMode::Standard
        );
        assert!(
            request
                .completion_proof_contract
                .iter()
                .any(|proof| proof.proof_id == "temporary_profile_removed")
        );
        assert_eq!(
            request.completion_report_template.request_id,
            request.request_id
        );
        assert_eq!(
            request.completion_report_template.adapter_id,
            "tempo-old-adapter"
        );
        assert_eq!(
            request.completion_report_template.proof_ids,
            request
                .completion_proof_contract
                .iter()
                .map(|proof| proof.proof_id.clone())
                .collect::<Vec<_>>()
        );
        Ok(())
    }

    #[test]
    fn browser_adapter_nested_old_payloads_backfill_non_empty_proof_contracts()
    -> Result<(), serde_json::Error> {
        let handoff: BrowserAdapterHandoff = serde_json::from_str(
            r#"{
                "contract_version": "browser-adapter-v1",
                "launch_endpoint": null,
                "launchable": false,
                "handoff_fields": ["request_id", "guard_plan"],
                "launch_request_template": {
                    "request_id": "old-handoff-launch",
                    "adapter_id": null,
                    "contract_version": "browser-adapter-v1",
                    "requested_level": "network_suppressed",
                    "actor": "human",
                    "sensitivity": "sensitive",
                    "target_origins": ["https://bank.example"],
                    "credential_mode": "user_mediated",
                    "artifact_mode": "explicit_downloads",
                    "requested_controls": ["egress_policy"],
                    "guard_plan": {
                        "network": {
                            "allowed_origins": ["https://bank.example"],
                            "deny_private_networks": true,
                            "deny_localhost": true,
                            "deny_metadata_endpoints": true,
                            "require_dns_rebinding_protection": true,
                            "require_redirect_revalidation": true,
                            "require_proxy_enforcement": true,
                            "outbound_network_disabled_without_proxy": true
                        },
                        "credentials": {
                            "mode": "user_mediated",
                            "ambient_credentials_allowed": false,
                            "user_mediation_required": true,
                            "scoped_secret_channel_required": false
                        },
                        "storage": {
                            "mode": "explicit_downloads",
                            "plaintext_persistence_allowed": false,
                            "explicit_artifact_allowlist_required": true,
                            "encryption_required_for_persistence": true,
                            "teardown_proof_required": true
                        },
                        "required_runtime_guards": ["fresh guard plan"]
                    },
                    "required_completion_proofs": ["temporary profile directory removed"],
                    "same_user_capability_required": true,
                    "endpoint_network_policy_binding_required": true,
                    "notes": []
                },
                "required_completion_proofs": ["temporary profile directory removed"],
                "unavailable_reason": "old payload"
            }"#,
        )?;

        assert!(
            handoff
                .completion_proof_contract
                .iter()
                .any(|proof| proof.proof_id == "browser_process_terminated")
        );
        assert_eq!(
            handoff
                .launch_request_template
                .completion_report_template
                .request_id,
            "old-handoff-launch"
        );
        assert_eq!(
            handoff
                .launch_request_template
                .completion_report_template
                .adapter_id,
            "adapter-id-bound-at-registration"
        );
        assert_eq!(
            handoff.launch_request_template.issued_at,
            browser_adapter_launch_template_issued_at()
        );
        assert_eq!(
            handoff.launch_request_template.expires_at,
            browser_adapter_launch_template_expires_at()
        );
        assert!(handoff.launch_request_template.replay_protection_required);

        let contract: BrowserAdapterContract = serde_json::from_str(
            r#"{
                "version": "browser-adapter-v1",
                "status": "planned",
                "launch_endpoint": null,
                "handoff_fields": ["guard_plan"],
                "required_guard_fields": ["guard_plan.network.deny_metadata_endpoints"],
                "required_completion_proofs": ["temporary profile directory removed"],
                "unavailable_reason": "old payload"
            }"#,
        )?;
        assert!(
            contract
                .completion_proof_contract
                .iter()
                .any(|proof| proof.proof_id == "temporary_profile_removed")
        );
        Ok(())
    }

    #[test]
    fn browser_admission_request_defaults_additive_intent_fields() -> Result<(), serde_json::Error>
    {
        let request: BrowserAdmissionRequest = serde_json::from_str(
            r#"{
                "requested_level": "network_suppressed",
                "actor": "agent",
                "sensitivity": "sensitive"
            }"#,
        )?;
        assert_eq!(request.target_origins, Vec::<String>::new());
        assert_eq!(
            request.credential_mode,
            BrowserCredentialMode::NoCredentials
        );
        assert_eq!(request.artifact_mode, BrowserArtifactMode::Discard);
        assert_eq!(
            request.required_controls,
            Vec::<BrowserSandboxControl>::new()
        );
        assert!(!request.allow_downgrade);
        assert_eq!(request.task_label, None);
        assert!(
            serde_json::from_str::<BrowserAdmissionRequest>(
                r#"{
                    "requested_level": "network_suppressed",
                    "actor": "agent",
                    "sensitivity": "sensitive",
                    "ambient_cookies": true
                }"#
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn request_round_trips_with_wasm_source() -> Result<(), serde_json::Error> {
        let request = ExecuteRequest {
            lane: Lane::Wasm,
            source: Source::WasmWat {
                text: "(module)".to_string(),
            },
            entrypoint: Some("run".to_string()),
            input: serde_json::json!({"n": 10}),
            stdin: String::new(),
            policy: Policy::default(),
            idempotency_key: Some("step-1".to_string()),
        };
        let encoded = serde_json::to_value(&request)?;
        let decoded: ExecuteRequest = serde_json::from_value(encoded)?;
        assert_eq!(decoded, request);
        Ok(())
    }
}
