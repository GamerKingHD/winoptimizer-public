//! One-shot authenticated broker session used for resumable administrator work.
//!
//! The unelevated process owns the session endpoint.  The elevated child must
//! authenticate the endpoint, session, nonce, application, protocol, operation
//! id, and exact confirmed-plan digest before it can continue.  The endpoint is
//! consumed after one valid hello and is never a general command channel.

use crate::core::operation::OperationPlan;
use chrono::{DateTime, Duration, Utc};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration as StdDuration;

pub const BROKER_PROTOCOL_VERSION: u16 = 1;
pub const BROKER_LIFETIME_SECONDS: i64 = 10 * 60;

const MAX_FRAME_BYTES: usize = 64 * 1024;
const PIPE_BUFFER_BYTES: u32 = 64 * 1024;

pub fn broker_pipe_sddl() -> &'static str {
    "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;OW)"
}

#[cfg(any(test, debug_assertions))]
static FAIL_ELEVATION_RESPONSE: AtomicBool = AtomicBool::new(false);

#[cfg(any(test, debug_assertions))]
#[allow(dead_code)]
pub(crate) fn fail_elevation_response() {
    FAIL_ELEVATION_RESPONSE.store(true, Ordering::Release);
}

#[cfg(any(test, debug_assertions))]
fn take_fail_elevation_response() -> bool {
    FAIL_ELEVATION_RESPONSE.swap(false, Ordering::AcqRel)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrokerErrorCode {
    MalformedProtocol,
    UnsupportedProtocol,
    ApplicationMismatch,
    SessionMismatch,
    NonceMismatch,
    OperationMismatch,
    PlanDigestMismatch,
    Expired,
    Replay,
    EndpointUnauthorized,
    PlanInvalid,
    Transport,
    Execution,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrokerError {
    pub code: BrokerErrorCode,
    pub phase: String,
    pub message: String,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_error_code: Option<u32>,
}

impl BrokerError {
    fn new(code: BrokerErrorCode, phase: &str, message: impl Into<String>) -> Self {
        Self {
            code,
            phase: phase.into(),
            message: message.into(),
            retryable: false,
            native_error_code: None,
        }
    }

    fn transport(phase: &str, error: impl Into<String>) -> Self {
        Self::new(BrokerErrorCode::Transport, phase, error)
    }

    fn malformed(message: impl Into<String>) -> Self {
        Self::new(BrokerErrorCode::MalformedProtocol, "protocol", message)
    }

    fn with_native_code(mut self, code: u32) -> Self {
        self.native_error_code = Some(code);
        self
    }
}

impl fmt::Display for BrokerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.phase, self.message)
    }
}

impl std::error::Error for BrokerError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokerSession {
    pub endpoint: String,
    pub session_id: String,
    pub nonce: String,
    pub operation_id: String,
    pub plan_digest: String,
    pub expires_at: String,
    pub protocol_version: u16,
    pub application_version: String,
    #[cfg(windows)]
    windows_session_id: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrokerRequest {
    pub protocol_version: u16,
    pub application_version: String,
    pub session_id: String,
    pub nonce: String,
    pub operation_id: String,
    pub plan_digest: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrokerOutcome {
    pub operation_id: String,
    pub plan_digest: String,
    pub status: BrokerOutcomeStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<BrokerError>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrokerOutcomeStatus {
    Succeeded,
    PartiallySucceeded,
    Failed,
    OutcomeUnknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "message", rename_all = "snake_case")]
enum BrokerClientMessage {
    Hello { request: BrokerRequest },
    Outcome { outcome: BrokerOutcome },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "message", rename_all = "snake_case")]
enum BrokerServerMessage {
    Accepted {
        protocol_version: u16,
        operation_id: String,
        plan_digest: String,
    },
    Rejected {
        error: BrokerError,
    },
    Receipt {
        outcome: BrokerOutcome,
    },
}

impl BrokerSession {
    pub fn create(
        plan: &OperationPlan,
        operation_id: &str,
        nonce: &str,
        plan_digest: &str,
    ) -> Result<Self, BrokerError> {
        Self::create_at(plan, operation_id, nonce, plan_digest, Utc::now())
    }

    fn create_at(
        plan: &OperationPlan,
        operation_id: &str,
        nonce: &str,
        expected_digest: &str,
        now: DateTime<Utc>,
    ) -> Result<Self, BrokerError> {
        validate_confirmed_plan(plan)?;
        if operation_id.is_empty() || operation_id != plan.id.0 {
            return Err(BrokerError::new(
                BrokerErrorCode::OperationMismatch,
                "session",
                "broker operation id is not the confirmed plan id",
            ));
        }
        if nonce.is_empty() {
            return Err(BrokerError::new(
                BrokerErrorCode::NonceMismatch,
                "session",
                "broker nonce is empty",
            ));
        }
        let actual_digest = plan_digest(plan)?;
        if expected_digest != actual_digest {
            return Err(BrokerError::new(
                BrokerErrorCode::PlanDigestMismatch,
                "session",
                "broker digest does not match the confirmed plan",
            ));
        }

        Ok(Self {
            endpoint: format!(r"\\.\pipe\WinOptimizer-{}", uuid::Uuid::new_v4()),
            session_id: uuid::Uuid::new_v4().to_string(),
            nonce: nonce.to_string(),
            operation_id: operation_id.to_string(),
            plan_digest: actual_digest,
            expires_at: (now + Duration::seconds(BROKER_LIFETIME_SECONDS)).to_rfc3339(),
            protocol_version: BROKER_PROTOCOL_VERSION,
            application_version: env!("CARGO_PKG_VERSION").into(),
            #[cfg(windows)]
            windows_session_id: current_windows_session_id()?,
        })
    }

    pub fn from_parts(
        endpoint: String,
        session_id: String,
        nonce: String,
        operation_id: String,
        plan_digest: String,
        expires_at: String,
    ) -> Result<Self, BrokerError> {
        let expires = parse_expiration(&expires_at)?;
        if endpoint.is_empty()
            || session_id.is_empty()
            || nonce.is_empty()
            || operation_id.is_empty()
            || plan_digest.is_empty()
            || expires <= Utc::now()
        {
            return Err(BrokerError::new(
                BrokerErrorCode::MalformedProtocol,
                "session",
                "broker session arguments are incomplete or expired",
            ));
        }
        Ok(Self {
            endpoint,
            session_id,
            nonce,
            operation_id,
            plan_digest,
            expires_at,
            protocol_version: BROKER_PROTOCOL_VERSION,
            application_version: env!("CARGO_PKG_VERSION").into(),
            #[cfg(windows)]
            windows_session_id: current_windows_session_id()?,
        })
    }

    pub fn request(&self) -> BrokerRequest {
        BrokerRequest {
            protocol_version: self.protocol_version,
            application_version: self.application_version.clone(),
            session_id: self.session_id.clone(),
            nonce: self.nonce.clone(),
            operation_id: self.operation_id.clone(),
            plan_digest: self.plan_digest.clone(),
            expires_at: self.expires_at.clone(),
        }
    }

    fn expiration(&self) -> Result<DateTime<Utc>, BrokerError> {
        parse_expiration(&self.expires_at)
    }
}

pub fn validate_confirmed_plan(plan: &OperationPlan) -> Result<(), BrokerError> {
    if !plan.requires_elevation || plan.targets.is_empty() {
        return Err(BrokerError::new(
            BrokerErrorCode::PlanInvalid,
            "plan",
            "broker requires a non-empty administrator plan",
        ));
    }
    plan.descriptor.validate_contract().map_err(|error| {
        BrokerError::new(BrokerErrorCode::PlanInvalid, "plan", error.to_string())
    })?;
    plan.validate_relationships()
        .map_err(|error| BrokerError::new(BrokerErrorCode::PlanInvalid, "plan", error.to_string()))
}

pub fn plan_digest(plan: &OperationPlan) -> Result<String, BrokerError> {
    let bytes = serde_json::to_vec(plan).map_err(|error| {
        BrokerError::new(BrokerErrorCode::PlanInvalid, "plan", error.to_string())
    })?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}

pub fn validate_plan_binding(
    session: &BrokerSession,
    plan: &OperationPlan,
) -> Result<(), BrokerError> {
    validate_confirmed_plan(plan)?;
    if plan.id.0 != session.operation_id {
        return Err(BrokerError::new(
            BrokerErrorCode::OperationMismatch,
            "plan",
            "loaded plan operation id does not match the broker session",
        ));
    }
    if plan_digest(plan)? != session.plan_digest {
        return Err(BrokerError::new(
            BrokerErrorCode::PlanDigestMismatch,
            "plan",
            "loaded plan digest does not match the broker session",
        ));
    }
    Ok(())
}

#[derive(Debug)]
pub struct SessionVerifier {
    session: BrokerSession,
    consumed: AtomicBool,
}

impl SessionVerifier {
    pub fn new(session: BrokerSession) -> Self {
        Self {
            session,
            consumed: AtomicBool::new(false),
        }
    }

    pub fn validate_and_consume(
        &self,
        request: &BrokerRequest,
        now: DateTime<Utc>,
    ) -> Result<(), BrokerError> {
        if now >= self.session.expiration()? {
            return Err(BrokerError::new(
                BrokerErrorCode::Expired,
                "authenticate",
                "broker session has expired",
            ));
        }
        if self.consumed.load(Ordering::Acquire) {
            return Err(BrokerError::new(
                BrokerErrorCode::Replay,
                "authenticate",
                "broker session has already been consumed",
            ));
        }
        if request.protocol_version != self.session.protocol_version {
            return Err(BrokerError::new(
                BrokerErrorCode::UnsupportedProtocol,
                "authenticate",
                "broker protocol version is unsupported",
            ));
        }
        if request.application_version != self.session.application_version {
            return Err(BrokerError::new(
                BrokerErrorCode::ApplicationMismatch,
                "authenticate",
                "broker application version does not match",
            ));
        }
        if request.session_id != self.session.session_id {
            return Err(BrokerError::new(
                BrokerErrorCode::SessionMismatch,
                "authenticate",
                "broker session identifier does not match",
            ));
        }
        if request.nonce != self.session.nonce {
            return Err(BrokerError::new(
                BrokerErrorCode::NonceMismatch,
                "authenticate",
                "broker nonce does not match",
            ));
        }
        if request.operation_id != self.session.operation_id {
            return Err(BrokerError::new(
                BrokerErrorCode::OperationMismatch,
                "authenticate",
                "broker operation identifier does not match",
            ));
        }
        if request.plan_digest != self.session.plan_digest {
            return Err(BrokerError::new(
                BrokerErrorCode::PlanDigestMismatch,
                "authenticate",
                "broker plan digest does not match",
            ));
        }
        if request.expires_at != self.session.expires_at {
            return Err(BrokerError::new(
                BrokerErrorCode::Expired,
                "authenticate",
                "broker expiration binding does not match",
            ));
        }
        if self.consumed.swap(true, Ordering::AcqRel) {
            return Err(BrokerError::new(
                BrokerErrorCode::Replay,
                "authenticate",
                "broker session has already been consumed",
            ));
        }
        Ok(())
    }

    pub fn validate_outcome(&self, outcome: &BrokerOutcome) -> Result<(), BrokerError> {
        if outcome.operation_id != self.session.operation_id {
            return Err(BrokerError::new(
                BrokerErrorCode::OperationMismatch,
                "outcome",
                "broker outcome operation identifier does not match",
            ));
        }
        if outcome.plan_digest != self.session.plan_digest {
            return Err(BrokerError::new(
                BrokerErrorCode::PlanDigestMismatch,
                "outcome",
                "broker outcome plan digest does not match",
            ));
        }
        Ok(())
    }
}

fn parse_expiration(value: &str) -> Result<DateTime<Utc>, BrokerError> {
    DateTime::parse_from_rfc3339(value)
        .map(|date| date.with_timezone(&Utc))
        .map_err(|_| {
            BrokerError::new(
                BrokerErrorCode::Expired,
                "session",
                "broker expiration is invalid",
            )
        })
}

fn validate_frame_size(size: usize) -> Result<(), BrokerError> {
    if size == 0 || size > MAX_FRAME_BYTES {
        return Err(BrokerError::malformed("broker frame size is invalid"));
    }
    Ok(())
}

fn encode_frame<T: Serialize>(message: &T) -> Result<Vec<u8>, BrokerError> {
    let payload = serde_json::to_vec(message).map_err(|error| {
        BrokerError::malformed(format!("broker message cannot be serialized: {error}"))
    })?;
    validate_frame_size(payload.len())?;
    let size = u32::try_from(payload.len())
        .map_err(|_| BrokerError::malformed("broker frame is too large"))?;
    let mut frame = Vec::with_capacity(payload.len() + 4);
    frame.extend_from_slice(&size.to_le_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

fn decode_frame<T: DeserializeOwned>(frame: &[u8]) -> Result<T, BrokerError> {
    if frame.len() < 4 {
        return Err(BrokerError::malformed("broker frame header is truncated"));
    }
    let size = u32::from_le_bytes([frame[0], frame[1], frame[2], frame[3]]) as usize;
    validate_frame_size(size)?;
    if frame.len() != size + 4 {
        return Err(BrokerError::malformed(
            "broker frame length does not match its payload",
        ));
    }
    serde_json::from_slice(&frame[4..])
        .map_err(|_| BrokerError::malformed("broker message is not valid protocol data"))
}

fn authenticated_broker_response(
    verifier: &SessionVerifier,
    request: &BrokerRequest,
    session: &BrokerSession,
    now: DateTime<Utc>,
) -> Result<BrokerServerMessage, BrokerError> {
    verifier.validate_and_consume(request, now)?;

    #[cfg(any(test, debug_assertions))]
    if take_fail_elevation_response() {
        return Err(BrokerError::new(
            BrokerErrorCode::Internal,
            "response",
            "failure injection: fail_elevation_response",
        ));
    }

    Ok(BrokerServerMessage::Accepted {
        protocol_version: session.protocol_version,
        operation_id: session.operation_id.clone(),
        plan_digest: session.plan_digest.clone(),
    })
}

#[cfg(windows)]
pub fn start_server(session: BrokerSession) -> Result<(), BrokerError> {
    let pipe = create_server_pipe(&session.endpoint)?;
    thread::Builder::new()
        .name("winoptimizer-elevation-broker".into())
        .spawn(move || {
            if let Err(error) = serve_once(pipe, session) {
                log::warn!("one-shot elevation broker closed: {error}");
            }
        })
        .map_err(|error| {
            BrokerError::transport("server", format!("cannot start broker thread: {error}"))
        })?;
    Ok(())
}

#[cfg(not(windows))]
pub fn start_server(_session: BrokerSession) -> Result<(), BrokerError> {
    Err(BrokerError::new(
        BrokerErrorCode::Transport,
        "server",
        "named-pipe elevation broker is only available on Windows",
    ))
}

#[cfg(windows)]
pub struct BrokerClient {
    pipe: PipeHandle,
    verifier: SessionVerifier,
}

#[cfg(windows)]
pub fn connect(session: BrokerSession) -> Result<BrokerClient, BrokerError> {
    if Utc::now() >= session.expiration()? {
        return Err(BrokerError::new(
            BrokerErrorCode::Expired,
            "connect",
            "broker session has expired",
        ));
    }
    let pipe_name: Vec<u16> = session
        .endpoint
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let remaining = (session.expiration()? - Utc::now())
        .to_std()
        .unwrap_or_else(|_| StdDuration::from_secs(0));
    let timeout_ms = remaining.as_millis().min(u32::MAX as u128) as u32;
    unsafe {
        use windows::core::PCWSTR;
        use windows::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE};
        use windows::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_MODE, OPEN_EXISTING};
        use windows::Win32::System::Pipes::WaitNamedPipeW;

        if !WaitNamedPipeW(PCWSTR(pipe_name.as_ptr()), timeout_ms).as_bool() {
            return Err(BrokerError::transport(
                "connect",
                "broker endpoint did not become available",
            )
            .with_native_code(windows::Win32::Foundation::GetLastError().0));
        }
        let pipe = CreateFileW(
            PCWSTR(pipe_name.as_ptr()),
            GENERIC_READ.0 | GENERIC_WRITE.0,
            FILE_SHARE_MODE(0),
            None,
            OPEN_EXISTING,
            Default::default(),
            None,
        )
        .map_err(|error| BrokerError::transport("connect", error.to_string()))?;
        let mode = windows::Win32::System::Pipes::PIPE_READMODE_MESSAGE
            | windows::Win32::System::Pipes::PIPE_WAIT;
        windows::Win32::System::Pipes::SetNamedPipeHandleState(
            pipe,
            Some(&mode as *const _),
            None,
            None,
        )
        .map_err(|error| BrokerError::transport("connect", error.to_string()))?;

        let client = BrokerClient {
            pipe: PipeHandle(pipe),
            verifier: SessionVerifier::new(session.clone()),
        };
        client.write_message(&BrokerClientMessage::Hello {
            request: session.request(),
        })?;
        match client.read_message::<BrokerServerMessage>()? {
            BrokerServerMessage::Accepted {
                protocol_version,
                operation_id,
                plan_digest,
            } if protocol_version == session.protocol_version
                && operation_id == session.operation_id
                && plan_digest == session.plan_digest =>
            {
                Ok(client)
            }
            BrokerServerMessage::Rejected { error } => Err(error),
            _ => Err(BrokerError::malformed(
                "broker acceptance did not bind the session",
            )),
        }
    }
}

#[cfg(windows)]
impl BrokerClient {
    pub fn complete(&mut self, outcome: BrokerOutcome) -> Result<(), BrokerError> {
        self.verifier.validate_outcome(&outcome)?;
        self.write_message(&BrokerClientMessage::Outcome {
            outcome: outcome.clone(),
        })?;
        match self.read_message::<BrokerServerMessage>()? {
            BrokerServerMessage::Receipt { outcome: receipt } => {
                self.verifier.validate_outcome(&receipt)?;
                Ok(())
            }
            BrokerServerMessage::Rejected { error } => Err(error),
            _ => Err(BrokerError::malformed("broker receipt was not returned")),
        }
    }

    fn write_message<T: Serialize>(&self, message: &T) -> Result<(), BrokerError> {
        let frame = encode_frame(message)?;
        write_pipe_frame(self.pipe.0, &frame)
    }

    fn read_message<T: DeserializeOwned>(&self) -> Result<T, BrokerError> {
        let frame = read_pipe_frame(self.pipe.0)?;
        decode_frame(&frame)
    }
}

#[cfg(windows)]
fn serve_once(pipe: PipeHandle, session: BrokerSession) -> Result<(), BrokerError> {
    connect_server_until(pipe.0, session.expiration()?)?;
    verify_client_session(pipe.0, session.windows_session_id)?;

    let verifier = SessionVerifier::new(session.clone());
    let message: BrokerClientMessage = decode_frame(&read_pipe_frame(pipe.0)?)?;
    let request = match message {
        BrokerClientMessage::Hello { request } => request,
        BrokerClientMessage::Outcome { .. } => {
            let error = BrokerError::malformed("broker outcome arrived before authentication");
            let _ = write_server_message(
                pipe.0,
                &BrokerServerMessage::Rejected {
                    error: error.clone(),
                },
            );
            return Err(error);
        }
    };
    let response = match authenticated_broker_response(&verifier, &request, &session, Utc::now()) {
        Ok(response) => response,
        Err(error) => {
            let _ = write_server_message(
                pipe.0,
                &BrokerServerMessage::Rejected {
                    error: error.clone(),
                },
            );
            return Err(error);
        }
    };
    write_server_message(pipe.0, &response)?;

    let message: BrokerClientMessage = decode_frame(&read_pipe_frame(pipe.0)?)?;
    let outcome = match message {
        BrokerClientMessage::Outcome { outcome } => outcome,
        BrokerClientMessage::Hello { .. } => {
            let error = BrokerError::malformed("broker hello was repeated instead of an outcome");
            let _ = write_server_message(
                pipe.0,
                &BrokerServerMessage::Rejected {
                    error: error.clone(),
                },
            );
            return Err(error);
        }
    };
    verifier.validate_outcome(&outcome)?;
    write_server_message(pipe.0, &BrokerServerMessage::Receipt { outcome })
}

#[cfg(windows)]
fn create_server_pipe(endpoint: &str) -> Result<PipeHandle, BrokerError> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{GetLastError, HLOCAL, INVALID_HANDLE_VALUE};
    use windows::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;
    use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
    use windows::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX};
    use windows::Win32::System::Pipes::{
        CreateNamedPipeW, PIPE_NOWAIT, PIPE_READMODE_MESSAGE, PIPE_TYPE_MESSAGE,
    };

    let sddl: Vec<u16> = broker_pipe_sddl()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let endpoint_wide: Vec<u16> = endpoint.encode_utf16().chain(std::iter::once(0)).collect();
    let mut descriptor = PSECURITY_DESCRIPTOR(std::ptr::null_mut());
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl.as_ptr()),
            1,
            &mut descriptor as *mut PSECURITY_DESCRIPTOR,
            None,
        )
        .map_err(|error| {
            BrokerError::new(
                BrokerErrorCode::EndpointUnauthorized,
                "endpoint",
                format!("cannot create named-pipe security descriptor: {error}"),
            )
        })?;

        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor.0,
            bInheritHandle: false.into(),
        };
        let handle = CreateNamedPipeW(
            PCWSTR(endpoint_wide.as_ptr()),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_NOWAIT,
            1,
            PIPE_BUFFER_BYTES,
            PIPE_BUFFER_BYTES,
            1000,
            Some(&attributes),
        );
        let _ = windows::Win32::Foundation::LocalFree(Some(HLOCAL(descriptor.0.cast())));
        if handle == INVALID_HANDLE_VALUE {
            return Err(BrokerError::new(
                BrokerErrorCode::EndpointUnauthorized,
                "endpoint",
                "cannot create one-shot named-pipe endpoint",
            )
            .with_native_code(GetLastError().0));
        }
        Ok(PipeHandle(handle))
    }
}

#[cfg(windows)]
fn connect_server_until(
    handle: windows::Win32::Foundation::HANDLE,
    expires: DateTime<Utc>,
) -> Result<(), BrokerError> {
    use windows::Win32::Foundation::{GetLastError, ERROR_PIPE_CONNECTED, ERROR_PIPE_LISTENING};
    use windows::Win32::System::Pipes::{
        ConnectNamedPipe, SetNamedPipeHandleState, PIPE_READMODE_MESSAGE, PIPE_WAIT,
    };

    loop {
        if Utc::now() >= expires {
            return Err(BrokerError::new(
                BrokerErrorCode::Expired,
                "connect",
                "broker endpoint expired before authentication",
            ));
        }
        unsafe {
            match ConnectNamedPipe(handle, None) {
                Ok(()) => break,
                Err(_) => {
                    let code = GetLastError();
                    if code == ERROR_PIPE_CONNECTED {
                        break;
                    }
                    if code != ERROR_PIPE_LISTENING {
                        return Err(BrokerError::transport(
                            "connect",
                            "named-pipe connection failed",
                        )
                        .with_native_code(code.0));
                    }
                }
            }
        }
        thread::sleep(StdDuration::from_millis(25));
    }

    let mode = PIPE_READMODE_MESSAGE | PIPE_WAIT;
    unsafe {
        SetNamedPipeHandleState(handle, Some(&mode as *const _), None, None)
            .map_err(|error| BrokerError::transport("connect", error.to_string()))?;
    }
    Ok(())
}

#[cfg(windows)]
fn verify_client_session(
    handle: windows::Win32::Foundation::HANDLE,
    expected_session_id: u32,
) -> Result<(), BrokerError> {
    let mut actual_session_id = 0;
    unsafe {
        windows::Win32::System::Pipes::GetNamedPipeClientSessionId(handle, &mut actual_session_id)
            .map_err(|error| {
                BrokerError::new(
                    BrokerErrorCode::EndpointUnauthorized,
                    "endpoint",
                    format!("cannot identify named-pipe client session: {error}"),
                )
            })?;
    }
    if actual_session_id != expected_session_id {
        return Err(BrokerError::new(
            BrokerErrorCode::EndpointUnauthorized,
            "endpoint",
            "named-pipe client belongs to a different Windows session",
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn write_server_message(
    handle: windows::Win32::Foundation::HANDLE,
    message: &BrokerServerMessage,
) -> Result<(), BrokerError> {
    let frame = encode_frame(message)?;
    write_pipe_frame(handle, &frame)
}

#[cfg(windows)]
fn write_pipe_frame(
    handle: windows::Win32::Foundation::HANDLE,
    frame: &[u8],
) -> Result<(), BrokerError> {
    use windows::Win32::Storage::FileSystem::{FlushFileBuffers, WriteFile};
    let mut written = 0;
    unsafe {
        WriteFile(handle, Some(frame), Some(&mut written), None)
            .map_err(|error| BrokerError::transport("write", error.to_string()))?;
        FlushFileBuffers(handle)
            .map_err(|error| BrokerError::transport("write", error.to_string()))?;
    }
    if written as usize != frame.len() {
        return Err(BrokerError::transport(
            "write",
            "named-pipe frame was only partially written",
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn read_pipe_frame(handle: windows::Win32::Foundation::HANDLE) -> Result<Vec<u8>, BrokerError> {
    use windows::Win32::Storage::FileSystem::ReadFile;
    let mut frame = vec![0u8; MAX_FRAME_BYTES + 4];
    let mut read = 0;
    unsafe {
        ReadFile(handle, Some(&mut frame), Some(&mut read), None)
            .map_err(|error| BrokerError::transport("read", error.to_string()))?;
    }
    frame.truncate(read as usize);
    if frame.is_empty() {
        return Err(BrokerError::malformed("broker sent an empty frame"));
    }
    Ok(frame)
}

#[cfg(windows)]
fn current_windows_session_id() -> Result<u32, BrokerError> {
    let mut session_id = 0;
    unsafe {
        let process_id = windows::Win32::System::Threading::GetCurrentProcessId();
        windows::Win32::System::RemoteDesktop::ProcessIdToSessionId(process_id, &mut session_id)
            .map_err(|error| {
                BrokerError::new(
                    BrokerErrorCode::EndpointUnauthorized,
                    "endpoint",
                    format!("cannot determine current Windows session: {error}"),
                )
            })?;
    }
    Ok(session_id)
}

#[cfg(windows)]
struct PipeHandle(windows::Win32::Foundation::HANDLE);

#[cfg(windows)]
// SAFETY: the handle is created by the parent thread and moved exactly once
// into the broker thread, which owns it until Drop closes it.
unsafe impl Send for PipeHandle {}

#[cfg(windows)]
impl Drop for PipeHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::operation::synthetic_operation_contract;

    fn session(now: DateTime<Utc>) -> (BrokerSession, OperationPlan) {
        let plan = synthetic_operation_contract().unwrap().plan;
        let digest = plan_digest(&plan).unwrap();
        let session =
            BrokerSession::create_at(&plan, &plan.id.0, "nonce-for-test", &digest, now).unwrap();
        (session, plan)
    }

    #[test]
    fn valid_session_is_one_shot_and_replay_is_rejected() {
        let now = Utc::now();
        let (session, _) = session(now);
        let verifier = SessionVerifier::new(session.clone());
        verifier
            .validate_and_consume(&session.request(), now)
            .unwrap();
        let error = verifier
            .validate_and_consume(&session.request(), now)
            .unwrap_err();
        assert_eq!(error.code, BrokerErrorCode::Replay);
    }

    #[test]
    fn wrong_operation_and_session_are_rejected() {
        let now = Utc::now();
        let (session, _) = session(now);
        let verifier = SessionVerifier::new(session.clone());
        let mut request = session.request();
        request.operation_id = "wrong-operation".into();
        assert_eq!(
            verifier
                .validate_and_consume(&request, now)
                .unwrap_err()
                .code,
            BrokerErrorCode::OperationMismatch
        );

        request = session.request();
        request.session_id = uuid::Uuid::new_v4().to_string();
        assert_eq!(
            verifier
                .validate_and_consume(&request, now)
                .unwrap_err()
                .code,
            BrokerErrorCode::SessionMismatch
        );
    }

    #[test]
    fn expired_session_is_rejected_before_consumption() {
        let now = Utc::now();
        let (session, _) = session(now - Duration::seconds(BROKER_LIFETIME_SECONDS + 1));
        let verifier = SessionVerifier::new(session.clone());
        let error = verifier
            .validate_and_consume(&session.request(), now)
            .unwrap_err();
        assert_eq!(error.code, BrokerErrorCode::Expired);
    }

    #[test]
    fn corrupted_plan_digest_is_rejected() {
        let now = Utc::now();
        let (session, mut plan) = session(now);
        plan.targets[0].description.push_str(" changed");
        let error = validate_plan_binding(&session, &plan).unwrap_err();
        assert_eq!(error.code, BrokerErrorCode::PlanDigestMismatch);
    }

    #[test]
    fn broker_pipe_sddl_is_owner_only_and_not_interactive_users_generic_all() {
        let sddl = broker_pipe_sddl();
        assert!(
            !sddl.contains("IU"),
            "broker DACL must not grant Interactive Users: {sddl}"
        );
        assert!(
            !sddl.contains("GA;;;IU"),
            "broker DACL must not grant Interactive Users Generic All: {sddl}"
        );
        assert!(
            sddl.contains("OW"),
            "broker DACL must grant the owner: {sddl}"
        );
    }

    #[test]
    fn malformed_protocol_frames_are_rejected() {
        assert_eq!(
            decode_frame::<BrokerClientMessage>(&[1, 2, 3])
                .unwrap_err()
                .code,
            BrokerErrorCode::MalformedProtocol
        );
        assert_eq!(
            decode_frame::<BrokerClientMessage>(&[1, 0, 0, 0, b'{'])
                .unwrap_err()
                .code,
            BrokerErrorCode::MalformedProtocol
        );
        let mut oversized = vec![0u8; 4];
        oversized[..4].copy_from_slice(&((MAX_FRAME_BYTES as u32) + 1).to_le_bytes());
        assert_eq!(
            decode_frame::<BrokerClientMessage>(&oversized)
                .unwrap_err()
                .code,
            BrokerErrorCode::MalformedProtocol
        );
    }

    #[test]
    fn outcome_binding_rejects_wrong_operation() {
        let now = Utc::now();
        let (session, _) = session(now);
        let verifier = SessionVerifier::new(session);
        let outcome = BrokerOutcome {
            operation_id: "wrong-operation".into(),
            plan_digest: "digest".into(),
            status: BrokerOutcomeStatus::Succeeded,
            error: None,
        };
        assert_eq!(
            verifier.validate_outcome(&outcome).unwrap_err().code,
            BrokerErrorCode::OperationMismatch
        );
    }

    #[test]
    fn fail_elevation_response_is_one_shot_before_mutation_and_replay() {
        let now = Utc::now();
        let (broker_session, _) = session(now);
        let verifier = SessionVerifier::new(broker_session.clone());
        fail_elevation_response();

        let response = authenticated_broker_response(
            &verifier,
            &broker_session.request(),
            &broker_session,
            now,
        );
        let mut privileged_mutation_attempted = false;
        if response.is_ok() {
            privileged_mutation_attempted = true;
        }
        let error = response.unwrap_err();
        assert_eq!(error.code, BrokerErrorCode::Internal);
        assert_eq!(error.phase, "response");
        assert_eq!(error.message, "failure injection: fail_elevation_response");
        assert!(!privileged_mutation_attempted);

        let replay_error = authenticated_broker_response(
            &verifier,
            &broker_session.request(),
            &broker_session,
            now,
        )
        .unwrap_err();
        assert_eq!(replay_error.code, BrokerErrorCode::Replay);

        let (next_session, _) = session(now);
        let next_verifier = SessionVerifier::new(next_session.clone());
        assert!(matches!(
            authenticated_broker_response(
                &next_verifier,
                &next_session.request(),
                &next_session,
                now
            ),
            Ok(BrokerServerMessage::Accepted { .. })
        ));
    }
}
