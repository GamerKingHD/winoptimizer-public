use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OperationId(pub String);

impl From<&str> for OperationId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationCategory {
    Cleanup,
    Startup,
    SystemTweak,
    Repair,
    AppManagement,
    SystemConfiguration,
    ProcessManagement,
    Maintenance,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    None,
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum ProofStatus {
    Proven,
    PartiallyProven,
    #[default]
    Experimental,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum ExposureLevel {
    Core,
    Supporting,
    Advanced,
    Labs,
    Internal,
    #[default]
    Legacy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum RecoveryLevel {
    Exact,
    BestEffort,
    #[default]
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivilegeRequirement {
    StandardUser,
    Administrator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reversibility {
    FullyReversible,
    BestEffort,
    NotReversible,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RebootPolicy {
    NotRequired,
    MayBeRequired,
    Required,
}

/// Declares whether an operation should use Windows System Restore as an
/// additional safety measure. This never replaces WinOptimizer's own undo or
/// recovery contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum RestorePointPolicy {
    /// Do not attempt to create a restore point for this operation.
    #[default]
    NotNeeded,
    /// A restore point may be created, but failure does not block mutation.
    Optional,
    /// Try to create a restore point and warn explicitly if it fails.
    Recommended,
    /// Mutation requires a successfully created restore point when the
    /// platform provides the restore-point facility.
    RequiredIfAvailable,
}

impl RestorePointPolicy {
    pub const ALL: [Self; 4] = [
        Self::NotNeeded,
        Self::Optional,
        Self::Recommended,
        Self::RequiredIfAvailable,
    ];

    /// Whether the execution path should attempt restore-point creation.
    pub const fn should_attempt(self) -> bool {
        !matches!(self, Self::NotNeeded)
    }

    /// Whether a failed attempt must prevent the system mutation.
    pub const fn blocks_on_failure(self) -> bool {
        matches!(self, Self::RequiredIfAvailable)
    }

    /// Whether a failed attempt requires an explicit user-facing warning.
    pub const fn warns_on_failure(self) -> bool {
        matches!(self, Self::Recommended | Self::RequiredIfAvailable)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationDescriptor {
    /// Stable operation identifier used for execution, retry, and lineage
    /// routing. This is intentionally distinct from `capability_id`.
    pub id: OperationId,
    /// Stable user-facing capability group, matching the capability manifest.
    #[serde(default)]
    pub capability_id: String,
    pub title: String,
    pub category: OperationCategory,
    pub risk: RiskLevel,
    pub privilege: PrivilegeRequirement,
    pub reversibility: Reversibility,
    pub reboot_policy: RebootPolicy,
    pub experimental: bool,
    #[serde(default)]
    pub proof_status: ProofStatus,
    #[serde(default)]
    pub exposure: ExposureLevel,
    #[serde(default)]
    pub recovery: RecoveryLevel,
    #[serde(default)]
    pub restore_point_policy: RestorePointPolicy,
    #[serde(default)]
    pub mutates_system: bool,
    #[serde(default)]
    pub requires_confirmation: bool,
    #[serde(default)]
    pub requires_revalidation: bool,
    #[serde(default)]
    pub requires_verification: bool,
    #[serde(default)]
    pub requires_journal: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedTarget {
    pub target_id: String,
    pub requested_action: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationWarning {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpectedEffect {
    pub target_id: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationPlan {
    pub id: OperationId,
    pub descriptor: OperationDescriptor,
    pub targets: Vec<PlannedTarget>,
    pub warnings: Vec<OperationWarning>,
    pub expected_effects: Vec<ExpectedEffect>,
    pub requires_elevation: bool,
    pub requires_restart: bool,
    pub reversibility: Reversibility,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_operation_id: Option<OperationId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_of_operation_id: Option<OperationId>,
}

impl OperationPlan {
    pub fn validate_relationships(&self) -> Result<(), OperationError> {
        if self.parent_operation_id.as_ref() == Some(&self.id)
            || self.retry_of_operation_id.as_ref() == Some(&self.id)
        {
            return Err(OperationError::validation(
                "an operation cannot be its own parent or retry source",
            ));
        }
        if self.retry_of_operation_id.is_some() && self.parent_operation_id.is_none() {
            return Err(OperationError::validation(
                "a retry plan must identify its parent operation",
            ));
        }
        if let (Some(parent), Some(retry_of)) =
            (&self.parent_operation_id, &self.retry_of_operation_id)
        {
            if parent != retry_of {
                return Err(OperationError::validation(
                    "a retry plan parent must match its retry source",
                ));
            }
        }
        Ok(())
    }

    /// Stable semantic identity used to bind durable confirmation and
    /// execution-start evidence to a plan.
    ///
    /// Deliberately hashes the operation identity and the ordered target
    /// identities/actions rather than a serialized byte representation:
    /// unrelated serialization or field-order changes must not make a
    /// previously authorized plan appear different, and a plan whose targets
    /// changed must not collide with the authorized one.
    pub fn identity_digest(&self) -> String {
        let mut hasher = blake3::Hasher::new();
        hasher.update(self.id.0.as_bytes());
        hasher.update(b"\x1f");
        hasher.update(self.descriptor.id.0.as_bytes());
        for target in &self.targets {
            hasher.update(b"\x1e");
            hasher.update(target.target_id.as_bytes());
            hasher.update(b"\x1f");
            hasher.update(target.requested_action.as_bytes());
        }
        hasher.finalize().to_hex().to_string()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateSnapshot {
    pub summary: String,
    pub properties: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StateEvidence {
    Observed { snapshot: StateSnapshot },
    Unavailable { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StateValue {
    Text { value: String },
    Unavailable { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateFieldDiff {
    pub field: String,
    pub before: StateValue,
    pub after: StateValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StateDiffStatus {
    Changed,
    Unchanged,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    Changed,
    Unchanged,
    Skipped,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationStatus {
    Verified,
    Unverified,
    NotApplicable,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationErrorCode {
    PermissionDenied,
    ElevationRequired,
    TargetNotFound,
    TargetChangedSincePreview,
    TargetProtected,
    ValidationFailed,
    OperationCancelled,
    OperationTimedOut,
    OperationInterrupted,
    ExecutionFailed,
    VerificationFailed,
    PartialFailure,
    UnsupportedWindowsVersion,
    RebootRequired,
    SerializationFailed,
    InternalError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationErrorCategory {
    Authorization,
    Target,
    Validation,
    Cancellation,
    Execution,
    Verification,
    Platform,
    Serialization,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationErrorPhase {
    Planning,
    Revalidation,
    Execution,
    Verification,
    Cancellation,
    Unknown,
}

impl OperationErrorCode {
    pub fn category(self) -> OperationErrorCategory {
        match self {
            Self::PermissionDenied | Self::ElevationRequired => {
                OperationErrorCategory::Authorization
            }
            Self::TargetNotFound | Self::TargetChangedSincePreview | Self::TargetProtected => {
                OperationErrorCategory::Target
            }
            Self::ValidationFailed => OperationErrorCategory::Validation,
            Self::OperationCancelled | Self::OperationTimedOut | Self::OperationInterrupted => {
                OperationErrorCategory::Cancellation
            }
            Self::ExecutionFailed | Self::PartialFailure => OperationErrorCategory::Execution,
            Self::VerificationFailed => OperationErrorCategory::Verification,
            Self::UnsupportedWindowsVersion | Self::RebootRequired => {
                OperationErrorCategory::Platform
            }
            Self::SerializationFailed => OperationErrorCategory::Serialization,
            Self::InternalError => OperationErrorCategory::Internal,
        }
    }

    pub fn phase(self) -> OperationErrorPhase {
        match self {
            Self::ValidationFailed | Self::TargetNotFound | Self::UnsupportedWindowsVersion => {
                OperationErrorPhase::Planning
            }
            Self::TargetChangedSincePreview | Self::TargetProtected => {
                OperationErrorPhase::Revalidation
            }
            Self::OperationCancelled | Self::OperationTimedOut | Self::OperationInterrupted => {
                OperationErrorPhase::Cancellation
            }
            Self::VerificationFailed => OperationErrorPhase::Verification,
            Self::PermissionDenied
            | Self::ElevationRequired
            | Self::ExecutionFailed
            | Self::PartialFailure
            | Self::RebootRequired
            | Self::SerializationFailed
            | Self::InternalError => OperationErrorPhase::Execution,
        }
    }

    pub fn requires_elevation(self) -> bool {
        matches!(self, Self::ElevationRequired)
    }

    pub fn requires_restart(self) -> bool {
        matches!(self, Self::RebootRequired)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct OperationError {
    pub code: OperationErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
    pub retryable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OperationErrorContext {
    pub operation_id: Option<OperationId>,
    pub target_id: Option<String>,
    pub phase: Option<OperationErrorPhase>,
    pub native_error_code: Option<u32>,
    pub diagnostic_context: BTreeMap<String, String>,
    pub requires_elevation: Option<bool>,
    pub requires_restart: Option<bool>,
}

impl OperationErrorContext {
    pub fn for_plan(plan: &OperationPlan) -> Self {
        Self {
            operation_id: Some(plan.id.clone()),
            requires_elevation: Some(plan.requires_elevation),
            requires_restart: Some(plan.requires_restart),
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextualOperationError {
    pub error: OperationError,
    pub context: OperationErrorContext,
}

impl ContextualOperationError {
    pub fn new(error: OperationError, context: OperationErrorContext) -> Self {
        Self { error, context }
    }
}

impl fmt::Display for ContextualOperationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.error.fmt(formatter)
    }
}

impl std::error::Error for ContextualOperationError {}

#[derive(Serialize)]
struct OperationErrorWire<'a> {
    code: OperationErrorCode,
    category: OperationErrorCategory,
    operation_id: Option<&'a OperationId>,
    target_id: Option<&'a String>,
    phase: OperationErrorPhase,
    native_error_code: Option<u32>,
    message: &'a str,
    human_message: &'a str,
    retryable: bool,
    requires_elevation: bool,
    requires_restart: bool,
    diagnostic_context: &'a BTreeMap<String, String>,
}

impl Serialize for OperationError {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let diagnostic_context = BTreeMap::new();
        OperationErrorWire {
            code: self.code,
            category: self.code.category(),
            operation_id: None,
            target_id: self.target_id.as_ref(),
            phase: self.code.phase(),
            native_error_code: None,
            message: &self.message,
            human_message: &self.message,
            retryable: self.retryable,
            requires_elevation: self.code.requires_elevation(),
            requires_restart: self.code.requires_restart(),
            diagnostic_context: &diagnostic_context,
        }
        .serialize(serializer)
    }
}

impl Serialize for ContextualOperationError {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let target_id = self
            .context
            .target_id
            .as_ref()
            .or(self.error.target_id.as_ref());
        OperationErrorWire {
            code: self.error.code,
            category: self.error.code.category(),
            operation_id: self.context.operation_id.as_ref(),
            target_id,
            phase: self
                .context
                .phase
                .unwrap_or_else(|| self.error.code.phase()),
            native_error_code: self.context.native_error_code,
            message: &self.error.message,
            human_message: &self.error.message,
            retryable: self.error.retryable,
            requires_elevation: self
                .context
                .requires_elevation
                .unwrap_or_else(|| self.error.code.requires_elevation()),
            requires_restart: self
                .context
                .requires_restart
                .unwrap_or_else(|| self.error.code.requires_restart()),
            diagnostic_context: &self.context.diagnostic_context,
        }
        .serialize(serializer)
    }
}

impl OperationError {
    pub fn with_context(self, context: OperationErrorContext) -> ContextualOperationError {
        ContextualOperationError::new(self, context)
    }

    pub fn validation(message: impl Into<String>) -> Self {
        Self {
            code: OperationErrorCode::ValidationFailed,
            message: message.into(),
            target_id: None,
            retryable: false,
        }
    }
}

impl RestorePointPolicy {
    /// Validate that a restore-point policy is meaningful for the operation.
    /// Read-only operations must not request restore-point creation.
    pub fn validate_for_operation(self, mutates_system: bool) -> Result<(), OperationError> {
        if !mutates_system && self.should_attempt() {
            return Err(OperationError::validation(
                "read-only operation cannot request a restore point",
            ));
        }
        Ok(())
    }

    /// Validate the result of a restore-point attempt before mutation starts.
    /// Optional and recommended policies are deliberately non-blocking.
    pub fn validate_creation(self, created: bool) -> Result<(), OperationError> {
        if self.blocks_on_failure() && !created {
            return Err(OperationError {
                code: OperationErrorCode::ExecutionFailed,
                message: "required restore point could not be created; mutation is blocked".into(),
                target_id: None,
                retryable: true,
            });
        }
        Ok(())
    }
}

impl OperationDescriptor {
    pub fn validate_contract(&self) -> Result<(), OperationError> {
        self.restore_point_policy
            .validate_for_operation(self.mutates_system)?;
        if self.capability_id.trim().is_empty() {
            return Err(OperationError::validation(
                "operation descriptor is missing a capability id",
            ));
        }
        if self.mutates_system
            && (!self.requires_confirmation
                || !self.requires_revalidation
                || !self.requires_verification
                || !self.requires_journal)
        {
            return Err(OperationError::validation(
                "mutating operation descriptor is missing a mandatory safety gate",
            ));
        }
        if !self.mutates_system
            && (self.requires_confirmation
                || self.requires_revalidation
                || self.requires_verification
                || self.requires_journal)
        {
            return Err(OperationError::validation(
                "read-only operation descriptor cannot require mutation gates",
            ));
        }
        Ok(())
    }

    /// Validate an explicitly selected restore-point policy without changing
    /// the descriptor wire shape or creating a restore point.
    pub fn validate_restore_point_policy(
        &self,
        policy: RestorePointPolicy,
    ) -> Result<(), OperationError> {
        policy.validate_for_operation(self.mutates_system)
    }
}

impl fmt::Display for OperationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.message)
    }
}

impl std::error::Error for OperationError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetOutcome {
    pub target_id: String,
    pub requested_action: String,
    pub before: Option<StateSnapshot>,
    pub after: Option<StateSnapshot>,
    pub execution: ExecutionStatus,
    pub verification: VerificationStatus,
    pub error: Option<OperationError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetStateDiff {
    pub target_id: String,
    pub requested_action: String,
    pub before: StateEvidence,
    pub after: StateEvidence,
    pub status: StateDiffStatus,
    #[serde(default)]
    pub summary: Option<StateFieldDiff>,
    #[serde(default)]
    pub properties: Vec<StateFieldDiff>,
}

impl TargetOutcome {
    /// Derive a diff only from snapshots this outcome actually contains.
    /// `None` is intentionally unavailable rather than an inferred state.
    pub fn state_diff(&self) -> TargetStateDiff {
        TargetStateDiff::from_outcome(self)
    }
}

impl TargetStateDiff {
    pub fn from_outcome(outcome: &TargetOutcome) -> Self {
        let before = evidence(outcome.before.as_ref(), "before");
        let after = evidence(outcome.after.as_ref(), "after");
        let mut unknown = outcome.before.is_none() || outcome.after.is_none();
        let mut summary = None;
        let mut properties = Vec::new();

        if let (Some(before), Some(after)) = (&outcome.before, &outcome.after) {
            if before.summary != after.summary {
                summary = Some(StateFieldDiff {
                    field: "summary".into(),
                    before: StateValue::Text {
                        value: before.summary.clone(),
                    },
                    after: StateValue::Text {
                        value: after.summary.clone(),
                    },
                });
            }

            let fields: BTreeSet<String> = before
                .properties
                .keys()
                .chain(after.properties.keys())
                .cloned()
                .collect();
            for field in fields {
                let before_value = before.properties.get(&field);
                let after_value = after.properties.get(&field);
                if before_value == after_value {
                    continue;
                }
                if before_value.is_none() || after_value.is_none() {
                    unknown = true;
                }
                properties.push(StateFieldDiff {
                    field: field.clone(),
                    before: field_value(before_value, "before", &field),
                    after: field_value(after_value, "after", &field),
                });
            }
        }

        let status = if unknown {
            StateDiffStatus::Unknown
        } else if summary.is_some() || !properties.is_empty() {
            StateDiffStatus::Changed
        } else {
            StateDiffStatus::Unchanged
        };

        TargetStateDiff {
            target_id: outcome.target_id.clone(),
            requested_action: outcome.requested_action.clone(),
            before,
            after,
            status,
            summary,
            properties,
        }
    }
}

fn evidence(snapshot: Option<&StateSnapshot>, side: &str) -> StateEvidence {
    match snapshot {
        Some(snapshot) => StateEvidence::Observed {
            snapshot: snapshot.clone(),
        },
        None => StateEvidence::Unavailable {
            reason: format!(
                "{side}-state snapshot was not captured; the current outcome does not distinguish an absent state from unavailable evidence"
            ),
        },
    }
}

fn field_value(value: Option<&String>, side: &str, field: &str) -> StateValue {
    match value {
        Some(value) => StateValue::Text {
            value: value.clone(),
        },
        None => StateValue::Unavailable {
            reason: format!("{side}-state did not capture the `{field}` field"),
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UndoAvailability {
    UndoAvailable,
    NotReversible,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationResult {
    pub operation_id: OperationId,
    pub requested: usize,
    pub changed: usize,
    pub unchanged: usize,
    pub skipped: usize,
    pub failed: usize,
    pub verified: usize,
    pub unverified: usize,
    pub outcomes: Vec<TargetOutcome>,
    pub reboot_required: bool,
    pub undo: UndoAvailability,
}

impl OperationResult {
    pub fn new(
        operation_id: OperationId,
        outcomes: Vec<TargetOutcome>,
        reboot_required: bool,
        undo: UndoAvailability,
    ) -> Result<Self, OperationError> {
        if outcomes.is_empty() {
            return Err(OperationError::validation(
                "an operation result must contain at least one target outcome",
            ));
        }

        let result = Self {
            operation_id,
            requested: outcomes.len(),
            changed: count_execution(&outcomes, ExecutionStatus::Changed),
            unchanged: count_execution(&outcomes, ExecutionStatus::Unchanged),
            skipped: count_execution(&outcomes, ExecutionStatus::Skipped),
            failed: count_execution(&outcomes, ExecutionStatus::Failed),
            verified: count_verification(&outcomes, VerificationStatus::Verified),
            unverified: outcomes.len()
                - count_verification(&outcomes, VerificationStatus::Verified),
            outcomes,
            reboot_required,
            undo,
        };
        result.validate()?;
        Ok(result)
    }

    pub fn validate(&self) -> Result<(), OperationError> {
        if self.requested == 0 {
            return Err(OperationError::validation(
                "an operation result must contain at least one target outcome",
            ));
        }
        if self.requested != self.changed + self.unchanged + self.skipped + self.failed {
            return Err(OperationError::validation(
                "operation target counts do not reconcile",
            ));
        }
        if self.requested != self.outcomes.len() {
            return Err(OperationError::validation(
                "requested count does not match per-target outcomes",
            ));
        }
        let target_ids: BTreeSet<&str> = self
            .outcomes
            .iter()
            .map(|outcome| outcome.target_id.as_str())
            .collect();
        if target_ids.len() != self.outcomes.len() {
            return Err(OperationError::validation(
                "operation result contains duplicate target outcomes",
            ));
        }
        if self.requested != self.verified + self.unverified {
            return Err(OperationError::validation(
                "verification counts do not reconcile",
            ));
        }
        if self.changed != count_execution(&self.outcomes, ExecutionStatus::Changed)
            || self.unchanged != count_execution(&self.outcomes, ExecutionStatus::Unchanged)
            || self.skipped != count_execution(&self.outcomes, ExecutionStatus::Skipped)
            || self.failed != count_execution(&self.outcomes, ExecutionStatus::Failed)
        {
            return Err(OperationError::validation(
                "operation counts do not match per-target outcomes",
            ));
        }
        if self.verified != count_verification(&self.outcomes, VerificationStatus::Verified) {
            return Err(OperationError::validation(
                "verification counts do not match per-target outcomes",
            ));
        }
        if self
            .outcomes
            .iter()
            .any(|outcome| outcome.target_id.is_empty())
        {
            return Err(OperationError::validation(
                "target outcomes require a target id",
            ));
        }
        if self.outcomes.iter().any(|outcome| {
            outcome.error.as_ref().is_some_and(|error| {
                error.code == OperationErrorCode::OperationCancelled
                    && (outcome.execution != ExecutionStatus::Skipped
                        || outcome.verification != VerificationStatus::Failed)
            })
        }) {
            return Err(OperationError::validation(
                "a cancelled target must be skipped and unverified",
            ));
        }
        Ok(())
    }

    pub fn completion_state(&self) -> OperationState {
        let successful = self
            .outcomes
            .iter()
            .filter(|outcome| {
                matches!(
                    outcome.execution,
                    ExecutionStatus::Changed | ExecutionStatus::Unchanged
                ) && outcome.verification == VerificationStatus::Verified
            })
            .count();

        if successful == self.requested {
            OperationState::Succeeded
        } else if successful > 0 {
            OperationState::PartiallySucceeded
        } else if self.outcomes.iter().all(|outcome| {
            outcome
                .error
                .as_ref()
                .is_some_and(|error| error.code == OperationErrorCode::OperationCancelled)
        }) {
            OperationState::Cancelled
        } else {
            OperationState::Failed
        }
    }
}

#[derive(Serialize)]
struct OperationResultRef<'a> {
    operation_id: &'a OperationId,
    requested: usize,
    changed: usize,
    unchanged: usize,
    skipped: usize,
    failed: usize,
    verified: usize,
    unverified: usize,
    outcomes: &'a [TargetOutcome],
    reboot_required: bool,
    undo: UndoAvailability,
}

impl Serialize for OperationResult {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::Error;

        self.validate().map_err(S::Error::custom)?;
        OperationResultRef {
            operation_id: &self.operation_id,
            requested: self.requested,
            changed: self.changed,
            unchanged: self.unchanged,
            skipped: self.skipped,
            failed: self.failed,
            verified: self.verified,
            unverified: self.unverified,
            outcomes: &self.outcomes,
            reboot_required: self.reboot_required,
            undo: self.undo,
        }
        .serialize(serializer)
    }
}

#[derive(Deserialize)]
struct OperationResultWire {
    operation_id: OperationId,
    requested: usize,
    changed: usize,
    unchanged: usize,
    skipped: usize,
    failed: usize,
    verified: usize,
    unverified: usize,
    outcomes: Vec<TargetOutcome>,
    reboot_required: bool,
    undo: UndoAvailability,
}

impl<'de> Deserialize<'de> for OperationResult {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error;

        let wire = OperationResultWire::deserialize(deserializer)?;
        let result = Self {
            operation_id: wire.operation_id,
            requested: wire.requested,
            changed: wire.changed,
            unchanged: wire.unchanged,
            skipped: wire.skipped,
            failed: wire.failed,
            verified: wire.verified,
            unverified: wire.unverified,
            outcomes: wire.outcomes,
            reboot_required: wire.reboot_required,
            undo: wire.undo,
        };
        result.validate().map_err(D::Error::custom)?;
        Ok(result)
    }
}

fn count_execution(outcomes: &[TargetOutcome], status: ExecutionStatus) -> usize {
    outcomes
        .iter()
        .filter(|outcome| outcome.execution == status)
        .count()
}

fn count_verification(outcomes: &[TargetOutcome], status: VerificationStatus) -> usize {
    outcomes
        .iter()
        .filter(|outcome| outcome.verification == status)
        .count()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationState {
    Idle,
    Previewing,
    AwaitingConfirmation,
    AwaitingElevation,
    Running,
    Verifying,
    Succeeded,
    PartiallySucceeded,
    Failed,
    UndoAvailable,
    NotReversible,
    Cancelled,
    Interrupted,
}

/// Durable execution states used to distinguish an unverified interruption
/// from a terminal operation result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationExecutionState {
    Planned,
    AwaitingConfirmation,
    Confirmed,
    ExecutionStarting,
    Executing,
    MutationCompleted,
    Verifying,
    Succeeded,
    PartiallySucceeded,
    Failed,
    OutcomeUnknown,
    CancelledBeforeMutation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetExecutionState {
    Pending,
    RevalidationFailed,
    Ready,
    MutationStarted,
    Changed,
    Unchanged,
    Skipped,
    VerificationSucceeded,
    VerificationFailed,
    Failed,
    OutcomeUnknown,
}

impl OperationExecutionState {
    /// Legal durable lifecycle graph for an operation. `record_execution_state`
    /// and the journal's internal event writer reject any edge not listed here.
    /// A missing prior state is a bootstrap (first event) and is allowed.
    pub fn can_transition_to(self, next: Self) -> bool {
        use OperationExecutionState::*;
        if self == next {
            return true;
        }
        match self {
            Planned => matches!(
                next,
                AwaitingConfirmation
                    | Confirmed
                    | ExecutionStarting
                    | CancelledBeforeMutation
                    | OutcomeUnknown
            ),
            AwaitingConfirmation => matches!(
                next,
                Confirmed | ExecutionStarting | CancelledBeforeMutation | OutcomeUnknown
            ),
            Confirmed => matches!(
                next,
                ExecutionStarting | CancelledBeforeMutation | OutcomeUnknown
            ),
            ExecutionStarting => matches!(
                next,
                Executing | MutationCompleted | Failed | OutcomeUnknown | CancelledBeforeMutation
            ),
            Executing => matches!(
                next,
                MutationCompleted | Failed | OutcomeUnknown | CancelledBeforeMutation
            ),
            MutationCompleted => matches!(
                next,
                Verifying | Succeeded | PartiallySucceeded | Failed | OutcomeUnknown
            ),
            Verifying => matches!(
                next,
                Succeeded | PartiallySucceeded | Failed | OutcomeUnknown
            ),
            Succeeded | PartiallySucceeded | Failed | OutcomeUnknown | CancelledBeforeMutation => {
                false
            }
        }
    }

    /// Pre-execution states exist before any target mutation could have begun.
    pub fn is_pre_execution(self) -> bool {
        matches!(
            self,
            Self::Planned | Self::AwaitingConfirmation | Self::Confirmed
        )
    }

    /// Ambiguous states may be left behind when a process disappears mid-flight
    /// without a terminal record. These are the only states startup
    /// reconciliation may convert to `OutcomeUnknown`.
    pub fn is_ambiguous(self) -> bool {
        matches!(
            self,
            Self::ExecutionStarting | Self::Executing | Self::MutationCompleted | Self::Verifying
        )
    }

    /// Terminal states are durably recorded final outcomes. Reconciliation
    /// must never overwrite them.
    pub fn is_terminal(self) -> bool {
        !self.is_pre_execution() && !self.is_ambiguous()
    }
}

impl TargetExecutionState {
    /// Ambiguous states are the only target states genuinely incomplete across
    /// a crash boundary. Startup reconciliation converts these to
    /// `OutcomeUnknown`; every other state is a durable observation and must be
    /// preserved.
    pub fn is_ambiguous(self) -> bool {
        matches!(self, Self::Pending | Self::Ready | Self::MutationStarted)
    }

    pub fn is_terminal(self) -> bool {
        !self.is_ambiguous()
    }
}

impl OperationState {
    pub const ALL: [Self; 13] = [
        Self::Idle,
        Self::Previewing,
        Self::AwaitingConfirmation,
        Self::AwaitingElevation,
        Self::Running,
        Self::Verifying,
        Self::Succeeded,
        Self::PartiallySucceeded,
        Self::Failed,
        Self::UndoAvailable,
        Self::NotReversible,
        Self::Cancelled,
        Self::Interrupted,
    ];

    pub fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Idle, Self::Previewing)
                | (
                    Self::Previewing,
                    Self::AwaitingConfirmation | Self::Cancelled | Self::Interrupted
                )
                | (
                    Self::AwaitingConfirmation,
                    Self::AwaitingElevation | Self::Running | Self::Cancelled | Self::Interrupted
                )
                | (
                    Self::AwaitingElevation,
                    Self::Running | Self::Cancelled | Self::Interrupted
                )
                | (
                    Self::Running,
                    Self::Verifying | Self::Cancelled | Self::Interrupted
                )
                | (
                    Self::Verifying,
                    Self::Succeeded | Self::PartiallySucceeded | Self::Failed | Self::Interrupted
                )
                | (
                    Self::Succeeded | Self::PartiallySucceeded | Self::Failed,
                    Self::UndoAvailable | Self::NotReversible
                )
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationStateMachine {
    pub current: OperationState,
    pub history: Vec<OperationState>,
}

impl Default for OperationStateMachine {
    fn default() -> Self {
        Self {
            current: OperationState::Idle,
            history: vec![OperationState::Idle],
        }
    }
}

impl OperationStateMachine {
    pub fn transition(&mut self, next: OperationState) -> Result<(), OperationError> {
        if !self.current.can_transition_to(next) {
            return Err(OperationError::validation(format!(
                "forbidden operation transition: {:?} -> {:?}",
                self.current, next
            )));
        }
        self.current = next;
        self.history.push(next);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyntheticOperationContract {
    pub descriptor: OperationDescriptor,
    pub plan: OperationPlan,
    pub traces: Vec<Vec<OperationState>>,
    pub result: OperationResult,
    pub sample_error: OperationError,
}

pub fn synthetic_operation_contract() -> Result<SyntheticOperationContract, OperationError> {
    let descriptor = OperationDescriptor {
        id: OperationId::from("synthetic.operation-kernel"),
        capability_id: "operation.kernel".into(),
        title: "Synthetic operation kernel check".into(),
        category: OperationCategory::Other,
        risk: RiskLevel::None,
        privilege: PrivilegeRequirement::Administrator,
        reversibility: Reversibility::FullyReversible,
        reboot_policy: RebootPolicy::NotRequired,
        experimental: true,
        proof_status: ProofStatus::Proven,
        exposure: ExposureLevel::Internal,
        recovery: RecoveryLevel::None,
        restore_point_policy: RestorePointPolicy::NotNeeded,
        mutates_system: false,
        requires_confirmation: false,
        requires_revalidation: false,
        requires_verification: false,
        requires_journal: false,
    };
    let targets = vec![
        PlannedTarget {
            target_id: "synthetic.changed".into(),
            requested_action: "set".into(),
            description: "Synthetic verified change".into(),
        },
        PlannedTarget {
            target_id: "synthetic.failed".into(),
            requested_action: "set".into(),
            description: "Synthetic failed change".into(),
        },
    ];
    let plan = OperationPlan {
        id: descriptor.id.clone(),
        descriptor: descriptor.clone(),
        targets,
        warnings: vec![OperationWarning {
            code: "synthetic_only".into(),
            message: "This operation does not change Windows.".into(),
        }],
        expected_effects: vec![ExpectedEffect {
            target_id: "synthetic.changed".into(),
            description: "The synthetic value changes from before to after.".into(),
        }],
        requires_elevation: true,
        requires_restart: false,
        reversibility: Reversibility::FullyReversible,
        parent_operation_id: None,
        retry_of_operation_id: None,
    };

    let trace_targets = [
        &[
            OperationState::Previewing,
            OperationState::AwaitingConfirmation,
            OperationState::AwaitingElevation,
            OperationState::Running,
            OperationState::Verifying,
            OperationState::Succeeded,
            OperationState::UndoAvailable,
        ][..],
        &[
            OperationState::Previewing,
            OperationState::AwaitingConfirmation,
            OperationState::Running,
            OperationState::Verifying,
            OperationState::PartiallySucceeded,
            OperationState::NotReversible,
        ][..],
        &[
            OperationState::Previewing,
            OperationState::AwaitingConfirmation,
            OperationState::Running,
            OperationState::Verifying,
            OperationState::Failed,
            OperationState::NotReversible,
        ][..],
        &[OperationState::Previewing, OperationState::Cancelled][..],
        &[
            OperationState::Previewing,
            OperationState::AwaitingConfirmation,
            OperationState::Running,
            OperationState::Interrupted,
        ][..],
    ];
    let mut traces = Vec::with_capacity(trace_targets.len());
    for targets in trace_targets {
        let mut machine = OperationStateMachine::default();
        for state in targets {
            machine.transition(*state)?;
        }
        traces.push(machine.history);
    }

    let sample_error = OperationError {
        code: OperationErrorCode::TargetChangedSincePreview,
        message: "Synthetic target changed after preview.".into(),
        target_id: Some("synthetic.failed".into()),
        retryable: true,
    };
    let result = OperationResult::new(
        descriptor.id.clone(),
        vec![
            TargetOutcome {
                target_id: "synthetic.changed".into(),
                requested_action: "set".into(),
                before: Some(StateSnapshot {
                    summary: "before".into(),
                    properties: BTreeMap::from([("value".into(), "0".into())]),
                }),
                after: Some(StateSnapshot {
                    summary: "after".into(),
                    properties: BTreeMap::from([("value".into(), "1".into())]),
                }),
                execution: ExecutionStatus::Changed,
                verification: VerificationStatus::Verified,
                error: None,
            },
            TargetOutcome {
                target_id: "synthetic.failed".into(),
                requested_action: "set".into(),
                before: None,
                after: None,
                execution: ExecutionStatus::Failed,
                verification: VerificationStatus::Failed,
                error: Some(sample_error.clone()),
            },
        ],
        false,
        UndoAvailability::UndoAvailable,
    )?;

    Ok(SyntheticOperationContract {
        descriptor,
        plan,
        traces,
        result,
        sample_error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn allowed_and_forbidden_transitions_are_enforced() {
        let allowed = HashSet::from([
            (OperationState::Idle, OperationState::Previewing),
            (
                OperationState::Previewing,
                OperationState::AwaitingConfirmation,
            ),
            (OperationState::Previewing, OperationState::Cancelled),
            (OperationState::Previewing, OperationState::Interrupted),
            (
                OperationState::AwaitingConfirmation,
                OperationState::AwaitingElevation,
            ),
            (
                OperationState::AwaitingConfirmation,
                OperationState::Running,
            ),
            (
                OperationState::AwaitingConfirmation,
                OperationState::Cancelled,
            ),
            (
                OperationState::AwaitingConfirmation,
                OperationState::Interrupted,
            ),
            (OperationState::AwaitingElevation, OperationState::Running),
            (OperationState::AwaitingElevation, OperationState::Cancelled),
            (
                OperationState::AwaitingElevation,
                OperationState::Interrupted,
            ),
            (OperationState::Running, OperationState::Verifying),
            (OperationState::Running, OperationState::Cancelled),
            (OperationState::Running, OperationState::Interrupted),
            (OperationState::Verifying, OperationState::Succeeded),
            (
                OperationState::Verifying,
                OperationState::PartiallySucceeded,
            ),
            (OperationState::Verifying, OperationState::Failed),
            (OperationState::Verifying, OperationState::Interrupted),
            (OperationState::Succeeded, OperationState::UndoAvailable),
            (OperationState::Succeeded, OperationState::NotReversible),
            (
                OperationState::PartiallySucceeded,
                OperationState::UndoAvailable,
            ),
            (
                OperationState::PartiallySucceeded,
                OperationState::NotReversible,
            ),
            (OperationState::Failed, OperationState::UndoAvailable),
            (OperationState::Failed, OperationState::NotReversible),
        ]);

        for current in OperationState::ALL {
            for next in OperationState::ALL {
                assert_eq!(
                    current.can_transition_to(next),
                    allowed.contains(&(current, next)),
                    "{current:?} -> {next:?}"
                );
            }
        }
    }

    #[test]
    fn synthetic_operation_traces_cover_every_state() {
        let contract = synthetic_operation_contract().unwrap();
        let visited: HashSet<_> = contract.traces.into_iter().flatten().collect();
        assert_eq!(visited, HashSet::from(OperationState::ALL));
    }

    #[test]
    fn descriptor_contract_rejects_missing_mutation_gates() {
        let mut descriptor = synthetic_operation_contract().unwrap().descriptor;
        descriptor.mutates_system = true;
        descriptor.requires_journal = false;
        assert!(descriptor.validate_contract().is_err());

        descriptor.mutates_system = false;
        descriptor.requires_journal = true;
        assert!(descriptor.validate_contract().is_err());
    }

    #[test]
    fn legacy_descriptor_payload_uses_conservative_metadata_defaults() {
        let descriptor = serde_json::from_value::<OperationDescriptor>(serde_json::json!({
            "id": "legacy.operation",
            "title": "Legacy operation",
            "category": "other",
            "risk": "low",
            "privilege": "standard_user",
            "reversibility": "not_reversible",
            "reboot_policy": "not_required",
            "experimental": true
        }))
        .unwrap();

        assert!(descriptor.capability_id.is_empty());
        assert_eq!(descriptor.proof_status, ProofStatus::Experimental);
        assert_eq!(descriptor.exposure, ExposureLevel::Legacy);
        assert_eq!(descriptor.recovery, RecoveryLevel::None);
        assert_eq!(
            descriptor.restore_point_policy,
            RestorePointPolicy::NotNeeded
        );
        assert!(!descriptor.mutates_system);
        assert!(!descriptor.requires_confirmation);
        assert!(!descriptor.requires_revalidation);
        assert!(!descriptor.requires_verification);
        assert!(!descriptor.requires_journal);
    }

    #[test]
    fn restore_point_policy_has_safe_serde_default_and_wire_names() {
        #[derive(serde::Deserialize)]
        struct PolicyPayload {
            #[serde(default)]
            policy: RestorePointPolicy,
        }

        assert_eq!(RestorePointPolicy::default(), RestorePointPolicy::NotNeeded);
        assert_eq!(
            serde_json::from_value::<PolicyPayload>(serde_json::json!({}))
                .unwrap()
                .policy,
            RestorePointPolicy::NotNeeded
        );

        for (policy, expected) in RestorePointPolicy::ALL.into_iter().zip([
            "not_needed",
            "optional",
            "recommended",
            "required_if_available",
        ]) {
            let json = serde_json::to_value(policy).unwrap();
            let decoded = serde_json::from_value::<RestorePointPolicy>(json.clone()).unwrap();
            assert_eq!(decoded, policy);
            assert_eq!(json, serde_json::json!(expected));
        }
    }

    #[test]
    fn restore_point_policy_helpers_keep_failure_behavior_explicit() {
        assert!(!RestorePointPolicy::NotNeeded.should_attempt());
        assert!(RestorePointPolicy::Optional.should_attempt());
        assert!(RestorePointPolicy::Recommended.warns_on_failure());
        assert!(RestorePointPolicy::RequiredIfAvailable.blocks_on_failure());

        assert!(RestorePointPolicy::Recommended
            .validate_creation(false)
            .is_ok());
        let error = RestorePointPolicy::RequiredIfAvailable
            .validate_creation(false)
            .unwrap_err();
        assert_eq!(error.code, OperationErrorCode::ExecutionFailed);
        assert!(error.message.contains("mutation is blocked"));
    }

    #[test]
    fn restore_point_policy_rejects_read_only_requests() {
        let descriptor = synthetic_operation_contract().unwrap().descriptor;
        assert!(descriptor
            .validate_restore_point_policy(RestorePointPolicy::NotNeeded)
            .is_ok());
        assert!(descriptor
            .validate_restore_point_policy(RestorePointPolicy::Recommended)
            .is_err());

        assert!(RestorePointPolicy::RequiredIfAvailable
            .validate_for_operation(true)
            .is_ok());
    }

    #[test]
    fn partial_result_cannot_report_full_success() {
        let contract = synthetic_operation_contract().unwrap();
        assert_eq!(
            contract.result.completion_state(),
            OperationState::PartiallySucceeded
        );
        assert_eq!(contract.result.outcomes.len(), contract.result.requested);
    }

    #[test]
    fn retry_relationship_requires_matching_parent_and_source() {
        let mut plan = synthetic_operation_contract().unwrap().plan;
        plan.parent_operation_id = Some(OperationId::from("parent-operation"));
        plan.retry_of_operation_id = Some(OperationId::from("different-source"));

        let error = plan.validate_relationships().unwrap_err();
        assert!(error.message.contains("parent must match its retry source"));
    }

    #[test]
    fn cancelled_target_cannot_claim_a_change_or_verified_state() {
        let error = OperationResult::new(
            OperationId::from("invalid-cancellation"),
            vec![TargetOutcome {
                target_id: "cancelled.target".into(),
                requested_action: "remove".into(),
                before: None,
                after: None,
                execution: ExecutionStatus::Changed,
                verification: VerificationStatus::Verified,
                error: Some(OperationError {
                    code: OperationErrorCode::OperationCancelled,
                    message: "cancelled after target selection".into(),
                    target_id: Some("cancelled.target".into()),
                    retryable: true,
                }),
            }],
            false,
            UndoAvailability::NotReversible,
        )
        .unwrap_err();

        assert!(error.message.contains("cancelled target must be skipped"));
    }

    #[test]
    fn all_cancelled_targets_report_cancelled() {
        let result = OperationResult::new(
            OperationId::from("cancelled.operation"),
            vec![TargetOutcome {
                target_id: "cancelled.target".into(),
                requested_action: "remove".into(),
                before: None,
                after: None,
                execution: ExecutionStatus::Skipped,
                verification: VerificationStatus::Failed,
                error: Some(OperationError {
                    code: OperationErrorCode::OperationCancelled,
                    message: "cancelled before target".into(),
                    target_id: Some("cancelled.target".into()),
                    retryable: true,
                }),
            }],
            false,
            UndoAvailability::NotReversible,
        )
        .unwrap();

        assert_eq!(result.completion_state(), OperationState::Cancelled);
    }

    #[test]
    fn completed_targets_keep_partial_state_when_remaining_targets_are_cancelled() {
        let result = OperationResult::new(
            OperationId::from("partially-cancelled.operation"),
            vec![
                TargetOutcome {
                    target_id: "changed.target".into(),
                    requested_action: "remove".into(),
                    before: None,
                    after: None,
                    execution: ExecutionStatus::Changed,
                    verification: VerificationStatus::Verified,
                    error: None,
                },
                TargetOutcome {
                    target_id: "cancelled.target".into(),
                    requested_action: "remove".into(),
                    before: None,
                    after: None,
                    execution: ExecutionStatus::Skipped,
                    verification: VerificationStatus::Failed,
                    error: Some(OperationError {
                        code: OperationErrorCode::OperationCancelled,
                        message: "cancelled before target".into(),
                        target_id: Some("cancelled.target".into()),
                        retryable: true,
                    }),
                },
            ],
            false,
            UndoAvailability::NotReversible,
        )
        .unwrap();

        assert_eq!(
            result.completion_state(),
            OperationState::PartiallySucceeded
        );
    }

    #[test]
    fn invalid_serialized_result_is_rejected() {
        let contract = synthetic_operation_contract().unwrap();
        let mut invalid_outbound = contract.result.clone();
        invalid_outbound.failed = 0;
        assert!(serde_json::to_value(invalid_outbound).is_err());

        let mut json = serde_json::to_value(contract.result).unwrap();
        json["requested"] = serde_json::json!(3);
        let error = serde_json::from_value::<OperationResult>(json).unwrap_err();
        assert!(error.to_string().contains("do not reconcile"));
    }

    #[test]
    fn structured_error_survives_serialization() {
        let error = synthetic_operation_contract().unwrap().sample_error;
        let json = serde_json::to_value(&error).unwrap();
        assert_eq!(json["code"], "target_changed_since_preview");
        assert_eq!(json["category"], "target");
        assert_eq!(json["phase"], "revalidation");
        assert_eq!(json["target_id"], "synthetic.failed");
        assert_eq!(json["human_message"], json["message"]);
        assert_eq!(json["requires_elevation"], false);
        assert_eq!(json["requires_restart"], false);
        assert!(json["diagnostic_context"].is_object());
        assert_eq!(
            serde_json::from_value::<OperationError>(json).unwrap(),
            error
        );
    }

    #[test]
    fn contextual_structured_error_serializes_operation_metadata() {
        let mut diagnostic_context = BTreeMap::new();
        diagnostic_context.insert("source".into(), "undo".into());
        let error = OperationError {
            code: OperationErrorCode::ExecutionFailed,
            message: "restore failed".into(),
            target_id: Some("target.one".into()),
            retryable: true,
        }
        .with_context(OperationErrorContext {
            operation_id: Some(OperationId::from("operation.one")),
            phase: Some(OperationErrorPhase::Verification),
            native_error_code: Some(5),
            diagnostic_context,
            requires_elevation: Some(true),
            requires_restart: Some(true),
            ..OperationErrorContext::default()
        });
        let json = serde_json::to_value(error).unwrap();

        assert_eq!(json["operation_id"], "operation.one");
        assert_eq!(json["target_id"], "target.one");
        assert_eq!(json["phase"], "verification");
        assert_eq!(json["native_error_code"], 5);
        assert_eq!(json["diagnostic_context"]["source"], "undo");
        assert_eq!(json["retryable"], true);
        assert_eq!(json["requires_elevation"], true);
        assert_eq!(json["requires_restart"], true);
    }

    #[test]
    fn legacy_structured_error_payload_still_deserializes() {
        let error = serde_json::from_value::<OperationError>(serde_json::json!({
            "code": "execution_failed",
            "message": "legacy failure",
            "target_id": "legacy.target",
            "retryable": true
        }))
        .unwrap();

        assert_eq!(error.code, OperationErrorCode::ExecutionFailed);
        assert_eq!(error.message, "legacy failure");
        assert_eq!(error.target_id.as_deref(), Some("legacy.target"));
    }
}
