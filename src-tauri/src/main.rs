//! WinOptimizer - Windows system optimization utility.
//! Binary entry point that delegates to the library.

//! Disable console window (applies to both debug and release builds).
//! The cmd window is unwanted when the exe is run directly (e.g. in a VM).
#![windows_subsystem = "windows"]

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if let Some(result) = fixture_root_arg(&args) {
        match result {
            Ok(root) => winoptimizer_lib::run_with_runtime(
                winoptimizer_lib::core::runtime::RuntimeContext::fixture(root),
            ),
            Err(message) => {
                eprintln!("{message}");
                std::process::exit(2);
            }
        }
        return;
    }

    // Headless, read-only benchmark entry point used by the disposable VM.
    if let Some(pos) = args.iter().position(|arg| arg == "--disk-benchmark") {
        let Some(root) = args.get(pos + 1) else {
            eprintln!("--disk-benchmark requires a volume path and report path");
            std::process::exit(2);
        };
        let Some(report_path) = args.get(pos + 2) else {
            eprintln!("--disk-benchmark requires a volume path and report path");
            std::process::exit(2);
        };
        std::process::exit(winoptimizer_lib::run_disk_benchmark(root, report_path));
    }

    // Check for --silent mode (triggered by scheduled task)
    if args.contains(&"--silent".to_string()) {
        if let Some(pos) = args.iter().position(|a| a == "--run-tasks") {
            if let Some(tasks_str) = args.get(pos + 1) {
                let exit_code = run_silent_tasks(tasks_str);
                std::process::exit(exit_code);
            } else {
                log::error!("--silent specified with --run-tasks but no task list provided");
                std::process::exit(1);
            }
        }
        return;
    }

    // Check for --elevated mode (re-launched via UAC)
    if args.contains(&"--elevated".to_string()) {
        if args.contains(&"--resume-operation".to_string()) {
            winoptimizer_lib::run_elevated_resume();
            return;
        }
        log::error!("--elevated requires a broker-bound resumable operation");
        std::process::exit(1);
    }

    winoptimizer_lib::run();
}

/// Parse the test-only fixture mode without ever falling back to production.
fn fixture_root_arg(args: &[String]) -> Option<Result<std::path::PathBuf, String>> {
    let pos = args.iter().position(|arg| arg == "--fixture-root")?;
    let Some(value) = args.get(pos + 1).filter(|value| !value.is_empty()) else {
        return Some(Err(
            "--fixture-root requires an absolute directory path".into()
        ));
    };
    if args.iter().skip(pos + 2).any(|arg| arg == "--fixture-root") {
        return Some(Err("--fixture-root may only be specified once".into()));
    }
    let root = std::path::PathBuf::from(value);
    if !root.is_absolute() {
        return Some(Err(
            "--fixture-root requires an absolute directory path".into()
        ));
    }
    match std::fs::create_dir_all(&root) {
        Ok(()) if root.is_dir() => Some(Ok(root)),
        Ok(()) => Some(Err("--fixture-root must identify a directory".into())),
        Err(error) => Some(Err(format!("invalid --fixture-root: {error}"))),
    }
}

fn run_silent_tasks(tasks: &str) -> i32 {
    init_silent_logger();

    log::info!("Running read-only maintenance scans: {}", tasks);
    let safe_paths = winoptimizer_lib::core::safe_paths::SafePaths::new();
    let mut exit_code = 0;

    for task in tasks.split(',') {
        match task.trim() {
            "temp" => match winoptimizer_lib::cleaners::temp_files::scan(&safe_paths) {
                Ok(items) => {
                    let bytes: u64 = items.iter().map(|item| item.size_bytes).sum();
                    log::info!("Temp scan: {} items, {} bytes eligible", items.len(), bytes);
                }
                Err(e) => {
                    log::error!("Temp scan failed: {}", e);
                    exit_code = 1;
                }
            },
            "browser_cache" => match winoptimizer_lib::cleaners::browser_cache::detect_browsers() {
                Ok(browsers) => match winoptimizer_lib::cleaners::browser_cache::scan(&browsers) {
                    Ok(items) => {
                        let bytes: u64 = items.iter().map(|item| item.size_bytes).sum();
                        log::info!(
                            "Browser cache scan: {} items, {} bytes eligible",
                            items.len(),
                            bytes
                        );
                    }
                    Err(e) => {
                        log::error!("Browser cache scan failed: {}", e);
                        exit_code = 1;
                    }
                },
                Err(e) => {
                    log::error!("Browser detection failed: {}", e);
                    exit_code = 1;
                }
            },
            "logs" => match winoptimizer_lib::cleaners::system_logs::scan() {
                Ok(items) => {
                    let bytes: u64 = items.iter().map(|item| item.size_bytes).sum();
                    log::info!("Log scan: {} items, {} bytes eligible", items.len(), bytes);
                }
                Err(e) => {
                    log::error!("Log scan failed: {}", e);
                    exit_code = 1;
                }
            },
            "windows_update" => match winoptimizer_lib::cleaners::windows_update::scan() {
                Ok(items) => {
                    let bytes: u64 = items.iter().map(|item| item.size_bytes).sum();
                    log::info!(
                        "Windows Update cache scan: {} items, {} bytes eligible",
                        items.len(),
                        bytes
                    );
                }
                Err(e) => {
                    log::error!("Windows Update cache scan failed: {}", e);
                    exit_code = 1;
                }
            },
            other => {
                log::warn!("Unknown silent task: {}", other);
            }
        }
    }

    log::info!(
        "Read-only maintenance scans completed with exit code: {}",
        exit_code
    );
    exit_code
}

/// Initialize a file logger for silent mode so errors are captured.
fn init_silent_logger() {
    let Ok(paths) = winoptimizer_lib::core::paths::AppPaths::installed() else {
        return;
    };
    let log_path = paths.logs.join("silent.log");
    if let Some(parent) = std::path::Path::new(&log_path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // Append to log file
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        use std::io::Write;
        let _ = writeln!(
            file,
            "\n=== WinOptimizer Silent Run: {} ===",
            chrono::Utc::now().to_rfc3339()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::fixture_root_arg;

    #[test]
    fn fixture_root_requires_an_absolute_value() {
        assert!(
            fixture_root_arg(&["app".into(), "--fixture-root".into(), "relative".into()])
                .unwrap()
                .is_err()
        );
    }

    #[test]
    fn fixture_root_missing_value_fails_closed() {
        assert!(fixture_root_arg(&["app".into(), "--fixture-root".into()])
            .unwrap()
            .is_err());
    }

    #[test]
    fn fixture_root_rejects_an_existing_file() {
        let path =
            std::env::temp_dir().join(format!("winoptimizer-fixture-file-{}", std::process::id()));
        std::fs::write(&path, b"not a directory").unwrap();
        let args = vec![
            "app".into(),
            "--fixture-root".into(),
            path.to_string_lossy().into(),
        ];
        assert!(fixture_root_arg(&args).unwrap().is_err());
        std::fs::remove_file(path).unwrap();
    }
}
