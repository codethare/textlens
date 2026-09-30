//! Select a region of a Wayland screen, OCR the text inside it, copy it to the clipboard.
//!
//! Pipeline, entirely in memory:
//! `slurp` -> `grim -g - -` -> `tesseract stdin stdout -l <lang>` -> `wl-copy`
//!
//! The region can also be named without the pointer (`--full`, `--focused`,
//! `--geometry`) for keyboard-only use.

use std::io::{self, Write};
use std::process::{Command, Output, Stdio};

const DEFAULT_LANG: &str = "eng";

const USAGE: &str = "\
textlens - copy the text inside a screen region to the clipboard

USAGE:
    textlens [options]

OPTIONS:
    -l, --lang <LANG>      tesseract language(s), e.g. eng+chi_sim (default: eng)
    -g, --geometry <REG>   capture this region instead of picking one, e.g. '20,10 640x480'
    -f, --focused          capture the focused window (sway + jq, no pointer)
        --full             capture every output (no pointer)
    -h, --help             print this help

Without --geometry, --focused or --full, the region is picked with the pointer.
Requires: slurp, grim, tesseract, wl-copy; swaymsg + jq only for --focused
Exit codes: 0 copied, 1 cancelled or failed, 2 bad arguments
";

struct Args {
    lang: String,
    region: Region,
}

enum Region {
    /// Pick with the pointer, via slurp.
    Pick,
    Geometry(String),
    Focused,
    Full,
}

enum Failure {
    Cancelled,
    Failed(String),
}

fn fail(msg: impl Into<String>) -> Failure {
    Failure::Failed(msg.into())
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = match parse_args(&argv) {
        // `None` means help was requested.
        Ok(None) => {
            print!("{USAGE}");
            return;
        }
        Ok(Some(args)) => args,
        Err(msg) => {
            eprintln!("textlens: {msg}\n\n{USAGE}");
            std::process::exit(2);
        }
    };

    match capture_text(&args) {
        Ok(()) => {}
        Err(Failure::Cancelled) => std::process::exit(1),
        Err(Failure::Failed(msg)) => {
            eprintln!("textlens: {msg}");
            std::process::exit(1);
        }
    }
}

/// `Ok(None)` requests help, `Err` is a usage error.
fn parse_args(argv: &[String]) -> Result<Option<Args>, String> {
    let mut lang = DEFAULT_LANG.to_string();
    let mut region = None;
    let mut argv = argv.iter();
    while let Some(arg) = argv.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(None),
            "-l" | "--lang" => {
                lang = argv.next().ok_or("--lang requires a value")?.clone();
            }
            "-g" | "--geometry" => {
                let value = argv.next().ok_or("--geometry requires a value")?.clone();
                set_region(&mut region, Region::Geometry(value))?;
            }
            "-f" | "--focused" => set_region(&mut region, Region::Focused)?,
            "--full" => set_region(&mut region, Region::Full)?,
            other => match other.strip_prefix("--lang=") {
                Some(value) => lang = value.to_string(),
                None => match other.strip_prefix("--geometry=") {
                    Some(value) => set_region(&mut region, Region::Geometry(value.to_string()))?,
                    None => return Err(format!("unknown argument: {other}")),
                },
            },
        }
    }
    Ok(Some(Args {
        lang,
        region: region.unwrap_or(Region::Pick),
    }))
}

fn set_region(slot: &mut Option<Region>, region: Region) -> Result<(), String> {
    if slot.is_some() {
        return Err("--geometry, --focused and --full are mutually exclusive".into());
    }
    *slot = Some(region);
    Ok(())
}

fn capture_text(args: &Args) -> Result<(), Failure> {
    let geometry = match &args.region {
        Region::Pick => match slurp()? {
            Some(geometry) => Some(geometry),
            None => return Err(Failure::Cancelled),
        },
        Region::Geometry(geometry) => Some(geometry.clone()),
        Region::Focused => Some(focused_window()?),
        Region::Full => None,
    };
    let png = grim(geometry.as_deref())?;
    let raw = tesseract(&png, &args.lang)?;
    let text = cleanup(&raw).ok_or_else(|| fail("no text recognised in the selection"))?;
    copy(&text)
}

/// Screenshot region selected with the pointer. `Ok(None)` means the user cancelled.
fn slurp() -> Result<Option<String>, Failure> {
    let out = Command::new("slurp")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| run_error("slurp", e))?;
    if out.status.success() {
        let geometry = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if geometry.is_empty() {
            return Err(fail("slurp returned no geometry"));
        }
        return Ok(Some(geometry));
    }
    // slurp reports a pointer/Escape cancel as "selection cancelled" on stderr.
    if String::from_utf8_lossy(&out.stderr).contains("selection cancelled") {
        return Ok(None);
    }
    Err(fail(format!("slurp failed: {}", stderr_or_status(&out))))
}

/// Focused window geometry, using the `swaymsg` + `jq` one-liner from grim's README.
fn focused_window() -> Result<String, Failure> {
    const FILTER: &str =
        r#".. | select(.type?) | select(.focused).rect | "\(.x),\(.y) \(.width)x\(.height)""#;
    let tree = run_capture("swaymsg", &["-t", "get_tree"])?;
    let mut cmd = Command::new("jq");
    cmd.args(["-j", FILTER]);
    let out = run_piped(&mut cmd, &tree).map_err(|e| run_error("jq", e))?;
    if !out.status.success() {
        return Err(fail(format!("jq failed: {}", stderr_or_status(&out))));
    }
    let geometry = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if geometry.is_empty() {
        return Err(fail("sway reported no focused window"));
    }
    Ok(geometry)
}

/// PNG on stdout. `None` captures every output, `Some` only that geometry.
fn grim(geometry: Option<&str>) -> Result<Vec<u8>, Failure> {
    let mut cmd = Command::new("grim");
    let input = match geometry {
        Some(geometry) => {
            cmd.args(["-g", "-", "-"]);
            geometry.as_bytes()
        }
        None => {
            cmd.arg("-");
            b""
        }
    };
    let out = run_piped(&mut cmd, input).map_err(|e| run_error("grim", e))?;
    if !out.status.success() {
        return Err(fail(format!("grim failed: {}", stderr_or_status(&out))));
    }
    if out.stdout.is_empty() {
        return Err(fail("grim produced no image"));
    }
    Ok(out.stdout)
}

fn tesseract(png: &[u8], lang: &str) -> Result<String, Failure> {
    let mut cmd = Command::new("tesseract");
    cmd.args(["stdin", "stdout", "-l", lang]);
    let out = run_piped(&mut cmd, png).map_err(|e| run_error("tesseract", e))?;
    if !out.status.success() {
        return Err(fail(format!(
            "tesseract failed: {}",
            stderr_or_status(&out)
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn copy(text: &str) -> Result<(), Failure> {
    let mut cmd = Command::new("wl-copy");
    let out = run_piped(&mut cmd, text.as_bytes()).map_err(|e| run_error("wl-copy", e))?;
    if !out.status.success() {
        return Err(fail(format!("wl-copy failed: {}", stderr_or_status(&out))));
    }
    Ok(())
}

/// Tesseract pads its output with newlines; nothing left means nothing was recognised.
fn cleanup(text: &str) -> Option<String> {
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// Run `cmd` with `input` on stdin, collecting both output streams.
fn run_piped(cmd: &mut Command, input: &[u8]) -> io::Result<Output> {
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdin = child.stdin.take().expect("stdin is piped");
    let input = input.to_vec();
    // Feed stdin from a thread: the child can fill its stdout pipe while we are still
    // writing, and both pipes are only drained by `wait_with_output`.
    let writer = std::thread::spawn(move || stdin.write_all(&input));
    let out = child.wait_with_output()?;
    // EPIPE is expected when the child exits without reading all of stdin.
    let _ = writer.join();
    Ok(out)
}

fn run_capture(tool: &str, args: &[&str]) -> Result<Vec<u8>, Failure> {
    let out = Command::new(tool)
        .args(args)
        .output()
        .map_err(|e| run_error(tool, e))?;
    if !out.status.success() {
        return Err(fail(format!("{tool} failed: {}", stderr_or_status(&out))));
    }
    Ok(out.stdout)
}

fn run_error(tool: &str, err: io::Error) -> Failure {
    fail(format!("cannot run {tool}: {err}"))
}

fn stderr_or_status(out: &Output) -> String {
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stderr = stderr.trim();
    if stderr.is_empty() {
        out.status.to_string()
    } else {
        stderr.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Option<Args>, String> {
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        parse_args(&args)
    }

    #[test]
    fn language_defaults_to_english() {
        assert_eq!(parse(&[]).unwrap().unwrap().lang, DEFAULT_LANG);
    }

    #[test]
    fn language_flag_forms() {
        assert_eq!(parse(&["-l", "jpn"]).unwrap().unwrap().lang, "jpn");
        assert_eq!(
            parse(&["--lang", "eng+chi_sim"]).unwrap().unwrap().lang,
            "eng+chi_sim"
        );
        assert_eq!(parse(&["--lang=deu"]).unwrap().unwrap().lang, "deu");
    }

    #[test]
    fn help_and_usage_errors() {
        assert!(parse(&["-h"]).unwrap().is_none());
        assert!(parse(&["--help"]).unwrap().is_none());
        assert!(parse(&["--lang"]).is_err());
        assert!(parse(&["--nope"]).is_err());
    }

    #[test]
    fn region_defaults_to_pointer_pick() {
        assert!(matches!(parse(&[]).unwrap().unwrap().region, Region::Pick));
    }

    #[test]
    fn keyboard_region_flags() {
        assert!(matches!(
            parse(&["--full"]).unwrap().unwrap().region,
            Region::Full
        ));
        assert!(matches!(
            parse(&["-f"]).unwrap().unwrap().region,
            Region::Focused
        ));
        let geometry = parse(&["-g", "20,10 640x480"]).unwrap().unwrap().region;
        assert!(matches!(geometry, Region::Geometry(g) if g == "20,10 640x480"));
        assert!(parse(&["--geometry"]).is_err());
        assert!(parse(&["-g", "1,2 3x4", "--full"]).is_err());
    }

    #[test]
    fn run_piped_handles_payload_larger_than_pipe_buffer() {
        // `cat` echoes to stdout while we are still writing stdin: this deadlocks
        // unless the write is moved off the reading thread.
        let input = vec![b'x'; 1 << 20];
        let out = run_piped(&mut Command::new("cat"), &input).unwrap();
        assert!(out.status.success());
        assert_eq!(out.stdout.len(), input.len());
    }

    #[test]
    fn cleanup_trims_padding_and_rejects_empty() {
        assert_eq!(cleanup("\n \n\t"), None);
        assert_eq!(cleanup(""), None);
        assert_eq!(cleanup("hello\n\n").as_deref(), Some("hello"));
        assert_eq!(cleanup("  a b\n c \n").as_deref(), Some("a b\n c"));
    }
}
