//! Bounded external process execution.
//!
//! Repair commands (SFC, DISM) can hang indefinitely.  This module runs a
//! child process inside a Windows Job Object so that a cancellation or an
//! upper time bound terminates the whole process tree, not just the direct
//! child.  The bound is deliberately generous: it exists to prevent literally
//! unbounded execution, not to abort legitimate slow repairs.

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Generous upper bound for a single repair or verification command.
pub const DEFAULT_REPAIR_TIMEOUT: Duration = Duration::from_secs(2 * 60 * 60);
const POLL_INTERVAL: Duration = Duration::from_millis(150);
const MAX_CAPTURED_BYTES: usize = 4 * 1024 * 1024;

/// Cancellation and time bound shared with the executing repair operation.
#[derive(Clone)]
pub struct RunControl {
    cancel: Arc<AtomicBool>,
    timeout: Duration,
}

impl RunControl {
    pub fn new(cancel: Arc<AtomicBool>, timeout: Duration) -> Self {
        Self { cancel, timeout }
    }

    /// A control that never cancels, for callers that do not expose a cancel
    /// affordance (for example direct unit tests).  The time bound still
    /// applies so no call can hang forever.
    pub fn unbounded_cancel() -> Self {
        Self {
            cancel: Arc::new(AtomicBool::new(false)),
            timeout: DEFAULT_REPAIR_TIMEOUT,
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }
}

/// Whether a Job Object actually owned the child process tree.
///
/// This is structured state, not textual output: callers must consult it
/// directly rather than inferring containment from captured command text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeContainment {
    /// The child was assigned to a Job Object, so cancellation or the time
    /// bound terminates the whole tree.
    Contained,
    /// Job Object assignment failed. Only the direct child is terminated on
    /// cancellation or timeout; a descendant can survive.
    Unavailable,
    /// The command was not a native child process (for example a DLL/API
    /// action), so process-tree containment does not apply.
    NotApplicable,
}

/// The structured reason a bounded command stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessTermination {
    /// The process exited with code 0.
    Completed,
    /// The process exited with a non-zero code.
    NonZeroExit,
    /// Cancellation was requested and the owned tree was terminated.
    Cancelled,
    /// The time bound elapsed and the owned tree was terminated.
    TimedOut,
}

#[derive(Debug)]
pub struct BoundedOutput {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
    pub cancelled: bool,
    /// Structured Job Object containment state for the owned process tree.
    pub containment: TreeContainment,
}

impl BoundedOutput {
    /// Derive the structured termination reason from the observed result.
    pub fn termination(&self) -> ProcessTermination {
        if self.cancelled {
            ProcessTermination::Cancelled
        } else if self.timed_out {
            ProcessTermination::TimedOut
        } else if self.exit_code == Some(0) {
            ProcessTermination::Completed
        } else {
            ProcessTermination::NonZeroExit
        }
    }
}

/// Run a command with cancellation, an upper time bound, and process-tree
/// termination.
pub fn run_bounded(
    program: &str,
    args: &[String],
    control: &RunControl,
) -> std::io::Result<BoundedOutput> {
    run_bounded_with_assignment(program, args, control, false)
}

/// Internal entry point with an assignment seam so tests can force the
/// Job Object assignment-failure path. Production always calls with `false`.
fn run_bounded_with_assignment(
    program: &str,
    args: &[String],
    control: &RunControl,
    force_assignment_failure: bool,
) -> std::io::Result<BoundedOutput> {
    #[cfg(windows)]
    {
        windows_run_bounded(program, args, control, force_assignment_failure)
    }
    #[cfg(not(windows))]
    {
        let _ = (control, force_assignment_failure);
        let output = Command::new(program).args(args).output()?;
        Ok(BoundedOutput {
            exit_code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            timed_out: false,
            cancelled: false,
            containment: TreeContainment::NotApplicable,
        })
    }
}

#[cfg(windows)]
fn windows_run_bounded(
    program: &str,
    args: &[String],
    control: &RunControl,
    force_assignment_failure: bool,
) -> std::io::Result<BoundedOutput> {
    use std::os::windows::io::AsRawHandle;
    use std::os::windows::process::CommandExt;
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    // JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE ensures the tree dies even if this
    // process exits abnormally while the repair is still running.
    let job: HANDLE = unsafe { CreateJobObjectW(None, None) }?;
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    unsafe {
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            std::ptr::from_ref(&limits).cast(),
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )?;
    }

    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW);
    let mut child = command.spawn()?;

    // Assign immediately after spawn.  The child is a trusted Windows binary,
    // so the residual window between spawn and assignment cannot be used to
    // smuggle a descendant out of the job in practice.
    let tree_contained = if force_assignment_failure {
        false
    } else {
        unsafe { AssignProcessToJobObject(job, HANDLE(child.as_raw_handle())) }.is_ok()
    };
    let containment = if tree_contained {
        TreeContainment::Contained
    } else {
        TreeContainment::Unavailable
    };

    let stdout_reader = child.stdout.take().map(spawn_capped_reader);
    let stderr_reader = child.stderr.take().map(spawn_capped_reader);

    let deadline = Instant::now() + control.timeout();
    let mut timed_out = false;
    let mut cancelled = false;
    let status = loop {
        if control.is_cancelled() {
            cancelled = true;
            break None;
        }
        if Instant::now() >= deadline {
            timed_out = true;
            break None;
        }
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => std::thread::sleep(POLL_INTERVAL),
            Err(error) => return Err(error),
        }
    };

    let exit_code = status.and_then(|status| status.code());
    if status.is_none() {
        // Terminate the whole job; fall back to the direct child if the job
        // assignment was refused (for example a runner-owned parent job).
        unsafe {
            let _ = TerminateJobObject(job, 1);
        }
        if !tree_contained {
            let _ = child.kill();
        }
        let _ = child.wait();
    }

    let stdout = stdout_reader.map(join_capped_reader).unwrap_or_default();
    let stderr = stderr_reader.map(join_capped_reader).unwrap_or_default();
    unsafe {
        let _ = CloseHandle(job);
    }

    Ok(BoundedOutput {
        exit_code,
        stdout,
        stderr,
        timed_out,
        cancelled,
        containment,
    })
}

fn spawn_capped_reader(mut reader: impl Read + Send + 'static) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut buffer = [0_u8; 16 * 1024];
        let mut captured = Vec::new();
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    if captured.len() < MAX_CAPTURED_BYTES {
                        let remaining = MAX_CAPTURED_BYTES - captured.len();
                        captured.extend_from_slice(&buffer[..read.min(remaining)]);
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        String::from_utf8_lossy(&captured).into_owned()
    })
}

fn join_capped_reader(handle: std::thread::JoinHandle<String>) -> String {
    handle.join().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn control(timeout: Duration) -> RunControl {
        RunControl::new(Arc::new(AtomicBool::new(false)), timeout)
    }

    fn cancel_after(delay: Duration) -> Arc<AtomicBool> {
        let cancel = Arc::new(AtomicBool::new(false));
        let trigger = Arc::clone(&cancel);
        std::thread::spawn(move || {
            std::thread::sleep(delay);
            trigger.store(true, Ordering::Relaxed);
        });
        cancel
    }

    /// A parent Node process that spawns a descendant Node process which writes
    /// `marker` after a delay, then the parent idles. The descendant is spawned
    /// detached so it survives the parent being terminated unless a Job Object
    /// owns the tree; if the Job Object owns it, terminating the job kills both
    /// and the marker never appears.
    #[cfg(windows)]
    fn descendant_marker_command(marker: &std::path::Path) -> (&'static str, Vec<String>) {
        let marker_path = marker.to_string_lossy().to_string();
        let parent = format!(
            "const {{spawn}}=require('child_process');spawn(process.execPath,['-e','setTimeout(()=>require(\"fs\").writeFileSync(process.argv[1],\"leaked\"),4000)',{marker_path:?}],{{stdio:'ignore',detached:true,windowsHide:true}});setTimeout(()=>{{}},30000);"
        );
        ("node", vec!["-e".into(), parent])
    }

    #[cfg(windows)]
    fn marker_dir(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "winoptimizer-bounded-tree-{tag}-{}",
            std::process::id()
        ))
    }

    #[test]
    fn captures_output_and_exit_code() {
        let output = run_bounded(
            "cmd",
            &["/C".into(), "echo".into(), "bounded-ok".into()],
            &control(Duration::from_secs(30)),
        )
        .unwrap();
        assert_eq!(output.exit_code, Some(0));
        assert!(output.stdout.contains("bounded-ok"), "{}", output.stdout);
        assert!(!output.timed_out);
        assert!(!output.cancelled);
    }

    #[test]
    fn nonzero_exit_is_reported() {
        let output = run_bounded(
            "cmd",
            &["/C".into(), "exit".into(), "3".into()],
            &control(Duration::from_secs(30)),
        )
        .unwrap();
        assert_eq!(output.exit_code, Some(3));
    }

    #[test]
    fn a_timeout_terminates_the_process_tree() {
        let output = run_bounded(
            "cmd",
            &["/C".into(), "ping -n 30 127.0.0.1 >NUL".into()],
            &control(Duration::from_millis(700)),
        )
        .unwrap();
        assert!(output.timed_out);
        assert_eq!(output.exit_code, None);
        assert!(!output.cancelled);
        assert_eq!(output.termination(), ProcessTermination::TimedOut);
        #[cfg(windows)]
        assert_eq!(output.containment, TreeContainment::Contained);
    }

    #[test]
    fn cancellation_terminates_the_process_tree() {
        let output = run_bounded(
            "cmd",
            &["/C".into(), "ping -n 30 127.0.0.1 >NUL".into()],
            &RunControl::new(
                cancel_after(Duration::from_millis(300)),
                Duration::from_secs(30),
            ),
        )
        .unwrap();
        assert!(output.cancelled);
        assert_eq!(output.exit_code, None);
        assert!(!output.timed_out);
        assert_eq!(output.termination(), ProcessTermination::Cancelled);
        #[cfg(windows)]
        assert_eq!(output.containment, TreeContainment::Contained);
    }

    /// A contained descendant is terminated with its parent on cancellation.
    #[cfg(windows)]
    #[test]
    fn cancellation_terminates_a_contained_descendant() {
        let dir = marker_dir("contained");
        std::fs::create_dir_all(&dir).unwrap();
        let marker = dir.join("descendant-leaked.txt");
        let (program, args) = descendant_marker_command(&marker);
        let output = run_bounded(
            program,
            &args,
            &RunControl::new(
                cancel_after(Duration::from_millis(300)),
                Duration::from_secs(30),
            ),
        )
        .unwrap();
        assert!(output.cancelled);
        assert_eq!(output.containment, TreeContainment::Contained);
        // Give the descendant well past its 4s write delay to surface if it
        // survived cancellation.
        std::thread::sleep(Duration::from_secs(6));
        assert!(
            !marker.exists(),
            "a descendant owned by the Job Object must not survive cancellation"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// When Job Object assignment is refused, the direct child is terminated
    /// but descendants may survive. The structured result must say the tree
    /// was not contained, and the caller must not claim full tree termination.
    #[cfg(windows)]
    #[test]
    fn assignment_failure_reports_unavailable_containment() {
        let dir = marker_dir("unavailable");
        std::fs::create_dir_all(&dir).unwrap();
        let marker = dir.join("descendant-leaked.txt");
        let (program, args) = descendant_marker_command(&marker);
        let output = run_bounded_with_assignment(
            program,
            &args,
            &RunControl::new(
                cancel_after(Duration::from_millis(300)),
                Duration::from_secs(30),
            ),
            true,
        )
        .unwrap();
        assert!(output.cancelled);
        assert_eq!(output.containment, TreeContainment::Unavailable);
        assert_eq!(output.termination(), ProcessTermination::Cancelled);
        // The direct child is gone. The descendant may survive; we deliberately
        // do not enumerate or terminate unrelated PIDs. Waiting observes the
        // residual behavior so the test documents it truthfully.
        std::thread::sleep(Duration::from_secs(6));
        assert!(
            marker.exists(),
            "with containment unavailable the descendant may survive; the result must not claim containment"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
