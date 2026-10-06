//! Command-line flags. With none, todoro opens as usual.

use std::path::Path;

use crate::changelog::VERSION;
use crate::options::Settings;

pub const HELP: &str = "\
todoro: a vim-style terminal todo app

Usage: todoro [option]

With no option, todoro opens. Press ? inside it for every key.

Options:
  --where      Print the todoro folder, where every workspace is kept
  --version    Print the version
  --help       Print this help

Environment:
  TODORO_SETTINGS  Use this settings file
  TODORO_FILE      Use this single todos file, with no folder or workspaces
  NO_COLOR         Start with colours off";

/// What to do for the command line.
#[derive(Debug, PartialEq)]
pub enum Run {
    /// Open todoro.
    App,
    /// Print this and stop.
    Print(String),
    /// Print this error and stop with this exit code.
    Fail(String, i32),
}

/// What to do for `args` (without the program name), given the settings
/// and `TODORO_FILE`.
pub fn parse(args: &[String], settings: &Settings, todoro_file: Option<&Path>) -> Run {
    match args {
        [] => Run::App,
        [flag] => match flag.as_str() {
            "--version" | "-V" => Run::Print(format!("todoro {VERSION}")),
            "--help" | "-h" => Run::Print(HELP.to_string()),
            "--where" => match (todoro_file, &settings.data_dir) {
                (Some(file), _) => Run::Print(file.display().to_string()),
                (None, Some(dir)) => Run::Print(dir.display().to_string()),
                (None, None) => Run::Fail("todoro isn't set up yet. Run todoro to choose a folder.".into(), 1),
            },
            other => Run::Fail(format!("todoro: unknown option {other}\n\n{HELP}"), 2),
        },
        _ => Run::Fail(format!("todoro: give at most one option\n\n{HELP}"), 2),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn run(args: &[&str], settings: &Settings, file: Option<&Path>) -> Run {
        parse(&args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>(), settings, file)
    }

    #[test]
    fn no_options_opens_todoro() {
        assert_eq!(run(&[], &Settings::default(), None), Run::App);
    }

    #[test]
    fn version_and_help() {
        assert_eq!(run(&["--version"], &Settings::default(), None), Run::Print(format!("todoro {VERSION}")));
        assert_eq!(run(&["-V"], &Settings::default(), None), Run::Print(format!("todoro {VERSION}")));
        let Run::Print(help) = run(&["--help"], &Settings::default(), None) else { panic!("help") };
        assert!(help.contains("--where"));
        assert_eq!(run(&["-h"], &Settings::default(), None), Run::Print(help));
    }

    #[test]
    fn where_prints_just_the_folder() {
        let settings = Settings { data_dir: Some(PathBuf::from("/home/sam/todoro")), ..Settings::default() };
        assert_eq!(run(&["--where"], &settings, None), Run::Print("/home/sam/todoro".into()));
        // A single file, when TODORO_FILE is set.
        let file = PathBuf::from("/tmp/todos.json");
        assert_eq!(run(&["--where"], &settings, Some(&file)), Run::Print("/tmp/todos.json".into()));
    }

    #[test]
    fn where_before_setup_fails() {
        let Run::Fail(message, 1) = run(&["--where"], &Settings::default(), None) else { panic!("should fail") };
        assert!(message.contains("set up"));
    }

    #[test]
    fn unknown_or_extra_options_fail_with_the_help() {
        let Run::Fail(message, 2) = run(&["--nope"], &Settings::default(), None) else { panic!("should fail") };
        assert!(message.contains("--nope") && message.contains("Usage"));
        assert!(matches!(run(&["--version", "--help"], &Settings::default(), None), Run::Fail(_, 2)));
    }
}
