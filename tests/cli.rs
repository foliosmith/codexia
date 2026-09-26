use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
    sync::{mpsc, Arc},
    thread,
    time::Duration,
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use serde_json::Value;

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[test]
fn parse_writes_book_ir_to_file_and_stdout() {
    let workspace = TempWorkspace::new("parse");
    let epub = workspace.path.join("fixture.epub");
    let output_path = workspace.path.join("book_ir.json");
    fs::write(&epub, minimal_epub()).expect("write EPUB fixture");

    let file_output = codexia(&[
        "parse",
        path_text(&epub),
        "--output",
        path_text(&output_path),
    ]);
    assert_success(&file_output);
    assert!(stderr(&file_output).contains("BookIR written"));

    let book_ir: Value = serde_json::from_slice(&fs::read(&output_path).expect("read BookIR"))
        .expect("parse BookIR JSON");
    assert_eq!(book_ir["title"], "CLI Fixture");
    assert_eq!(book_ir["chapters"].as_array().map(Vec::len), Some(1));
    assert!(book_ir["blocks"]
        .as_array()
        .is_some_and(|blocks| !blocks.is_empty()));

    let stdout_output = codexia(&["parse", path_text(&epub)]);
    assert_success(&stdout_output);
    let stdout_book: Value =
        serde_json::from_slice(&stdout_output.stdout).expect("parse stdout BookIR");
    assert_eq!(stdout_book, book_ir);
}

#[test]
fn compile_builds_a_package_that_validate_checks_semantically() {
    let workspace = TempWorkspace::new("compile");
    let epub = workspace.path.join("fixture.epub");
    let package = workspace.path.join("package");
    fs::write(&epub, minimal_epub()).expect("write EPUB fixture");

    let compile_output = codexia(&[
        "compile",
        path_text(&epub),
        "--profile",
        "standard",
        "--out",
        path_text(&package),
    ]);
    assert_success(&compile_output);
    for name in [
        "manifest.json",
        "structure.json",
        "book_ir.json",
        "blocks.jsonl",
    ] {
        assert!(package.join(name).is_file(), "missing {name}");
    }

    let validate_output = codexia(&["validate", path_text(&package)]);
    assert_success(&validate_output);
    assert!(stderr(&validate_output).contains("is valid"));

    let manifest_path = package.join("manifest.json");
    let mut manifest: Value =
        serde_json::from_slice(&fs::read(&manifest_path).expect("read manifest"))
            .expect("parse manifest");
    manifest["block_count"] = Value::from(999_u64);
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).expect("serialize corrupt manifest"),
    )
    .expect("write corrupt manifest");

    let corrupt_output = codexia(&["validate", path_text(&package)]);
    assert!(!corrupt_output.status.success());
    assert!(stderr(&corrupt_output).contains("count mismatch"));
}

#[test]
#[cfg(unix)]
fn compile_runs_the_full_book_analysis_pipeline_and_reuses_its_cache() {
    let workspace = TempWorkspace::new("chapter-analysis");
    let epub = workspace.path.join("fixture.epub");
    let package = workspace.path.join("package");
    let request_capture = workspace.path.join("request.json");
    let analyzer = workspace.path.join("analyzer.sh");
    fs::write(&epub, minimal_epub()).expect("write EPUB fixture");
    let script = r#"#!/bin/sh
request=$(cat)
case "$request" in
  *'"task":"explain_passage"'*)
    printf '%s' '{"cards":[{"card_type":"explanation","card_id":"agent-explanation","title":"Agent explanation","content":{"explanation":"Grounded by the supplied runtime context."},"source_refs":[],"confidence_basis_points":7000,"grounding":"inferred","spoiler_status":"within_boundary","follow_up_actions":[]}]}'
    ;;
  *'"task":"book_synthesis"'*)
    printf '%s' '{"book_map":{"central_question":"What makes a deterministic fixture?","thesis":"Stable inputs produce stable outputs.","chapter_roles":[{"chapter_id":"chapter_001","role":"Introduces the fixture.","depends_on_chapter_ids":[]}],"reading_paths":[{"path_id":"deep","kind":"deep","title":"Deep","description":"Read all details.","chapter_ids":["chapter_001"]},{"path_id":"fast","kind":"fast","title":"Fast","description":"Read the key chapter.","chapter_ids":["chapter_001"]},{"path_id":"selective","kind":"selective","title":"Selective","description":"Inspect the fixture.","chapter_ids":["chapter_001"]}],"difficulty_map":[{"chapter_id":"chapter_001","level":"introductory","reason":"Short fixture."}],"key_chapter_ids":["chapter_001"]},"concepts":[],"claims":[],"entities":[],"checkpoints":[{"checkpoint_id":"checkpoint_001","chapter_id":"chapter_001","summary":"The fixture is deterministic.","must_understand":["Stable input","Stable parse","Stable output"],"recall_questions":[{"question_id":"recall_001","prompt":"What is stable?","expected_points":["Input and output"]}],"reflection_questions":[{"question_id":"reflection_001","prompt":"Why does stability matter?","expected_points":["Repeatability"]}],"flashcards":[{"flashcard_id":"flashcard_001","front":"Determinism","back":"Stable input produces stable output.","concept_ids":[]}],"source_refs":[],"grounding":"inferred"}],"book_reflection_questions":[{"question_id":"book_reflection_001","prompt":"How would you test determinism?","expected_points":["Repeat the compile"]}]}'
    ;;
  *)
    printf '%s' "$request" > '__CAPTURE__'
    printf '%s' '{"summary":{"one_sentence":"One sentence.","short":"Short summary.","deep":"Deep summary.","role_in_book":"Introduces the fixture."},"key_ideas":["Determinism"],"concepts":[],"claims":[],"argument_flow":[],"difficult_passages":[],"entities":[]}'
    ;;
esac
"#
    .replace("__CAPTURE__", &request_capture.display().to_string());
    fs::write(&analyzer, script).expect("write analyzer fixture");
    let mut permissions = fs::metadata(&analyzer)
        .expect("analyzer metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&analyzer, permissions).expect("make analyzer executable");

    let output = codexia(&[
        "compile",
        path_text(&epub),
        "--out",
        path_text(&package),
        "--analyzer-command",
        path_text(&analyzer),
        "--analysis-jobs",
        "1",
    ]);
    assert_success(&output);
    assert!(stderr(&output).contains("1 analyses"));

    let request: Value =
        serde_json::from_slice(&fs::read(&request_capture).expect("read analyzer request"))
            .expect("parse analyzer request");
    assert_eq!(request["task"], "chapter_analysis");
    assert_eq!(request["prompt_tasks"].as_array().map(Vec::len), Some(5));
    assert_eq!(request["context"]["chapter"]["chapter_id"], "chapter_001");
    assert!(request["context"]["chapter"]["blocks"]
        .as_array()
        .is_some_and(|blocks| !blocks.is_empty()));

    let analysis_path = package.join("chapters/chapter_001.analysis.json");
    let analysis: Value =
        serde_json::from_slice(&fs::read(&analysis_path).expect("read chapter analysis"))
            .expect("parse chapter analysis");
    assert_eq!(analysis["schema_version"], "0.1");
    assert_eq!(analysis["analysis_profile"], "standard");
    assert_eq!(analysis["chapter_id"], "chapter_001");
    assert_eq!(analysis["summary"]["one_sentence"], "One sentence.");

    for name in [
        "book_map.json",
        "concepts.json",
        "claims.json",
        "entities.json",
        "checkpoints.json",
        "recall_cards.json",
        "eval_report.json",
        "compile_status.json",
    ] {
        assert!(package.join(name).is_file(), "missing {name}");
    }
    let validate_output = codexia(&["validate", path_text(&package)]);
    assert_success(&validate_output);
    assert!(stderr(&validate_output).contains("analysis artifacts: ok"));

    let cached_output = codexia(&[
        "compile",
        path_text(&epub),
        "--out",
        path_text(&package),
        "--analyzer-command",
        path_text(&analyzer),
        "--analysis-jobs",
        "1",
    ]);
    assert_success(&cached_output);
    assert!(stderr(&cached_output).contains("Reused cached package"));

    let runtime = codexia::web_runtime::WebRuntime::load(
        &package,
        workspace.path.join("runtime-state"),
        Some(analyzer.clone()),
    )
    .expect("load Web Runtime");
    let page = runtime.dispatch("GET", "/", &[]);
    assert_eq!(page.status(), 200);
    assert!(page.content_type().starts_with("text/html"));
    assert!(String::from_utf8_lossy(page.body()).contains("reader-shell"));
    let studio_page = runtime.dispatch("GET", "/studio", &[]);
    assert_eq!(studio_page.status(), 200);
    assert!(String::from_utf8_lossy(studio_page.body()).contains("Book Agent Studio"));

    let bootstrap = response_json(runtime.dispatch("GET", "/v1/bootstrap", &[]));
    let book_id = bootstrap["book"]["book_id"].as_str().expect("book id");
    let session = response_json(runtime.dispatch(
        "POST",
        "/v1/reader-sessions",
        r#"{"book_id":"__BOOK__","current_location":{"chapter_id":"chapter_001","block_id":null,"char_offset":null,"epub_cfi":null},"spoiler_mode":"read_range"}"#
            .replace("__BOOK__", book_id)
            .as_bytes(),
    ));
    let session_id = session["session_id"].as_str().expect("session id");
    let chapter = response_json(runtime.dispatch(
        "GET",
        &format!("/v1/books/{book_id}/chapters/chapter_001/content"),
        &[],
    ));
    let block = chapter["blocks"]
        .as_array()
        .and_then(|blocks| blocks.last())
        .expect("chapter block");
    let block_id = block["block_id"].as_str().expect("block id");
    let block_text = block["text"].as_str().expect("block text");
    let fingerprint = block["text_fingerprint"].as_str().expect("fingerprint");
    let updated_session = response_json(runtime.dispatch(
        "PATCH",
        &format!("/v1/reader-sessions/{session_id}"),
        &serde_json::to_vec(&serde_json::json!({
            "current_location": {"chapter_id":"chapter_001","block_id":block_id,"char_offset":0,"epub_cfi":null},
            "read_until": {"chapter_id":"chapter_001","block_id":block_id,"char_offset":block_text.chars().count(),"epub_cfi":null},
            "progress_basis_points": 10_000,
        }))
        .expect("serialize session patch"),
    ));
    let explain_body = serde_json::json!({
        "selected_text": block_text,
        "source_ref": {
            "block_id": block_id,
            "start_char": 0,
            "end_char": block_text.chars().count(),
            "text_fingerprint": fingerprint,
        },
        "reader_state": {
            "session_id": session_id,
            "current_location": updated_session["current_location"].clone(),
            "read_until": updated_session["read_until"].clone(),
            "completed_chapter_ids": [],
            "progress_basis_points": updated_session["progress_basis_points"].clone(),
        },
        "spoiler_mode": "read_range",
        "intent": "explain",
    });
    let first_block_id = chapter["blocks"][0]["block_id"]
        .as_str()
        .expect("first block id");
    let mut blocked_body = explain_body.clone();
    blocked_body["reader_state"]["session_id"] = Value::Null;
    blocked_body["reader_state"]["read_until"]["block_id"] = Value::from(first_block_id);
    blocked_body["reader_state"]["read_until"]["char_offset"] = Value::from(0);
    let blocked = runtime.dispatch(
        "POST",
        &format!("/v1/books/{book_id}/explain"),
        &serde_json::to_vec(&blocked_body).expect("serialize blocked explain"),
    );
    assert_eq!(blocked.status(), 403);
    assert!(String::from_utf8_lossy(blocked.body()).contains("spoiler_boundary"));

    let explain = response_json(runtime.dispatch(
        "POST",
        &format!("/v1/books/{book_id}/explain"),
        &serde_json::to_vec(&explain_body).expect("serialize explain"),
    ));
    assert_eq!(explain["cards"][0]["card_type"], "explanation");
    assert_eq!(explain["cards"][0]["title"], "Agent explanation");

    let checkpoint_body = serde_json::json!({
        "reader_state": explain_body["reader_state"].clone(),
        "spoiler_mode": "read_range",
    });
    let checkpoint = response_json(runtime.dispatch(
        "POST",
        &format!("/v1/books/{book_id}/chapters/chapter_001/checkpoint"),
        &serde_json::to_vec(&checkpoint_body).expect("serialize checkpoint"),
    ));
    assert_eq!(checkpoint["cards"][0]["card_type"], "checkpoint");

    let note = response_json(
        runtime.dispatch(
            "POST",
            &format!("/v1/reader-sessions/{session_id}/notes"),
            &serde_json::to_vec(&serde_json::json!({
                "chapter_id": "chapter_001",
                "block_id": block_id,
                "text": "A saved note",
            }))
            .expect("serialize note"),
        ),
    );
    assert_eq!(note["text"], "A saved note");

    let export = response_json(
        runtime.dispatch(
            "POST",
            &format!("/v1/books/{book_id}/exports"),
            &serde_json::to_vec(&serde_json::json!({
                "format": "markdown",
                "scope": "whole_book",
                "chapter_ids": [],
                "session_id": session_id,
            }))
            .expect("serialize export"),
        ),
    );
    assert!(Path::new(export["local_path"].as_str().expect("export path")).is_file());

    let studio = response_json(runtime.dispatch("GET", "/v1/studio/snapshot", &[]));
    let mut corrected = studio["chapters"]["chapter_001"].clone();
    corrected["summary"]["one_sentence"] = Value::from("Corrected Studio summary.");
    let manual_version = response_json(runtime.dispatch(
        "PUT",
        "/v1/studio/chapters/chapter_001",
        &serde_json::to_vec(&corrected).expect("serialize correction"),
    ));
    let manual_version_id = manual_version["version_id"]
        .as_str()
        .expect("manual version id");
    let versions =
        response_json(runtime.dispatch("GET", "/v1/studio/chapters/chapter_001/versions", &[]));
    assert_eq!(versions.as_array().map(Vec::len), Some(2));
    let diff = response_json(
        runtime.dispatch(
            "POST",
            "/v1/studio/compare",
            &serde_json::to_vec(&serde_json::json!({
                "kind": "chapter",
                "id": "chapter_001",
                "left_version": "base",
                "right_version": manual_version_id,
            }))
            .expect("serialize version comparison"),
        ),
    );
    assert!(diff.as_array().is_some_and(|items| items
        .iter()
        .any(|item| item["path"] == "$.summary.one_sentence")));
    let reanalyzed = response_json(runtime.dispatch(
        "POST",
        "/v1/studio/chapters/chapter_001/reanalyze",
        br#"{"analyzer_label":"fixture-v2"}"#,
    ));
    assert_eq!(reanalyzed["analyzer"], "fixture-v2");
    let golden = response_json(runtime.dispatch(
        "POST",
        "/v1/studio/golden-books",
        br#"{"label":"CLI Golden","annotations":{"summary":"approved"}}"#,
    ));
    assert_eq!(golden["label"], "CLI Golden");
    let eval = response_json(runtime.dispatch("POST", "/v1/studio/evals", b"{}"));
    for metric in [
        "parse_quality",
        "source_ref_validity",
        "chapter_coverage",
        "claim_grounding",
        "concept_grounding",
    ] {
        assert!(
            eval["metrics"][metric]["state"].is_string(),
            "missing {metric}"
        );
    }

    let api_key = "codexia-test-key-123456";
    let webhook_listener = TcpListener::bind("127.0.0.1:0").expect("bind webhook fixture");
    let webhook_url = format!(
        "http://{}/events",
        webhook_listener.local_addr().expect("webhook address")
    );
    let (webhook_sender, webhook_receiver) = mpsc::channel();
    thread::spawn(move || {
        let (mut stream, _) = webhook_listener.accept().expect("accept webhook");
        let mut bytes = [0_u8; 8192];
        let read = stream.read(&mut bytes).expect("read webhook");
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
            .expect("respond to webhook");
        webhook_sender
            .send(String::from_utf8_lossy(&bytes[..read]).into_owned())
            .expect("capture webhook");
    });
    let public_api = Arc::new(
        codexia::public_api::PublicApi::load(
            workspace.path.join("api-library"),
            codexia::public_api::PublicApiConfig {
                api_key: api_key.to_owned(),
                rate_limit_per_minute: 100,
                agent_request_cost_micros: 25,
                compile_cost_micros: 1_000,
                analyzer_command: Some(analyzer.clone()),
                webhook_url: Some(webhook_url),
                compiler_executable: PathBuf::from(env!("CARGO_BIN_EXE_codexia")),
            },
        )
        .expect("load Public API"),
    );
    let openapi = public_api.dispatch("GET", "/openapi.json", &BTreeMap::new(), &[]);
    assert_eq!(openapi.status(), 200);
    serde_json::from_slice::<Value>(openapi.body()).expect("parse OpenAPI document");
    let unauthorized = public_api.dispatch("GET", "/v1/usage", &BTreeMap::new(), &[]);
    assert_eq!(unauthorized.status(), 401);
    let api_headers = BTreeMap::from([
        ("X-API-Key".to_owned(), api_key.to_owned()),
        ("X-Codexia-Profile".to_owned(), "standard".to_owned()),
    ]);
    let accepted =
        response_json(public_api.dispatch("POST", "/v1/books", &api_headers, &minimal_epub()));
    let api_book_id = accepted["book_id"].as_str().expect("API book id");
    let mut ready = false;
    for _ in 0..100 {
        let status = response_json(public_api.dispatch(
            "GET",
            &format!("/v1/books/{api_book_id}/status"),
            &api_headers,
            &[],
        ));
        match status["state"].as_str() {
            Some("ready") => {
                ready = true;
                break;
            }
            Some("failed") => panic!("API compilation failed: {status}"),
            _ => thread::sleep(Duration::from_millis(25)),
        }
    }
    assert!(ready, "Public API compilation did not finish");
    let webhook = webhook_receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("processing webhook");
    assert!(webhook.contains("book.processing.completed"));
    thread::sleep(Duration::from_millis(50));
    let map = response_json(public_api.dispatch(
        "GET",
        &format!("/v1/books/{api_book_id}/map"),
        &api_headers,
        &[],
    ));
    assert_eq!(
        map["central_question"],
        "What makes a deterministic fixture?"
    );
    let usage = response_json(public_api.dispatch("GET", "/v1/usage", &api_headers, &[]));
    assert_eq!(usage["compile_count"], 1);
    assert!(usage["estimated_cost_micros"]
        .as_u64()
        .is_some_and(|value| value >= 1_000));

    let limited_api = Arc::new(
        codexia::public_api::PublicApi::load(
            workspace.path.join("limited-api"),
            codexia::public_api::PublicApiConfig {
                api_key: api_key.to_owned(),
                rate_limit_per_minute: 1,
                agent_request_cost_micros: 0,
                compile_cost_micros: 0,
                analyzer_command: None,
                webhook_url: None,
                compiler_executable: PathBuf::from(env!("CARGO_BIN_EXE_codexia")),
            },
        )
        .expect("load limited API"),
    );
    assert_eq!(
        limited_api
            .dispatch("GET", "/v1/usage", &api_headers, &[])
            .status(),
        200
    );
    assert_eq!(
        limited_api
            .dispatch("GET", "/v1/usage", &api_headers, &[])
            .status(),
        429
    );

    let book_map_path = package.join("book_map.json");
    let mut book_map: Value =
        serde_json::from_slice(&fs::read(&book_map_path).expect("read book map"))
            .expect("parse book map");
    book_map["source_hash"] = Value::from("wrong");
    fs::write(
        &book_map_path,
        serde_json::to_vec_pretty(&book_map).expect("serialize corrupt book map"),
    )
    .expect("write corrupt book map");
    let corrupt_output = codexia(&["validate", path_text(&package)]);
    assert!(!corrupt_output.status.success());
    assert!(stderr(&corrupt_output).contains("source_hash does not match"));
}

#[test]
fn cli_rejects_ambiguous_or_incomplete_arguments() {
    let workspace = TempWorkspace::new("arguments");
    let epub = workspace.path.join("fixture.epub");
    fs::write(&epub, minimal_epub()).expect("write EPUB fixture");

    let duplicate_profile = codexia(&[
        "compile",
        path_text(&epub),
        "--profile",
        "standard",
        "--profile",
        "deep",
        "--out",
        path_text(&workspace.path.join("package")),
    ]);
    assert!(!duplicate_profile.status.success());
    assert!(stderr(&duplicate_profile).contains("--profile may only be specified once"));

    let missing_output = codexia(&["parse", path_text(&epub), "--output", "--invalid"]);
    assert!(!missing_output.status.success());
    assert!(stderr(&missing_output).contains("--output requires a value"));

    let extra_validate = codexia(&["validate", "one", "two"]);
    assert!(!extra_validate.status.success());
    assert!(stderr(&extra_validate).contains("exactly one package directory"));

    let jobs_without_analyzer = codexia(&[
        "compile",
        path_text(&epub),
        "--out",
        path_text(&workspace.path.join("package")),
        "--analysis-jobs",
        "2",
    ]);
    assert!(!jobs_without_analyzer.status.success());
    assert!(stderr(&jobs_without_analyzer).contains("requires --analyzer-command"));

    let zero_chapter = codexia(&[
        "compile",
        path_text(&epub),
        "--out",
        path_text(&workspace.path.join("package")),
        "--analyze-through",
        "0",
    ]);
    assert!(!zero_chapter.status.success());
    assert!(stderr(&zero_chapter).contains("positive chapter number"));
}

fn codexia(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_codexia"))
        .args(args)
        .output()
        .expect("run codexia")
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "command failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        stderr(output)
    );
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn response_json(response: codexia::web_runtime::HttpResponse) -> Value {
    assert!(
        (200..300).contains(&response.status()),
        "runtime response failed: {}",
        String::from_utf8_lossy(response.body())
    );
    serde_json::from_slice(response.body()).expect("parse runtime response")
}

fn path_text(path: &Path) -> &str {
    path.to_str().expect("UTF-8 test path")
}

struct TempWorkspace {
    path: PathBuf,
}

impl TempWorkspace {
    fn new(label: &str) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "codexia-cli-{label}-{}-{timestamp}-{counter}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create test workspace");
        Self { path }
    }
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

struct ZipEntry<'a> {
    path: &'a str,
    bytes: &'a [u8],
}

fn minimal_epub() -> Vec<u8> {
    let container = br#"<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="EPUB/package.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#;
    let package = br#"<?xml version="1.0" encoding="utf-8"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="bookid"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="bookid">urn:codexia:cli</dc:identifier><dc:title>CLI Fixture</dc:title><dc:language>en</dc:language></metadata><manifest><item id="chapter" href="chapter.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="chapter"/></spine></package>"#;
    let chapter = br#"<?xml version="1.0" encoding="utf-8"?><html xmlns="http://www.w3.org/1999/xhtml"><head><title>Chapter One</title></head><body><h1>Chapter One</h1><p>A deterministic CLI fixture paragraph.</p></body></html>"#;
    write_stored_zip(&[
        ZipEntry {
            path: "mimetype",
            bytes: b"application/epub+zip",
        },
        ZipEntry {
            path: "META-INF/container.xml",
            bytes: container,
        },
        ZipEntry {
            path: "EPUB/package.opf",
            bytes: package,
        },
        ZipEntry {
            path: "EPUB/chapter.xhtml",
            bytes: chapter,
        },
    ])
}

fn write_stored_zip(entries: &[ZipEntry<'_>]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();

    for entry in entries {
        let offset = u32::try_from(out.len()).expect("fixture offset");
        let name = entry.path.as_bytes();
        let size = u32::try_from(entry.bytes.len()).expect("fixture size");
        let crc = crc32(entry.bytes);

        write_u32(&mut out, 0x0403_4b50);
        write_u16(&mut out, 20);
        write_u16(&mut out, 0);
        write_u16(&mut out, 0);
        write_u16(&mut out, 0);
        write_u16(&mut out, 0);
        write_u32(&mut out, crc);
        write_u32(&mut out, size);
        write_u32(&mut out, size);
        write_u16(&mut out, u16::try_from(name.len()).expect("fixture name"));
        write_u16(&mut out, 0);
        out.extend_from_slice(name);
        out.extend_from_slice(entry.bytes);

        write_u32(&mut central, 0x0201_4b50);
        write_u16(&mut central, 20);
        write_u16(&mut central, 20);
        write_u16(&mut central, 0);
        write_u16(&mut central, 0);
        write_u16(&mut central, 0);
        write_u16(&mut central, 0);
        write_u32(&mut central, crc);
        write_u32(&mut central, size);
        write_u32(&mut central, size);
        write_u16(
            &mut central,
            u16::try_from(name.len()).expect("fixture name"),
        );
        write_u16(&mut central, 0);
        write_u16(&mut central, 0);
        write_u16(&mut central, 0);
        write_u16(&mut central, 0);
        write_u32(&mut central, 0);
        write_u32(&mut central, offset);
        central.extend_from_slice(name);
    }

    let central_offset = u32::try_from(out.len()).expect("central offset");
    let central_size = u32::try_from(central.len()).expect("central size");
    out.extend_from_slice(&central);
    write_u32(&mut out, 0x0605_4b50);
    write_u16(&mut out, 0);
    write_u16(&mut out, 0);
    write_u16(&mut out, u16::try_from(entries.len()).expect("entry count"));
    write_u16(&mut out, u16::try_from(entries.len()).expect("entry count"));
    write_u32(&mut out, central_size);
    write_u32(&mut out, central_offset);
    write_u16(&mut out, 0);
    out
}

fn write_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn write_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffff_u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = 0_u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}
