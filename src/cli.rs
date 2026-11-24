//! Command-line parsing.

use std::path::{Path, PathBuf};

pub const USAGE: &str = "\
Usage: rs-cpuz [options]

  (no option)      open the window
  -txt=NAME        write the text report to NAME.txt and exit
  -html=NAME       write the HTML report to NAME.htm and exit
  -bench           run the benchmark without a window and save the scores
  -core=N          show the clocks of core N on the CPU tab
  -snap=DIR        render every tab to a .bmp file in DIR and exit
  -dump            print the detected hardware model and exit
  -demo            use the built-in sample system instead of probing hardware
  -noadmin         do not ask for administrator rights when the window opens
  -help            show this text

A relative NAME or DIR is resolved next to the executable.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Gui,
    Txt(String),
    Html(String),
    Bench,
    Snap(String),
    Dump,
    Help,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    pub command: Command,
    pub core: Option<u32>,
    pub demo: bool,
    /// Do not ask for elevation when the window opens.
    pub no_admin: bool,
}

/// Parses the arguments after the program name. Options start with `-`, `--` or `/`
/// and are case-insensitive.
pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Args, String> {
    let mut parsed = Args { command: Command::Gui, core: None, demo: false, no_admin: false };
    for arg in args {
        let body = arg
            .strip_prefix("--")
            .or_else(|| arg.strip_prefix('-'))
            .or_else(|| arg.strip_prefix('/'))
            .ok_or_else(|| format!("unexpected argument `{arg}`"))?;
        let (key, value) = match body.split_once('=') {
            Some((k, v)) => (k, Some(v.trim_matches('"'))),
            None => (body, None),
        };
        let required = || value.filter(|v| !v.is_empty()).ok_or_else(|| format!("`-{key}` needs a value"));
        match key.to_ascii_lowercase().as_str() {
            "txt" => parsed.command = Command::Txt(required()?.to_string()),
            "html" => parsed.command = Command::Html(required()?.to_string()),
            "snap" => parsed.command = Command::Snap(required()?.to_string()),
            "bench" => parsed.command = Command::Bench,
            "dump" => parsed.command = Command::Dump,
            "help" | "h" | "?" => parsed.command = Command::Help,
            "demo" => parsed.demo = true,
            "noadmin" => parsed.no_admin = true,
            "core" => {
                let v = required()?;
                parsed.core = Some(v.parse().map_err(|_| format!("`{v}` is not a core number"))?);
            }
            _ => return Err(format!("unknown option `{arg}`")),
        }
    }
    Ok(parsed)
}

/// Resolves a report name: relative names live in `base`, and the first of
/// `extensions` is appended unless the name already ends with one of them.
pub fn output_path(base: &Path, name: &str, extensions: &[&str]) -> PathBuf {
    let has_extension = Path::new(name)
        .extension()
        .is_some_and(|found| extensions.iter().any(|known| found.eq_ignore_ascii_case(known)));
    let file = match extensions.first() {
        Some(default) if !has_extension => format!("{name}.{default}"),
        _ => name.to_string(),
    };
    base.join(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Result<Args, String> {
        parse(list.iter().map(|s| s.to_string()))
    }

    #[test]
    fn defaults_to_gui() {
        assert_eq!(
            args(&[]).unwrap(),
            Args { command: Command::Gui, core: None, demo: false, no_admin: false }
        );
    }

    #[test]
    fn parses_report_modes() {
        assert_eq!(args(&["-txt=report"]).unwrap().command, Command::Txt("report".into()));
        assert_eq!(args(&["--HTML=\"my pc\""]).unwrap().command, Command::Html("my pc".into()));
        assert_eq!(args(&["/bench"]).unwrap().command, Command::Bench);
        assert_eq!(
            args(&["-snap=shots", "-demo"]).unwrap(),
            Args { command: Command::Snap("shots".into()), core: None, demo: true, no_admin: false }
        );
    }

    #[test]
    fn parses_noadmin() {
        assert!(args(&["-noadmin"]).unwrap().no_admin);
        assert!(args(&["/NoAdmin", "-core=2"]).unwrap().no_admin);
        assert!(!args(&["-demo"]).unwrap().no_admin);
    }

    #[test]
    fn parses_core() {
        assert_eq!(args(&["-core=3"]).unwrap().core, Some(3));
        assert!(args(&["-core=x"]).is_err());
        assert!(args(&["-core"]).is_err());
    }

    #[test]
    fn rejects_unknown_input() {
        assert!(args(&["-frobnicate"]).is_err());
        assert!(args(&["report.txt"]).is_err());
        assert!(args(&["-txt="]).is_err());
    }

    #[test]
    fn resolves_output_paths() {
        let base = Path::new("base");
        assert_eq!(output_path(base, "report", &["txt"]), base.join("report.txt"));
        assert_eq!(output_path(base, "report.TXT", &["txt"]), base.join("report.TXT"));
        assert_eq!(output_path(base, "v1.2", &["htm", "html"]), base.join("v1.2.htm"));
        let absolute = if cfg!(windows) { r"C:\out\r" } else { "/out/r" };
        assert_eq!(output_path(base, absolute, &["txt"]), PathBuf::from(format!("{absolute}.txt")));
    }

    #[test]
    fn keeps_either_html_extension() {
        let base = Path::new("base");
        let html = &["htm", "html"];
        assert_eq!(output_path(base, "x", html), base.join("x.htm"));
        assert_eq!(output_path(base, "x.htm", html), base.join("x.htm"));
        assert_eq!(output_path(base, "x.html", html), base.join("x.html"));
        assert_eq!(output_path(base, "x.HTML", html), base.join("x.HTML"));
        // Another report's extension is part of the name.
        assert_eq!(output_path(base, "x.txt", html), base.join("x.txt.htm"));
        assert_eq!(output_path(base, "x.html", &["txt"]), base.join("x.html.txt"));
    }
}
