//! Thin wrapper around `std::process::Command` that hides console windows.
//!
//! On Windows, every `Command::new(...)` call is patched with `CREATE_NO_WINDOW`
//! so external commands (`reg`, `sc`, `schtasks`, `powercfg`, `net`, etc.) do
//! not flash a CMD window in the user's face.
//!
//! Usage:
//! ```ignore
//! use crate::core::command_helpers::create_command;
//! let output = create_command("reg")?.arg("query", ...).output()?;
//! ```

use std::process::Command;

/// Create a `Command` for the given program, with the console window hidden.
///
/// On Windows this calls `.creation_flags(CREATE_NO_WINDOW)` so no console
/// window ever appears. On non-Windows it's a plain `Command::new`.
pub fn create_command(program: &str) -> Command {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut cmd = Command::new(program);
        cmd.creation_flags(CREATE_NO_WINDOW);
        cmd
    }
    #[cfg(not(target_os = "windows"))]
    {
        Command::new(program)
    }
}
