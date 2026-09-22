use thiserror::Error;

#[derive(Error, Debug)]
pub enum WinOptimizerError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("SQLite error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("Windows API error: {0}")]
    Windows(#[from] windows::core::Error),

    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("Path not allowed for deletion: {0}")]
    PathNotAllowed(String),

    #[error("Elevation required for this operation")]
    ElevationRequired,

    #[error("Operation cancelled by user")]
    Cancelled,

    #[error("{0}")]
    Other(String),
}

impl From<WinOptimizerError> for String {
    fn from(e: WinOptimizerError) -> Self {
        e.to_string()
    }
}

const OPERATION_CANCELLED_ERROR_CODE: &str = "operation_cancelled";

#[derive(serde::Serialize)]
struct TypedIpcError<'a> {
    code: &'static str,
    message: &'a str,
}

impl WinOptimizerError {
    fn typed_ipc_code(&self) -> Option<&'static str> {
        match self {
            Self::Cancelled => Some(OPERATION_CANCELLED_ERROR_CODE),
            // The preview decoder predates the typed error boundary and still
            // reports cancellation through the generic Other variant.
            Self::Other(message) if message == "duplicate preview cancelled" => {
                Some(OPERATION_CANCELLED_ERROR_CODE)
            }
            _ => None,
        }
    }
}

impl serde::Serialize for WinOptimizerError {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        if let Some(code) = self.typed_ipc_code() {
            let message = self.to_string();
            return TypedIpcError {
                code,
                message: &message,
            }
            .serialize(serializer);
        }
        serializer.serialize_str(&self.to_string())
    }
}

pub type Result<T> = std::result::Result<T, WinOptimizerError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_io_error_display() {
        let err = WinOptimizerError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "file not found",
        ));
        let msg = err.to_string();
        assert!(
            msg.contains("IO error"),
            "Should contain 'IO error': {}",
            msg
        );
        assert!(
            msg.contains("file not found"),
            "Should contain detail: {}",
            msg
        );
    }

    #[test]
    fn test_path_not_allowed() {
        let err = WinOptimizerError::PathNotAllowed("C:\\boot\\system".into());
        assert!(
            err.to_string().contains("Path not allowed"),
            "Should contain 'Path not allowed'"
        );
    }

    #[test]
    fn test_elevation_required() {
        let err = WinOptimizerError::ElevationRequired;
        assert!(
            err.to_string().contains("Elevation required"),
            "Should contain 'Elevation required'"
        );
    }

    #[test]
    fn test_other_error() {
        let err = WinOptimizerError::Other("custom message".into());
        assert_eq!(err.to_string(), "custom message");
    }

    #[test]
    fn test_from_io_error() {
        let io = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        let err: WinOptimizerError = io.into();
        assert!(err.to_string().contains("IO error"));
    }

    #[test]
    fn test_cancelled_errors_use_stable_ipc_code() {
        let err = WinOptimizerError::Cancelled;
        let serialized = serde_json::to_value(&err).unwrap();
        assert_eq!(serialized["code"], "operation_cancelled");
        assert_eq!(serialized["message"], "Operation cancelled by user");
    }

    #[test]
    fn legacy_preview_cancellation_gets_the_same_typed_code() {
        let err = WinOptimizerError::Other("duplicate preview cancelled".into());
        let serialized = serde_json::to_value(&err).unwrap();
        assert_eq!(serialized["code"], "operation_cancelled");
    }

    #[test]
    fn non_cancelled_errors_keep_legacy_string_serialization() {
        let err = WinOptimizerError::Other("custom message".into());
        assert_eq!(serde_json::to_value(&err).unwrap(), "custom message");
    }

    #[test]
    fn test_result_type_alias() {
        let ok: Result<i32> = Ok(42);
        match ok {
            Ok(v) => assert_eq!(v, 42),
            Err(_) => panic!(),
        }
    }
}
