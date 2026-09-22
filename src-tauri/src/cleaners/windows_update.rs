//! Windows Update cache cleaner - targets SoftwareDistribution download folder
//! and Delivery Optimization cache.

use crate::cleaners::CleanItem;
use crate::core::error::Result;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use walkdir::WalkDir;

const UPDATE_SERVICES: [&str; 2] = ["BITS", "wuauserv"];
const SERVICE_STATE_TIMEOUT: Duration = Duration::from_secs(5);
const SERVICE_STATE_POLL_INTERVAL: Duration = Duration::from_millis(50);

type ServiceResult<T> = std::result::Result<T, String>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServiceRunState {
    Running,
    Stopped,
}

trait ServiceController: Send + Sync {
    fn query(&self, service_name: &str) -> ServiceResult<ServiceRunState>;
    fn start(&self, service_name: &str) -> ServiceResult<()>;
    fn stop(&self, service_name: &str) -> ServiceResult<()>;
}

#[derive(Debug, Clone, Copy)]
struct ServiceSnapshot {
    name: &'static str,
    state: ServiceRunState,
}

/// Owns the temporary Windows Update service transition for one cleanup.
///
/// The guard records both services before changing either one. `restore` is
/// explicit so the caller can surface failures, while `Drop` remains a
/// best-effort last line of defense during early returns and unwinding.
pub(crate) struct WindowsUpdateServiceGuard {
    controller: Box<dyn ServiceController>,
    snapshots: Vec<ServiceSnapshot>,
    restored: bool,
}

pub(crate) fn begin_cleanup_service_guard() -> ServiceResult<WindowsUpdateServiceGuard> {
    WindowsUpdateServiceGuard::new(Box::new(NativeServiceController))
}

impl WindowsUpdateServiceGuard {
    fn new(controller: Box<dyn ServiceController>) -> ServiceResult<Self> {
        let snapshots = UPDATE_SERVICES
            .iter()
            .map(|&name| {
                controller
                    .query(name)
                    .map(|state| ServiceSnapshot { name, state })
            })
            .collect::<ServiceResult<Vec<_>>>()?;
        let mut guard = Self {
            controller,
            snapshots,
            restored: false,
        };

        if let Err(error) = guard.stop_running_services() {
            let restoration = guard.restore();
            return Err(match restoration {
                Ok(()) => format!("service preparation failed: {error}"),
                Err(restoration_error) => format!(
                    "service preparation failed: {error}; service restoration unresolved: {restoration_error}"
                ),
            });
        }

        Ok(guard)
    }

    pub(crate) fn restore(&mut self) -> ServiceResult<()> {
        if self.restored {
            return Ok(());
        }

        let mut failures = Vec::new();
        for snapshot in &self.snapshots {
            if let Err(error) = self.ensure_state(snapshot.name, snapshot.state) {
                failures.push(format!("{}: {error}", snapshot.name));
            }
        }

        if failures.is_empty() {
            self.restored = true;
            Ok(())
        } else {
            Err(format!(
                "service restoration failed: {}",
                failures.join("; ")
            ))
        }
    }

    fn stop_running_services(&self) -> ServiceResult<()> {
        for snapshot in &self.snapshots {
            if snapshot.state == ServiceRunState::Running {
                self.ensure_state(snapshot.name, ServiceRunState::Stopped)?;
            }
        }
        Ok(())
    }

    fn ensure_state(&self, service_name: &str, desired: ServiceRunState) -> ServiceResult<()> {
        if self.controller.query(service_name)? == desired {
            return Ok(());
        }

        let transition = match desired {
            ServiceRunState::Running => self.controller.start(service_name),
            ServiceRunState::Stopped => self.controller.stop(service_name),
        };
        if let Err(error) = transition {
            if self.controller.query(service_name).ok() == Some(desired) {
                return Ok(());
            }
            return Err(error);
        }

        let deadline = Instant::now() + SERVICE_STATE_TIMEOUT;
        loop {
            match self.controller.query(service_name) {
                Ok(state) if state == desired => return Ok(()),
                Ok(_) if Instant::now() >= deadline => {
                    return Err(format!("did not reach the requested state {desired:?}"));
                }
                Err(error) if Instant::now() >= deadline => return Err(error),
                Ok(_) | Err(_) => std::thread::sleep(SERVICE_STATE_POLL_INTERVAL),
            }
        }
    }
}

impl Drop for WindowsUpdateServiceGuard {
    fn drop(&mut self) {
        if self.restored {
            return;
        }

        let restoration = catch_unwind(AssertUnwindSafe(|| self.restore()));
        match restoration {
            Ok(Ok(())) => {}
            Ok(Err(error)) => eprintln!("Windows Update service restoration unresolved: {error}"),
            Err(_) => eprintln!("Windows Update service restoration panicked"),
        }
    }
}

struct NativeServiceController;

impl ServiceController for NativeServiceController {
    fn query(&self, service_name: &str) -> ServiceResult<ServiceRunState> {
        match crate::startup::services::inspect(service_name)?
            .status
            .as_str()
        {
            "Running" => Ok(ServiceRunState::Running),
            "Stopped" => Ok(ServiceRunState::Stopped),
            state => Err(format!(
                "native Service Control Manager returned unsupported state {state}"
            )),
        }
    }

    fn start(&self, service_name: &str) -> ServiceResult<()> {
        crate::startup::services::control_runtime_state_for_cleanup(service_name, "start")
    }

    fn stop(&self, service_name: &str) -> ServiceResult<()> {
        crate::startup::services::control_runtime_state_for_cleanup(service_name, "stop")
    }
}

/// Known Windows Update cache directories.
fn update_cache_dirs() -> Vec<(PathBuf, &'static str)> {
    let system_root = std::env::var("SYSTEMROOT").unwrap_or_else(|_| "C:\\Windows".into());
    let system = PathBuf::from(&system_root);

    vec![
        (
            system.join("SoftwareDistribution\\Download"),
            "Windows Update cache (Download folder)",
        ),
        (
            system.join("SoftwareDistribution\\DataStore"),
            "Windows Update data store (logs + history)",
        ),
    ]
}

/// Scan Windows Update cache directories.
pub fn scan() -> Result<Vec<CleanItem>> {
    scan_dirs(update_cache_dirs())
}

/// Scan an explicit set of update-cache directories.
///
/// Kept separate from environment discovery so characterization tests can
/// exercise real scanner output against the production authorization model.
pub(crate) fn scan_dirs(dirs: Vec<(PathBuf, &'static str)>) -> Result<Vec<CleanItem>> {
    let mut items = Vec::new();

    for (dir, description) in dirs {
        if !dir.exists() {
            continue;
        }
        // Derive a short group_key from the directory name
        let group_key = dir
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| description.to_string());

        for entry in WalkDir::new(&dir)
            .max_depth(3)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if entry.file_type().is_file() {
                let meta = entry.metadata().ok();
                let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
                if size > 0 {
                    let sub_group = entry
                        .path()
                        .strip_prefix(&dir)
                        .ok()
                        .and_then(|rel| {
                            rel.components()
                                .next()
                                .map(|c| c.as_os_str().to_string_lossy().to_string())
                        })
                        .unwrap_or_else(|| "Root".to_string());

                    items.push(CleanItem {
                        path: entry.path().to_string_lossy().to_string(),
                        size_bytes: size,
                        last_accessed: meta.and_then(|m| m.accessed().ok()).map(|t| {
                            let st: std::time::SystemTime = t;
                            let dt: chrono::DateTime<chrono::Utc> = st.into();
                            dt.to_rfc3339()
                        }),
                        category: "update_cache".to_string(),
                        description: description.to_string(),
                        group_key: group_key.clone(),
                        sub_group,
                    });
                }
            }
        }
    }

    items.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    #[derive(Debug)]
    struct MockState {
        services: BTreeMap<String, ServiceRunState>,
        fail_query: Option<String>,
        fail_start: Option<String>,
        fail_stop: Option<String>,
    }

    #[derive(Clone)]
    struct MockServiceController {
        state: Arc<Mutex<MockState>>,
    }

    impl MockServiceController {
        fn new(bits: ServiceRunState, windows_update: ServiceRunState) -> Self {
            let mut services = BTreeMap::new();
            services.insert("BITS".into(), bits);
            services.insert("wuauserv".into(), windows_update);
            Self {
                state: Arc::new(Mutex::new(MockState {
                    services,
                    fail_query: None,
                    fail_start: None,
                    fail_stop: None,
                })),
            }
        }

        fn service_state(&self, name: &str) -> ServiceRunState {
            self.state
                .lock()
                .unwrap()
                .services
                .get(name)
                .copied()
                .unwrap()
        }

        fn fail_start(&self, name: &str) {
            self.state.lock().unwrap().fail_start = Some(name.into());
        }

        fn fail_stop(&self, name: &str) {
            self.state.lock().unwrap().fail_stop = Some(name.into());
        }

        fn clear_failures(&self) {
            let mut state = self.state.lock().unwrap();
            state.fail_query = None;
            state.fail_start = None;
            state.fail_stop = None;
        }
    }

    impl ServiceController for MockServiceController {
        fn query(&self, service_name: &str) -> ServiceResult<ServiceRunState> {
            let state = self.state.lock().unwrap();
            if state.fail_query.as_deref() == Some(service_name) {
                return Err(format!("query failed for {service_name}"));
            }
            state
                .services
                .get(service_name)
                .copied()
                .ok_or_else(|| format!("unknown service {service_name}"))
        }

        fn start(&self, service_name: &str) -> ServiceResult<()> {
            let mut state = self.state.lock().unwrap();
            if state.fail_start.as_deref() == Some(service_name) {
                return Err(format!("start failed for {service_name}"));
            }
            state
                .services
                .insert(service_name.into(), ServiceRunState::Running);
            Ok(())
        }

        fn stop(&self, service_name: &str) -> ServiceResult<()> {
            let mut state = self.state.lock().unwrap();
            if state.fail_stop.as_deref() == Some(service_name) {
                return Err(format!("stop failed for {service_name}"));
            }
            state
                .services
                .insert(service_name.into(), ServiceRunState::Stopped);
            Ok(())
        }
    }

    #[test]
    fn guard_restores_running_and_stopped_service_states() {
        let controller =
            MockServiceController::new(ServiceRunState::Running, ServiceRunState::Stopped);
        let mut guard = WindowsUpdateServiceGuard::new(Box::new(controller.clone())).unwrap();

        assert_eq!(controller.service_state("BITS"), ServiceRunState::Stopped);
        assert_eq!(
            controller.service_state("wuauserv"),
            ServiceRunState::Stopped
        );

        guard.restore().unwrap();

        assert_eq!(controller.service_state("BITS"), ServiceRunState::Running);
        assert_eq!(
            controller.service_state("wuauserv"),
            ServiceRunState::Stopped
        );
    }

    #[test]
    fn guard_preserves_all_running_and_stopped_combinations() {
        for (bits, windows_update) in [
            (ServiceRunState::Running, ServiceRunState::Running),
            (ServiceRunState::Running, ServiceRunState::Stopped),
            (ServiceRunState::Stopped, ServiceRunState::Running),
            (ServiceRunState::Stopped, ServiceRunState::Stopped),
        ] {
            let controller = MockServiceController::new(bits, windows_update);
            let mut guard = WindowsUpdateServiceGuard::new(Box::new(controller.clone())).unwrap();

            assert_eq!(controller.service_state("BITS"), ServiceRunState::Stopped);
            assert_eq!(
                controller.service_state("wuauserv"),
                ServiceRunState::Stopped
            );

            guard.restore().unwrap();

            assert_eq!(controller.service_state("BITS"), bits);
            assert_eq!(controller.service_state("wuauserv"), windows_update);
        }
    }

    #[test]
    fn guard_reports_restore_failure_and_can_retry_without_claiming_restored() {
        let controller =
            MockServiceController::new(ServiceRunState::Running, ServiceRunState::Stopped);
        let mut guard = WindowsUpdateServiceGuard::new(Box::new(controller.clone())).unwrap();
        controller.fail_start("BITS");

        let error = guard.restore().unwrap_err();
        assert!(error.contains("BITS"));
        assert!(error.contains("service restoration failed"));
        assert_eq!(controller.service_state("BITS"), ServiceRunState::Stopped);

        controller.clear_failures();
        guard.restore().unwrap();
        assert_eq!(controller.service_state("BITS"), ServiceRunState::Running);
    }

    #[test]
    fn guard_restores_during_unwind() {
        let controller =
            MockServiceController::new(ServiceRunState::Running, ServiceRunState::Running);
        let result = catch_unwind(AssertUnwindSafe(|| {
            let _guard = WindowsUpdateServiceGuard::new(Box::new(controller.clone())).unwrap();
            panic!("simulated cleanup panic");
        }));

        assert!(result.is_err());
        assert_eq!(controller.service_state("BITS"), ServiceRunState::Running);
        assert_eq!(
            controller.service_state("wuauserv"),
            ServiceRunState::Running
        );
    }

    #[test]
    fn guard_setup_reports_unresolved_partial_restoration() {
        let controller =
            MockServiceController::new(ServiceRunState::Running, ServiceRunState::Running);
        controller.fail_stop("wuauserv");
        controller.fail_start("BITS");

        let error = match WindowsUpdateServiceGuard::new(Box::new(controller.clone())) {
            Ok(_) => panic!("service setup unexpectedly succeeded"),
            Err(error) => error,
        };

        assert!(error.contains("service restoration unresolved"));
        assert_eq!(controller.service_state("BITS"), ServiceRunState::Stopped);
        assert_eq!(
            controller.service_state("wuauserv"),
            ServiceRunState::Running
        );
    }
}
