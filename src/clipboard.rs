//! Copying text to the system clipboard, for `y` in the viewer. It uses the
//! copy command each system has (`clip` on Windows, `pbcopy` on macOS,
//! `wl-copy`, `xclip` or `xsel` elsewhere), and with none of them, asks the
//! terminal to copy it (OSC 52), which works over SSH in many terminals.

use std::io::{self, Write};
use std::process::{Command, Stdio};

/// Where copied text goes.
pub enum Clipboard {
    System,
    /// Kept in memory, for tests, so they never touch the real clipboard.
    #[cfg(test)]
    Memory(Vec<String>),
}

/// How the text was copied.
#[derive(Debug, PartialEq)]
pub enum Copied {
    /// By the system's copy command.
    System,
    /// Asked of the terminal, which may or may not allow it.
    Terminal,
}

impl Clipboard {
    pub fn copy(&mut self, text: &str) -> io::Result<Copied> {
        match self {
            Clipboard::System => copy_to_system(text),
            #[cfg(test)]
            Clipboard::Memory(copied) => {
                copied.push(text.to_string());
                Ok(Copied::System)
            }
        }
    }

    /// What's been copied, newest last, in tests.
    #[cfg(test)]
    pub fn copied(&self) -> &[String] {
        match self {
            Clipboard::System => &[],
            Clipboard::Memory(copied) => copied,
        }
    }
}

/// A copy command and its arguments.
type Tool = (&'static str, &'static [&'static str]);

/// The copy commands to try, in order, on `os` (as `std::env::consts::OS`)
/// with or without a Wayland or X display.
fn tools(os: &str, wayland: bool, x11: bool) -> Vec<Tool> {
    match os {
        "windows" => vec![("clip", &[])],
        "macos" => vec![("pbcopy", &[])],
        _ => {
            let mut tools: Vec<Tool> = Vec::new();
            if wayland {
                tools.push(("wl-copy", &[]));
            }
            if x11 {
                tools.push(("xclip", &["-selection", "clipboard", "-in"]));
                tools.push(("xsel", &["--clipboard", "--input"]));
            }
            tools
        }
    }
}

fn copy_to_system(text: &str) -> io::Result<Copied> {
    let set = |name: &str| std::env::var_os(name).is_some_and(|value| !value.is_empty());
    for (program, args) in tools(std::env::consts::OS, set("WAYLAND_DISPLAY"), set("DISPLAY")) {
        if run(program, args, text).is_ok() {
            return Ok(Copied::System);
        }
    }
    let mut out = io::stdout();
    out.write_all(osc52(text).as_bytes())?;
    out.flush()?;
    Ok(Copied::Terminal)
}

/// Runs a copy command with `text` as its input.
fn run(program: &str, args: &[&str], text: &str) -> io::Result<()> {
    let mut command = Command::new(program);
    command.args(args).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null());
    if program == "pbcopy" {
        // pbcopy reads its input in the locale's encoding.
        command.env("LC_CTYPE", "UTF-8");
    }
    let mut child = command.spawn()?;
    let bytes = if program == "clip" { utf16_with_bom(text) } else { text.as_bytes().to_vec() };
    child.stdin.take().expect("stdin is piped").write_all(&bytes)?;
    let status = child.wait()?;
    if status.success() { Ok(()) } else { Err(io::Error::other(format!("{program} failed"))) }
}

/// Windows' `clip` reads UTF-16 with a byte order mark as Unicode; plain
/// UTF-8 it would read in the old code page, mangling anything not ASCII.
fn utf16_with_bom(text: &str) -> Vec<u8> {
    let mut bytes = vec![0xFF, 0xFE];
    for unit in text.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    bytes
}

/// The escape sequence asking the terminal to put `text` on the clipboard.
fn osc52(text: &str) -> String {
    format!("\x1b]52;c;{}\x07", base64(text.as_bytes()))
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(tools: Vec<Tool>) -> Vec<&'static str> {
        tools.into_iter().map(|(program, _)| program).collect()
    }

    #[test]
    fn each_system_uses_its_own_copy_command() {
        assert_eq!(names(tools("windows", false, false)), ["clip"]);
        assert_eq!(names(tools("macos", false, false)), ["pbcopy"]);
        assert_eq!(names(tools("linux", true, true)), ["wl-copy", "xclip", "xsel"]);
        assert_eq!(names(tools("linux", false, true)), ["xclip", "xsel"]);
        assert_eq!(names(tools("freebsd", true, false)), ["wl-copy"]);
        // With no display, as over SSH, only the terminal can copy.
        assert!(tools("linux", false, false).is_empty());
    }

    #[test]
    fn clip_gets_utf16_with_a_byte_order_mark() {
        assert_eq!(utf16_with_bom("a✓"), [0xFF, 0xFE, b'a', 0, 0x13, 0x27]);
    }

    #[test]
    fn base64_pads_as_it_should() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64("✓ done".as_bytes()), "4pyTIGRvbmU=");
    }

    #[test]
    fn the_terminal_is_asked_with_osc_52() {
        assert_eq!(osc52("hi"), "\x1b]52;c;aGk=\x07");
    }

    #[test]
    fn memory_keeps_what_was_copied() {
        let mut clipboard = Clipboard::Memory(Vec::new());
        assert_eq!(clipboard.copy("one").unwrap(), Copied::System);
        clipboard.copy("two").unwrap();
        assert_eq!(clipboard.copied(), ["one", "two"]);
    }
}
