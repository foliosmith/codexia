use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
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
