//! Command-line flags. With none, todoro opens as usual.

use std::path::Path;

use crate::changelog::VERSION;
use crate::options::Settings;

pub const HELP: &str = "\
todoro: a vim-style terminal todo app

Usage: todoro [option]

With no option, todoro opens. Press ? inside it for every key.

Options:
  -l, --list                 Print today's list
  -a, --add TEXT             Add an item to today's list
  -a TEXT -p N               Add it as a sub-item of item N (as -l numbers it)
  -wl, --workspaces          List the workspaces, * marking the current one
  -w, --workspace NAME       Switch to a workspace: todoro and -l use it
  --where                    Print the todoro folder, where every workspace is kept
  --version                  Print the version
  --help                     Print this help

Keys:
  Change any key in keys.toml (KEYS). Inside todoro, the options (Space o)
  open it in your editor, with every default listed.

Environment:
  TODORO_SETTINGS  Use this settings file
  TODORO_KEYS      Use this keys file
  TODORO_FILE      Use this single todos file, with no folder or workspaces
  NO_COLOR         Start with colours off";

/// The help, with where the keys file is.
fn help(keys: Option<&Path>) -> String {
    let keys = keys.map_or_else(|| "beside the settings".to_string(), |path| path.display().to_string());
    HELP.replace("(KEYS)", &format!("({keys})"))
}

/// What to do for the command line.
#[derive(Debug, PartialEq)]
pub enum Run {
    /// Open todoro.
    App,
    /// Print today's list and stop.
    List,
    /// Add an item to today's list, under the open item numbered `parent`
    /// if there is one, and stop.
    Add { text: String, parent: Option<usize> },
    /// List the workspaces and stop.
    Workspaces,
    /// Switch to this workspace (`None` when no name was given) and stop.
    Switch(Option<String>),
    /// Print this and stop.
    Print(String),
    /// Print this error and stop with this exit code.
    Fail(String, i32),
}

/// What to do for `args` (without the program name), given the settings,
/// `TODORO_FILE` and where the keys file is.
pub fn parse(args: &[String], settings: &Settings, todoro_file: Option<&Path>, keys: Option<&Path>) -> Run {
    let workspace_flag = |flag: &str| matches!(flag, "-wl" | "--workspaces" | "-w" | "--workspace");
    match args.first().map(String::as_str) {
        // Workspaces only exist in the todoro folder.
        Some(flag) if workspace_flag(flag) && todoro_file.is_some() => {
            return Run::Fail("TODORO_FILE is set, so todoro uses just that file, with no workspaces.".into(), 1);
        }
        Some(flag) if workspace_flag(flag) && settings.data_dir.is_none() => {
            return Run::Fail("todoro isn't set up yet. Run todoro to choose a folder.".into(), 1);
        }
        _ => {}
    }
    if let Some(flag) = args.first()
        && (flag == "-a" || flag == "--add")
    {
        return add(&args[1..], keys);
    }
    match args {
        [] => Run::App,
        [flag] if flag == "-w" || flag == "--workspace" => Run::Switch(None),
        [flag, name] if flag == "-w" || flag == "--workspace" => Run::Switch(Some(name.clone())),
        [flag] => match flag.as_str() {
            "--version" | "-V" => Run::Print(format!("todoro {VERSION}")),
            "--help" | "-h" => Run::Print(help(keys)),
            "--list" | "-l" if todoro_file.is_none() && settings.data_dir.is_none() => {
                Run::Fail("todoro isn't set up yet. Run todoro to choose a folder.".into(), 1)
            }
            "--list" | "-l" => Run::List,
            "--workspaces" | "-wl" => Run::Workspaces,
            "--where" => match (todoro_file, &settings.data_dir) {
                (Some(file), _) => Run::Print(file.display().to_string()),
                (None, Some(dir)) => Run::Print(dir.display().to_string()),
                (None, None) => Run::Fail("todoro isn't set up yet. Run todoro to choose a folder.".into(), 1),
            },
            other => Run::Fail(format!("todoro: unknown option {other}\n\n{}", help(keys)), 2),
        },
        _ => Run::Fail(format!("todoro: give at most one option\n\n{}", help(keys)), 2),
    }
}

/// `-a TEXT`, with `-p N` (or `--parent N`) anywhere after it for a
/// sub-item. The words left are the text, joined as typed apart; after `--`
/// every word is text, even `-p`.
fn add(args: &[String], keys: Option<&Path>) -> Run {
    let mut words: Vec<&str> = Vec::new();
    let mut parent = None;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--" => {
                words.extend(rest.by_ref().map(String::as_str));
            }
            "-p" | "--parent" => match rest.next().map(|number| number.parse::<usize>()) {
                Some(Ok(number)) => parent = Some(number),
                Some(Err(_)) | None => {
                    return Run::Fail("todoro: -p takes the number of the item to add under, like: todoro -a Pack -p 2".into(), 2);
                }
            },
            word => words.push(word),
        }
    }
    let text = words.join(" ");
    if text.trim().is_empty() {
        return Run::Fail(format!("todoro: give the item's text, like: todoro -a Buy milk\n\n{}", help(keys)), 2);
    }
    Run::Add { text, parent }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn run(args: &[&str], settings: &Settings, file: Option<&Path>) -> Run {
        let keys = Path::new("/home/sam/.config/todoro/keys.toml");
        parse(&args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>(), settings, file, Some(keys))
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
        assert!(help.contains("keys.toml (/home/sam/.config/todoro/keys.toml)"), "{help}");
        assert!(help.contains("TODORO_KEYS"));
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
    fn list_prints_todays_list_once_set_up() {
        let settings = Settings { data_dir: Some(PathBuf::from("/home/sam/todoro")), ..Settings::default() };
        assert_eq!(run(&["--list"], &settings, None), Run::List);
        assert_eq!(run(&["-l"], &settings, None), Run::List);
        let file = PathBuf::from("/tmp/todos.json");
        assert_eq!(run(&["-l"], &Settings::default(), Some(&file)), Run::List);
        let Run::Fail(message, 1) = run(&["-l"], &Settings::default(), None) else { panic!("should fail") };
        assert!(message.contains("set up"));
        let Run::Print(help) = run(&["--help"], &settings, None) else { panic!("help") };
        assert!(help.contains("-l, --list"));
    }

    #[test]
    fn workspace_flags() {
        let settings = Settings { data_dir: Some(PathBuf::from("/home/sam/todoro")), ..Settings::default() };
        assert_eq!(run(&["-wl"], &settings, None), Run::Workspaces);
        assert_eq!(run(&["--workspaces"], &settings, None), Run::Workspaces);
        assert_eq!(run(&["-w", "work"], &settings, None), Run::Switch(Some("work".into())));
        assert_eq!(run(&["--workspace", "my work"], &settings, None), Run::Switch(Some("my work".into())));
        assert_eq!(run(&["-w"], &settings, None), Run::Switch(None));
        // A name goes only with -w.
        assert!(matches!(run(&["-wl", "work"], &settings, None), Run::Fail(_, 2)));
        assert!(matches!(run(&["-w", "work", "home"], &settings, None), Run::Fail(_, 2)));
        let Run::Print(help) = run(&["--help"], &settings, None) else { panic!("help") };
        assert!(help.contains("-wl, --workspaces") && help.contains("-w, --workspace NAME"));
    }

    fn add(args: &[&str]) -> Run {
        run(args, &Settings::default(), None)
    }

    #[test]
    fn add_takes_the_rest_as_text() {
        assert_eq!(add(&["-a", "Buy", "milk"]), Run::Add { text: "Buy milk".into(), parent: None });
        assert_eq!(add(&["--add", "Buy milk"]), Run::Add { text: "Buy milk".into(), parent: None });
    }

    #[test]
    fn p_adds_under_a_numbered_item_wherever_it_goes() {
        let flights = Run::Add { text: "Book flights".into(), parent: Some(2) };
        assert_eq!(add(&["-a", "Book", "flights", "-p", "2"]), flights);
        assert_eq!(add(&["-a", "-p", "2", "Book", "flights"]), flights);
        assert_eq!(add(&["-a", "Book", "--parent", "2", "flights"]), flights);
        // After --, -p is text.
        assert_eq!(add(&["-a", "--", "Check", "-p", "2"]), Run::Add { text: "Check -p 2".into(), parent: None });
        assert_eq!(add(&["-a", "Pack", "--", "-p", "2"]), Run::Add { text: "Pack -p 2".into(), parent: None });
    }

    #[test]
    fn add_needs_text_and_p_a_number() {
        assert!(matches!(add(&["-a"]), Run::Fail(message, 2) if message.contains("item's text")));
        assert!(matches!(add(&["-a", "-p", "2"]), Run::Fail(message, 2) if message.contains("item's text")));
        assert!(matches!(add(&["-a", "Pack", "-p"]), Run::Fail(message, 2) if message.contains("number")));
        assert!(matches!(add(&["-a", "Pack", "-p", "two"]), Run::Fail(message, 2) if message.contains("number")));
        let Run::Print(help) = add(&["--help"]) else { panic!("help") };
        assert!(help.contains("-a, --add TEXT") && help.contains("-p N"));
    }

    #[test]
    fn workspace_flags_need_the_todoro_folder() {
        let Run::Fail(message, 1) = run(&["-wl"], &Settings::default(), None) else { panic!("should fail") };
        assert!(message.contains("set up"));
        let settings = Settings { data_dir: Some(PathBuf::from("/home/sam/todoro")), ..Settings::default() };
        let file = PathBuf::from("/tmp/todos.json");
        let Run::Fail(message, 1) = run(&["-w", "work"], &settings, Some(&file)) else { panic!("should fail") };
        assert!(message.contains("TODORO_FILE"));
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
