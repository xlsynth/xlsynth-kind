use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

use tempfile::tempdir;

const DEFAULT_KIND2_IMAGE: &str =
    "kind2/kind2@sha256:b5b7c7fde98754040ea99385c83afc9339e6a244732aa2ad0f6ccf0cd517465a";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind2Runner {
    Binary(PathBuf),
    Docker { docker_bin: PathBuf, image: String },
}

impl Kind2Runner {
    /// Checks that the configured executable or Docker daemon can be reached.
    pub fn is_available(&self) -> bool {
        let (executable, argument) = match self {
            Self::Binary(path) => (path, "--version"),
            Self::Docker { docker_bin, .. } => (docker_bin, "info"),
        };
        Command::new(executable)
            .arg(argument)
            .output()
            .is_ok_and(|output| output.status.success())
    }
}

impl Default for Kind2Runner {
    fn default() -> Self {
        if let Ok(path) = std::env::var("XLSYNTH_KIND2_BIN") {
            Self::Binary(PathBuf::from(path))
        } else {
            Self::Docker {
                docker_bin: PathBuf::from(
                    std::env::var("XLSYNTH_DOCKER_BIN").unwrap_or_else(|_| "docker".to_string()),
                ),
                image: std::env::var("XLSYNTH_KIND2_IMAGE")
                    .unwrap_or_else(|_| DEFAULT_KIND2_IMAGE.to_string()),
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Kind2Options {
    pub runner: Kind2Runner,
    pub verbosity: u8,
    pub live_output: bool,
    pub print_cex: bool,
    pub extra_args: Vec<String>,
}

impl Default for Kind2Options {
    fn default() -> Self {
        Self {
            runner: Kind2Runner::default(),
            verbosity: std::env::var("XLSYNTH_KIND2_VERBOSITY")
                .ok()
                .and_then(|value| value.parse::<u8>().ok())
                .unwrap_or(0),
            live_output: std::env::var("XLSYNTH_KIND2_LIVE_OUTPUT")
                .ok()
                .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
                .unwrap_or(false),
            print_cex: std::env::var("XLSYNTH_KIND2_PRINT_CEX")
                .ok()
                .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
                .unwrap_or(false),
            extra_args: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind2CheckOutcome {
    Valid(String),
    Invalid(String),
    Unknown(String),
}

#[derive(Debug)]
pub enum Kind2Error {
    Io(std::io::Error),
    ToolFailure(String),
    MissingArtifact(PathBuf),
}

impl Display for Kind2Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "{err}"),
            Self::ToolFailure(err) => write!(f, "{err}"),
            Self::MissingArtifact(path) => {
                write!(
                    f,
                    "expected Kind 2 artifact was not produced: {}",
                    path.display()
                )
            }
        }
    }
}

impl Error for Kind2Error {}

impl From<std::io::Error> for Kind2Error {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

pub fn emit_kind2_system_smt(
    lustre_text: &str,
    top_node: &str,
    options: &Kind2Options,
) -> Result<String, Kind2Error> {
    let tempdir = tempdir()?;
    let model_path = tempdir.path().join("model.lus");
    let output_dir = tempdir.path().join("out");
    fs::write(&model_path, lustre_text)?;
    fs::create_dir_all(&output_dir)?;

    let output = run_kind2(
        options,
        &model_path,
        &output_dir,
        top_node,
        &["--certif", "true", "--slice_nodes", "off"],
    )?;

    if !output.status.success() {
        return Err(Kind2Error::ToolFailure(format!(
            "Kind 2 invocation failed with status {}.\nstdout:\n{}\nstderr:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    let artifact_path = output_dir.join("certif").join("kind2_sys.smt2");
    if !artifact_path.exists() {
        return Err(Kind2Error::MissingArtifact(artifact_path));
    }
    Ok(fs::read_to_string(artifact_path)?)
}

pub fn check_kind2_model(
    lustre_text: &str,
    top_node: &str,
    options: &Kind2Options,
) -> Result<Kind2CheckOutcome, Kind2Error> {
    let tempdir = tempdir()?;
    let model_path = tempdir.path().join("model.lus");
    let output_dir = tempdir.path().join("out");
    fs::write(&model_path, lustre_text)?;
    fs::create_dir_all(&output_dir)?;

    let output = run_kind2(options, &model_path, &output_dir, top_node, &[])?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stdout_plain = strip_ansi_escapes(&stdout);
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

    if stdout_plain.contains(": invalid") {
        return Ok(Kind2CheckOutcome::Invalid(stdout));
    }
    if stdout_plain.contains(": unknown") {
        return Ok(Kind2CheckOutcome::Unknown(stdout));
    }
    if stdout_plain.contains(": valid") {
        return Ok(Kind2CheckOutcome::Valid(stdout));
    }
    if !output.status.success() {
        return Err(Kind2Error::ToolFailure(format!(
            "Kind 2 invocation failed with status {}.\nstdout:\n{}\nstderr:\n{}",
            output.status, stdout, stderr
        )));
    }

    Ok(Kind2CheckOutcome::Unknown(format!(
        "Kind 2 completed without a recognized summary.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    )))
}

fn run_kind2(
    options: &Kind2Options,
    model_path: &PathBuf,
    output_dir: &PathBuf,
    top_node: &str,
    extra_args: &[&str],
) -> Result<std::process::Output, Kind2Error> {
    Ok(match &options.runner {
        Kind2Runner::Binary(bin) => {
            let mut command = Command::new(bin);
            if let Some(z3_bin) = std::env::var_os("XLSYNTH_Z3_BIN") {
                command.arg("--z3_bin").arg(z3_bin);
            }
            command.args(kind2_cli_args(
                options,
                output_dir.as_os_str(),
                top_node,
                model_path.as_os_str(),
                extra_args,
            ));
            run_command_capture_output(&mut command, options.live_output)?
        }
        Kind2Runner::Docker { docker_bin, image } => {
            let container_model = PathBuf::from("/tmp/model.lus");
            let container_output = PathBuf::from("/tmp/out");
            let copy_certificate = extra_args.contains(&"--certif");
            let mut create = Command::new(docker_bin);
            create.arg("create");
            if !copy_certificate {
                create.arg("--rm");
            }
            let create_output = create
                .arg("--entrypoint")
                .arg("/kind2")
                .arg(image)
                .args(kind2_cli_args(
                    options,
                    container_output.as_os_str(),
                    top_node,
                    container_model.as_os_str(),
                    extra_args,
                ))
                .output()?;
            if !create_output.status.success() {
                return Err(Kind2Error::ToolFailure(format!(
                    "failed to create Kind 2 container with status {}.\nstdout:\n{}\nstderr:\n{}",
                    create_output.status,
                    String::from_utf8_lossy(&create_output.stdout),
                    String::from_utf8_lossy(&create_output.stderr)
                )));
            }

            let container_id = String::from_utf8_lossy(&create_output.stdout)
                .trim()
                .to_string();
            if container_id.is_empty() {
                return Err(Kind2Error::ToolFailure(
                    "docker create did not return a container id".to_string(),
                ));
            }

            let result = (|| -> Result<Output, Kind2Error> {
                let copy_in = Command::new(docker_bin)
                    .arg("cp")
                    .arg(model_path)
                    .arg(format!("{}:{}", container_id, container_model.display()))
                    .output()?;
                if !copy_in.status.success() {
                    return Err(Kind2Error::ToolFailure(format!(
                        "failed to copy model into Kind 2 container with status {}.\nstdout:\n{}\nstderr:\n{}",
                        copy_in.status,
                        String::from_utf8_lossy(&copy_in.stdout),
                        String::from_utf8_lossy(&copy_in.stderr)
                    )));
                }

                let mut command = Command::new(docker_bin);
                command.arg("start").arg("-a").arg(&container_id);
                let output = run_command_capture_output(&mut command, options.live_output)?;

                if output.status.success() && copy_certificate {
                    let copy_out = Command::new(docker_bin)
                        .arg("cp")
                        .arg(format!("{}:{}/.", container_id, container_output.display()))
                        .arg(output_dir)
                        .output()?;
                    if !copy_out.status.success() {
                        return Err(Kind2Error::ToolFailure(format!(
                            "failed to copy Kind 2 artifacts from container with status {}.\nstdout:\n{}\nstderr:\n{}",
                            copy_out.status,
                            String::from_utf8_lossy(&copy_out.stdout),
                            String::from_utf8_lossy(&copy_out.stderr)
                        )));
                    }
                }
                Ok(output)
            })();
            if copy_certificate || !result.as_ref().is_ok_and(|output| output.status.success()) {
                let _ = Command::new(docker_bin)
                    .args(["rm", "-f", &container_id])
                    .output();
            }
            result?
        }
    })
}

fn run_command_capture_output(
    command: &mut Command,
    live_output: bool,
) -> Result<Output, std::io::Error> {
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let stdout = child
        .stdout
        .take()
        .expect("stdout was piped for child process");
    let stderr = child
        .stderr
        .take()
        .expect("stderr was piped for child process");
    let print_lock = Arc::new(Mutex::new(()));
    let stdout_handle = spawn_output_reader(stdout, live_output, false, Arc::clone(&print_lock));
    let stderr_handle = spawn_output_reader(stderr, live_output, true, print_lock);
    let status = child.wait()?;
    let stdout = join_output_reader(stdout_handle)?;
    let stderr = join_output_reader(stderr_handle)?;
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

fn spawn_output_reader<R: Read + Send + 'static>(
    mut reader: R,
    live_output: bool,
    is_stderr: bool,
    print_lock: Arc<Mutex<()>>,
) -> thread::JoinHandle<Result<Vec<u8>, std::io::Error>> {
    thread::spawn(move || {
        let mut collected = Vec::new();
        let mut buf = [0_u8; 4096];
        loop {
            let count = reader.read(&mut buf)?;
            if count == 0 {
                break;
            }
            let chunk = &buf[..count];
            collected.extend_from_slice(chunk);
            if live_output {
                let _guard = print_lock.lock().expect("print lock poisoned");
                if is_stderr {
                    let mut handle = std::io::stderr();
                    handle.write_all(chunk)?;
                    handle.flush()?;
                } else {
                    let mut handle = std::io::stdout();
                    handle.write_all(chunk)?;
                    handle.flush()?;
                }
            }
        }
        Ok(collected)
    })
}

fn join_output_reader(
    handle: thread::JoinHandle<Result<Vec<u8>, std::io::Error>>,
) -> Result<Vec<u8>, std::io::Error> {
    handle.join().map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::Other,
            "kind2 output reader thread panicked",
        )
    })?
}

fn kind2_cli_args(
    options: &Kind2Options,
    output_dir: &std::ffi::OsStr,
    top_node: &str,
    model_path: &std::ffi::OsStr,
    extra_args: &[&str],
) -> Vec<std::ffi::OsString> {
    let mut args = vec![
        std::ffi::OsString::from("--output_dir"),
        output_dir.to_os_string(),
    ];
    if let Some(flag) = verbosity_flag(options.verbosity) {
        args.push(std::ffi::OsString::from(flag));
    }
    if options.print_cex {
        args.push(std::ffi::OsString::from("--print_cex"));
        args.push(std::ffi::OsString::from("true"));
    }
    args.extend(options.extra_args.iter().map(std::ffi::OsString::from));
    args.extend(extra_args.iter().map(std::ffi::OsString::from));
    args.push(std::ffi::OsString::from("--lus_main"));
    args.push(std::ffi::OsString::from(top_node));
    args.push(model_path.to_os_string());
    args
}

pub fn extract_kind2_progress_lines(text: &str) -> Vec<String> {
    strip_ansi_escapes(text)
        .lines()
        .map(str::trim)
        .filter(|line| {
            let has_k = line.contains("k =") || line.contains("k=");
            let looks_like_progress = line.contains("IND")
                || line.contains("IC3")
                || line.contains("Message received")
                || line.contains("valid (k=")
                || line.contains("invalid after");
            has_k && looks_like_progress
        })
        .map(ToOwned::to_owned)
        .collect()
}

pub fn extract_kind2_counterexample(text: &str) -> Option<String> {
    let plain = strip_ansi_escapes(text);
    let lines = plain.lines().collect::<Vec<_>>();
    let start = lines.iter().position(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with("Counterexample")
            || trimmed.starts_with("Counterexample:")
            || trimmed.starts_with("Counterexample for")
    })?;

    let mut block = Vec::new();
    for line in &lines[start..] {
        if !block.is_empty() {
            let trimmed = line.trim();
            let looks_like_new_section = !trimmed.is_empty()
                && !line.starts_with(' ')
                && !line.starts_with('\t')
                && (trimmed.starts_with("Property")
                    || trimmed.starts_with("InvProp")
                    || trimmed.starts_with("Analysis")
                    || trimmed.starts_with("Runtime")
                    || trimmed.starts_with("Done")
                    || trimmed.starts_with("==")
                    || trimmed.starts_with("<"));
            if looks_like_new_section {
                break;
            }
        }
        block.push(*line);
    }
    let block = block.join("\n").trim().to_string();
    if block.is_empty() { None } else { Some(block) }
}

fn verbosity_flag(verbosity: u8) -> Option<&'static str> {
    match verbosity {
        0 => None,
        1 => Some("-v"),
        2 => Some("-vv"),
        _ => Some("-vvv"),
    }
}

pub fn extract_kind2_runtime_line(text: &str) -> Option<String> {
    strip_ansi_escapes(text)
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("Runtime:"))
        .map(ToOwned::to_owned)
}

fn strip_ansi_escapes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\u{1b}' {
            out.push(ch);
            continue;
        }
        if chars.peek() == Some(&'[') {
            chars.next();
            for next in chars.by_ref() {
                if ('@'..='~').contains(&next) {
                    break;
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::{
        Kind2Options, extract_kind2_counterexample, extract_kind2_progress_lines,
        extract_kind2_runtime_line, kind2_cli_args, run_command_capture_output,
    };

    #[test]
    fn extracts_verbose_progress_lines() {
        let text = "IND at k = 1     1 unknowns     0 unfalsifiables.\n\
IC3QE main loop at k=2\n\
Message received from property directed reachability (QE): Property InvProp[l4c3] true for 1 steps\n\
InvProp[l4c3]: valid (k=2)\n";
        let lines = extract_kind2_progress_lines(text);
        assert_eq!(
            lines,
            vec![
                "IND at k = 1     1 unknowns     0 unfalsifiables.".to_string(),
                "IC3QE main loop at k=2".to_string(),
                "InvProp[l4c3]: valid (k=2)".to_string(),
            ]
        );
    }

    #[test]
    fn extracts_counterexample_block() {
        let text = "InvProp[l4c3]: invalid after 0 steps\n\
Counterexample:\n\
  i = 0\n\
  o_a = 0\n\
  o_b = 1\n\
\n\
Runtime: 0.01s\n";
        assert_eq!(
            extract_kind2_counterexample(text).as_deref(),
            Some("Counterexample:\ni = 0\no_a = 0\no_b = 1")
        );
    }

    #[test]
    fn builds_kind2_cli_args() {
        let options = Kind2Options {
            verbosity: 2,
            print_cex: true,
            extra_args: vec!["--enable".to_string(), "IND".to_string()],
            ..Kind2Options::default()
        };
        let args = kind2_cli_args(
            &options,
            std::ffi::OsStr::new("/tmp/out"),
            "main",
            std::ffi::OsStr::new("/tmp/model.lus"),
            &["--certif", "true"],
        );
        let rendered = args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            rendered,
            vec![
                "--output_dir",
                "/tmp/out",
                "-vv",
                "--print_cex",
                "true",
                "--enable",
                "IND",
                "--certif",
                "true",
                "--lus_main",
                "main",
                "/tmp/model.lus",
            ]
        );
    }

    #[test]
    fn extracts_runtime_line() {
        let text = "InvProp[l4c3]: valid (k=2)\nRuntime: 0.12s\n";
        assert_eq!(
            extract_kind2_runtime_line(text).as_deref(),
            Some("Runtime: 0.12s")
        );
    }

    #[test]
    fn captures_stdout_and_stderr_with_live_output_enabled() {
        let mut command = Command::new("bash");
        command.args([
            "-lc",
            "printf 'out1\\nout2\\n'; printf 'err1\\nerr2\\n' >&2",
        ]);
        let output = run_command_capture_output(&mut command, true).unwrap();
        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout), "out1\nout2\n");
        assert_eq!(String::from_utf8_lossy(&output.stderr), "err1\nerr2\n");
    }
}
