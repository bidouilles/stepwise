//! Runs the student's program through the embedded Python tracer and parses
//! the recorded execution trace.

use serde::Deserialize;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const TRACER: &str = include_str!("tracer.py");
const MARKER: &str = "\u{1e}STEPWISE-TRACE\u{1e}";
pub const MAX_STEPS: usize = 1000;
const TIMEOUT: Duration = Duration::from_secs(10);

/// A value as shown in a variable slot: either written inline or a pointer to a heap object.
#[derive(Deserialize, Clone, PartialEq, Debug)]
#[serde(untagged)]
pub enum Val {
    Ref { r: u64 },
    Prim { p: String, k: String },
}

#[derive(Deserialize, Clone, PartialEq, Debug, Default)]
pub struct Obj {
    /// list, tuple, set, dict, instance, class, function, other
    pub kind: String,
    #[serde(rename = "type")]
    pub ty: String,
    #[serde(default)]
    pub items: Vec<Val>,
    #[serde(default)]
    pub pairs: Vec<(Val, Val)>,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub more: usize,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Frame {
    pub id: u64,
    pub name: String,
    pub locals: Vec<(String, Val)>,
    #[serde(default)]
    pub ret: Option<Val>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Step {
    /// line, call, return, exception, uncaught, end
    pub event: String,
    pub line: usize,
    pub func: String,
    /// How many bytes of the program's output had been printed at this step.
    pub out: usize,
    pub globals: Vec<(String, Val)>,
    pub frames: Vec<Frame>,
    pub heap: HashMap<u64, Obj>,
    #[serde(default)]
    pub msg: Option<String>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct TraceError {
    pub msg: String,
    pub line: usize,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Trace {
    pub steps: Vec<Step>,
    pub stdout: String,
    #[serde(default)]
    pub python: String,
    #[serde(default)]
    pub error: Option<TraceError>,
    /// Set when the run was cut short: "steps" or "time".
    #[serde(default)]
    pub limit: Option<String>,
}

impl Trace {
    pub fn output_at(&self, step: usize) -> &str {
        let n = self.steps.get(step).map_or(0, |s| s.out);
        self.stdout.get(..n).unwrap_or(&self.stdout)
    }
}

/// Set on the child process that acts as the Python interpreter.
pub const INTERPRETER_ENV: &str = "STEPWISE_INTERPRETER";

/// Starts the tracer. By default this is the app itself, running its built-in
/// Python interpreter (RustPython). `STEPWISE_PYTHON` selects another Python.
fn spawn_python(cwd: Option<&Path>) -> Result<Child, String> {
    let external = std::env::var_os("STEPWISE_PYTHON").filter(|p| !p.is_empty());
    let mut cmd = match &external {
        Some(python) => Command::new(python),
        None => {
            let exe = std::env::current_exe().map_err(|e| format!("Could not find the app's own path: {e}"))?;
            let mut cmd = Command::new(exe);
            cmd.env(INTERPRETER_ENV, "1");
            cmd
        }
    };
    cmd.arg("-c")
        .arg(TRACER)
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    cmd.spawn().map_err(|e| match external {
        Some(python) => format!("Could not start {}: {e}", python.to_string_lossy()),
        None => format!("Could not start the Python interpreter: {e}"),
    })
}

fn read_all(mut r: impl Read + Send + 'static) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = r.read_to_end(&mut buf);
        buf
    })
}

/// Blocking: call from a worker thread.
pub fn run_trace(code: &str, stdin: &str, cwd: Option<&Path>) -> Result<Trace, String> {
    let payload =
        serde_json::json!({ "code": code, "stdin": stdin, "max_steps": MAX_STEPS }).to_string();
    let mut child = spawn_python(cwd)?;
    if let Some(mut si) = child.stdin.take() {
        let _ = si.write_all(payload.as_bytes());
    }
    let out = read_all(child.stdout.take().ok_or("no stdout")?);
    let err = read_all(child.stderr.take().ok_or("no stderr")?);

    let deadline = Instant::now() + TIMEOUT;
    let mut timed_out = false;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() > deadline => {
                let _ = child.kill();
                let _ = child.wait();
                timed_out = true;
                break;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(e) => return Err(format!("Could not run Python: {e}")),
        }
    }
    if timed_out {
        return Err(format!(
            "The program ran for more than {} seconds and was stopped. \
             Is it waiting for something, or doing a very long computation?",
            TIMEOUT.as_secs()
        ));
    }
    let out = String::from_utf8_lossy(&out.join().unwrap_or_default()).into_owned();
    let err = String::from_utf8_lossy(&err.join().unwrap_or_default()).into_owned();
    let Some(pos) = out.rfind(MARKER) else {
        let detail = err.lines().last().unwrap_or("no output");
        return Err(format!("The tracer did not produce a result ({detail})."));
    };
    serde_json::from_str(&out[pos + MARKER.len()..])
        .map_err(|e| format!("Could not read the execution trace: {e}"))
}
