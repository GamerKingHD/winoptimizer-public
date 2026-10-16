//! Thin wrapper around `std::process::Command` that hides console windows.
//!
//! On Windows, every `Command::new(...)` call is patched with `CREATE_NO_WINDOW`
//! so external commands (`reg`, `sc`, `schtasks`, `powercfg`, `net`, etc.) do
//! not flash a CMD window in the user's face.
//!
//! Well-known Windows tools launched by bare name (`sfc`, `netsh`, `powershell`)
//! are resolved to their absolute location under the system directory, so a
//! look-alike executable planted in the application directory or elsewhere on
//! the search path is never started in their place.
//!
//! Usage:
//! ```ignore
//! use crate::core::command_helpers::create_command;
//! let output = create_command("reg")?.arg("query", ...).output()?;
//! ```

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Tools that ship in the Windows system directory and are launched by name.
const SYSTEM_DIRECTORY_TOOLS: &[&str] = &[
    "bcdedit", "chkdsk", "cmd", "dism", "ipconfig", "msiexec", "net", "net1", "netsh", "nslookup",
    "ping", "powercfg", "reg", "sc", "schtasks", "sfc", "taskkill", "tasklist", "tracert",
    "wevtutil",
];

/// Create a `Command` for the given program, with the console window hidden.
///
/// On Windows this calls `.creation_flags(CREATE_NO_WINDOW)` so no console
/// window ever appears. On non-Windows it's a plain `Command::new`.
pub fn create_command(program: &str) -> Command {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut cmd = Command::new(resolve_system_program(program));
        cmd.creation_flags(CREATE_NO_WINDOW);
        cmd
    }
    #[cfg(not(target_os = "windows"))]
    {
        Command::new(program)
    }
}

/// Resolve a bare Windows tool name to its absolute system-directory path.
///
/// Anything that is not a known tool, already contains a path, or is missing
/// from the system directory is returned unchanged, which keeps the previous
/// launch behavior for those cases.
pub fn resolve_system_program(program: &str) -> OsString {
    #[cfg(windows)]
    if let Some(directory) = system_directory() {
        if let Some(path) = resolve_in(&directory, program, &|candidate| candidate.is_file()) {
            return path.into_os_string();
        }
    }
    program.into()
}

#[cfg(windows)]
fn system_directory() -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows::Win32::System::SystemInformation::GetSystemDirectoryW;

    let mut buffer = [0u16; 260];
    let length = unsafe { GetSystemDirectoryW(Some(&mut buffer)) } as usize;
    if length == 0 || length >= buffer.len() {
        return None;
    }
    Some(PathBuf::from(OsString::from_wide(&buffer[..length])))
}

#[cfg_attr(not(windows), allow(dead_code))]
fn resolve_in(
    system_directory: &Path,
    program: &str,
    exists: &dyn Fn(&Path) -> bool,
) -> Option<PathBuf> {
    if program.contains(['\\', '/', ':']) {
        return None;
    }
    let lowered = program.to_ascii_lowercase();
    let name = lowered.strip_suffix(".exe").unwrap_or(&lowered);
    let candidate = match name {
        "powershell" => system_directory
            .join("WindowsPowerShell")
            .join("v1.0")
            .join("powershell.exe"),
        known if SYSTEM_DIRECTORY_TOOLS.contains(&known) => {
            system_directory.join(format!("{known}.exe"))
        }
        _ => return None,
    };
    exists(&candidate).then_some(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn present(_: &Path) -> bool {
        true
    }

    #[test]
    fn known_tools_resolve_case_insensitively_into_the_system_directory() {
        let directory = Path::new("C:\\Fake\\System32");
        for name in ["sfc", "SFC", "Sfc.EXE", "netsh.exe", "DISM"] {
            let resolved = resolve_in(directory, name, &present).unwrap();
            assert_eq!(resolved.parent(), Some(directory), "{name}");
            assert_eq!(
                resolved.extension().and_then(|e| e.to_str()),
                Some("exe"),
                "{name}"
            );
        }
    }

    #[test]
    fn powershell_resolves_to_its_versioned_directory() {
        let directory = Path::new("C:\\Fake\\System32");
        let resolved = resolve_in(directory, "powershell.exe", &present).unwrap();
        assert_eq!(
            resolved,
            directory
                .join("WindowsPowerShell")
                .join("v1.0")
                .join("powershell.exe")
        );
    }

    #[test]
    fn explicit_paths_and_unknown_programs_are_left_alone() {
        let directory = Path::new("C:\\Fake\\System32");
        for name in [
            "C:\\Tools\\sfc.exe",
            "..\\sfc.exe",
            "sub/sfc.exe",
            "C:sfc",
            "git",
            "dism_api",
            "",
        ] {
            assert!(resolve_in(directory, name, &present).is_none(), "{name:?}");
        }
    }

    #[test]
    fn a_tool_missing_from_the_system_directory_keeps_the_bare_name() {
        let directory = Path::new("C:\\Fake\\System32");
        assert!(resolve_in(directory, "sfc", &|_| false).is_none());
    }

    #[cfg(windows)]
    #[test]
    fn real_system_tools_resolve_to_existing_absolute_files() {
        let directory = system_directory().expect("system directory");
        for name in ["cmd", "sfc", "netsh", "powershell"] {
            let resolved = PathBuf::from(resolve_system_program(name));
            assert!(resolved.is_absolute(), "{name} -> {resolved:?}");
            assert!(resolved.is_file(), "{name} -> {resolved:?}");
            assert!(
                resolved
                    .to_string_lossy()
                    .to_ascii_lowercase()
                    .starts_with(&directory.to_string_lossy().to_ascii_lowercase()),
                "{name} -> {resolved:?}"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn create_command_launches_the_resolved_system_tool() {
        let command = create_command("cmd");
        let program = PathBuf::from(command.get_program());
        assert!(program.is_absolute(), "{program:?}");
    }
}
