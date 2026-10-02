//! Bounded command execution shared by compiler and reader actions.
use serde_json::{json, Value};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static NEXT: AtomicUsize = AtomicUsize::new(0);
pub(crate) fn id() -> String {
    format!(
        "{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}
pub(crate) fn limit(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|v| *v > 0)
        .unwrap_or(default)
}
struct Permit;
impl Drop for Permit {
    fn drop(&mut self) {
        ACTIVE.fetch_sub(1, Ordering::SeqCst);
    }
}

pub(crate) fn terminate(child: &mut std::process::Child) {
    if matches!(child.try_wait(), Ok(Some(_))) {
        return;
    }
    #[cfg(unix)]
    fn descendants(pid: u32, ids: &mut Vec<u32>) {
        if let Ok(output) = Command::new("pgrep")
            .args(["-P", &pid.to_string()])
            .output()
        {
            for id in String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter_map(|v| v.parse().ok())
            {
                descendants(id, ids);
                ids.push(id);
            }
        }
    }
    #[cfg(unix)]
    {
        // Keep the inherited process group so shutdown of the owning server also reaches adapters.
        let _ = Command::new("kill")
            .args(["-STOP", &child.id().to_string()])
            .status();
        let mut ids = Vec::new();
        descendants(child.id(), &mut ids);
        for id in ids {
            let _ = Command::new("kill")
                .args(["-KILL", &id.to_string()])
                .output();
        }
    }
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .output();
    }
    let _ = child.kill();
    let _ = child.wait();
}

pub(crate) fn run(
    executable: &Path,
    request: &Value,
    cancel: &AtomicBool,
    request_id: &str,
    diagnostics: Option<&Path>,
) -> Result<Value, String> {
    let start = Instant::now();
    let bytes = serde_json::to_vec(request).map_err(|_| "invalid_request")?;
    let mut usage = Value::Null;
    let mut output_bytes = 0;
    let result = (|| {
        if bytes.len() > limit("CODEXIA_MAX_CONTEXT_BYTES", 512 * 1024) {
            return Err("context_too_large".to_owned());
        }
        ACTIVE
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |v| {
                (v < limit("CODEXIA_MAX_ANALYZERS", 4)).then_some(v + 1)
            })
            .map_err(|_| "busy")?;
        let _permit = Permit;
        let temp = std::env::temp_dir().join(format!("codexia-usage-{}", id()));
        fs::create_dir(&temp).map_err(|_| "diagnostics_unavailable")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&temp, fs::Permissions::from_mode(0o700))
                .map_err(|_| "diagnostics_unavailable")?;
        }
        let usage_path = temp.join("usage.json");
        let execution = (|| {
            let mut child = Command::new(executable)
                .env("CODEXIA_USAGE_FILE", &usage_path)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|_| "analyzer_unavailable")?;
            let mut stdin = child.stdin.take().ok_or("analyzer_unavailable")?;
            let stdout = child.stdout.take().ok_or("analyzer_unavailable")?;
            let stderr = child.stderr.take().ok_or("analyzer_unavailable")?;
            let input = bytes.clone();
            let writer = thread::spawn(move || stdin.write_all(&input));
            let cap = limit("CODEXIA_MAX_OUTPUT_BYTES", 4 * 1024 * 1024);
            let reader = thread::spawn(move || {
                let mut b = Vec::new();
                stdout.take(cap as u64 + 1).read_to_end(&mut b).map(|_| b)
            });
            let errors = thread::spawn(move || {
                let mut b = Vec::new();
                stderr.take(65537).read_to_end(&mut b).map(|_| b.len())
            });
            let reader_task = matches!(
                request.get("task").and_then(Value::as_str),
                Some("ask_book" | "explain_passage" | "reflect_on_answer")
            );
            let timeout = Duration::from_millis(limit(
                if reader_task {
                    "CODEXIA_READER_TIMEOUT_MS"
                } else {
                    "CODEXIA_ANALYZER_TIMEOUT_MS"
                },
                if reader_task { 120000 } else { 1200000 },
            ) as u64);
            let status = loop {
                let reason = if cancel.load(Ordering::SeqCst) {
                    Some("cancelled")
                } else if start.elapsed() > timeout {
                    Some("timeout")
                } else {
                    None
                };
                if let Some(reason) = reason {
                    terminate(&mut child);
                    break Err(reason);
                }
                match child.try_wait() {
                    Ok(Some(status)) => break Ok(status),
                    Ok(None) => thread::sleep(Duration::from_millis(10)),
                    Err(_) => {
                        terminate(&mut child);
                        break Err("analyzer_failed");
                    }
                }
            };
            // Adapters must close inherited pipes on exit; the deadline remains active while draining them.
            while !writer.is_finished() || !reader.is_finished() || !errors.is_finished() {
                if start.elapsed() > timeout {
                    terminate(&mut child);
                    return Err("timeout");
                }
                thread::sleep(Duration::from_millis(10));
            }
            let sent = writer.join().map_err(|_| "analyzer_failed")?;
            let output = reader
                .join()
                .map_err(|_| "analyzer_failed")?
                .map_err(|_| "analyzer_failed")?;
            let error_bytes = errors
                .join()
                .map_err(|_| "analyzer_failed")?
                .map_err(|_| "analyzer_failed")?;
            let status = status?;
            sent.map_err(|_| "analyzer_failed")?;
            output_bytes = output.len();
            if output.len() > cap || error_bytes > 65536 {
                return Err("output_too_large");
            }
            if !status.success() {
                return Err("analyzer_failed");
            }
            serde_json::from_slice(&output).map_err(|_| "invalid_output")
        })();
        if let Ok(file) = fs::File::open(&usage_path) {
            if let Ok(v) = serde_json::from_reader::<_, Value>(file.take(4096)) {
                usage = json!({"input_tokens":v.get("input_tokens").and_then(Value::as_u64),"output_tokens":v.get("output_tokens").and_then(Value::as_u64)});
            }
        }
        let _ = fs::remove_dir_all(temp);
        execution.map_err(str::to_owned)
    })();
    if let Some(dir) = diagnostics {
        let price = |name| {
            std::env::var(name)
                .ok()
                .and_then(|v| v.parse::<f64>().ok())
                .filter(|v| v.is_finite() && *v >= 0.0)
        };
        let cost = usage["input_tokens"]
            .as_u64()
            .zip(usage["output_tokens"].as_u64())
            .zip(
                price("CODEXIA_INPUT_USD_PER_MILLION").zip(price("CODEXIA_OUTPUT_USD_PER_MILLION")),
            )
            .map(|((i, o), (ip, op))| (i as f64 * ip + o as f64 * op) / 1_000_000.0);
        let event = json!({"request_id":request_id,"task":request.get("task"),"status":result.as_ref().err().map_or("completed",String::as_str),"duration_ms":start.elapsed().as_millis(),"input_bytes":bytes.len(),"output_bytes":output_bytes,"usage":usage,"estimated_usd":cost,"analysis_version":request.get("analysis_version"),"allowed_ranges":request.pointer("/spoiler_boundary/read_coverage"),"retrieved_block_ids":request.pointer("/context/nearby_blocks").and_then(Value::as_array).map(|blocks|blocks.iter().filter_map(|b|b.get("block_id")).collect::<Vec<_>>()),"validation":"transport_and_json"});
        fs::create_dir_all(dir).map_err(|_| "diagnostics_unavailable")?;
        crate::book_analysis::write_atomic_json(&dir.join(format!("{request_id}.json")), &event)
            .map_err(|_| "diagnostics_unavailable")?;
    }
    result
}
