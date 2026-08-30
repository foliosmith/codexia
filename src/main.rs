//! codexia CLI — Book Agent compiler
//!
//! Usage:
//!   codexia parse <input.epub> [--output <file>]           Parse EPUB → BookIR
//!   codexia compile <input.epub> --out <dir>                Compile Book Package
//!     [--analyzer-command <executable>] [--analysis-jobs <count>]
//!   codexia validate <dir>                                  Validate a compiled package
//!   codexia help                                            Show help

#![forbid(unsafe_code)]

use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

use codexia::book_analysis::{self, CompileStatus, EvalReport, GroundingBlock};
use codexia::chapter_analysis::{self, AnalysisOptions, ChapterAnalysis, CommandChapterAnalyzer};
use codexia::epub_parser;
use codexia::ir;
use codexia::ir::Profile;
use codexia::normalizer;
use serde_json::Value;

fn main() -> ExitCode {
    match run(env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(1)
        }
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    match args.as_slice() {
        [cmd, rest @ ..] if cmd == "parse" => cmd_parse(rest),
        [cmd, rest @ ..] if cmd == "compile" => cmd_compile(rest),
        [cmd, rest @ ..] if cmd == "validate" => cmd_validate(rest),
        [cmd] if matches!(cmd.as_str(), "-h" | "--help" | "help") => {
            print_help();
            Ok(())
        }
        [] => {
            print_help();
            Ok(())
        }
        [cmd, ..] => Err(format!("unknown codexia command: {cmd}")),
    }
}

fn cmd_parse(args: &[String]) -> Result<(), String> {
    let mut output = None;
    let mut path = None;

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--output" | "-o" => {
                index += 1;
                let value = option_value(args, index, "--output")?;
                if output.replace(value.to_owned()).is_some() {
                    return Err("--output may only be specified once".to_owned());
                }
            }
            value if value.starts_with('-') => {
                return Err(format!("unknown parse option: {value}"));
            }
            value => {
                if path.replace(value.to_owned()).is_some() {
                    return Err("parse accepts exactly one EPUB path".to_owned());
                }
            }
        }
        index += 1;
    }

    let path = path.ok_or_else(|| "parse requires an EPUB path".to_owned())?;
    let bytes = fs::read(&path).map_err(|e| format!("cannot read {path}: {e}"))?;

    let book_ir = epub_parser::parse_epub(&bytes)?;

    let json = ir::book_ir_to_json(&book_ir);
    if let Some(output) = output {
        fs::write(&output, &json).map_err(|e| format!("cannot write {output}: {e}"))?;
        eprintln!("BookIR written to {output}");
    } else {
        println!("{json}");
    }
    Ok(())
}

fn cmd_compile(args: &[String]) -> Result<(), String> {
    let mut out_dir = None;
    let mut profile = "standard".to_owned();
    let mut profile_set = false;
    let mut analyzer_command = None;
    let mut analysis_jobs = None;
    let mut analyze_through = None;
    let mut force = false;
    let mut path = None;

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--out" | "-o" => {
                index += 1;
                let value = option_value(args, index, "--out")?;
                if out_dir.replace(value.to_owned()).is_some() {
                    return Err("--out may only be specified once".to_owned());
                }
            }
            "--profile" | "-p" => {
                index += 1;
                let value = option_value(args, index, "--profile")?;
                if profile_set {
                    return Err("--profile may only be specified once".to_owned());
                }
                profile = value.to_owned();
                profile_set = true;
            }
            "--analyzer-command" => {
                index += 1;
                let value = option_value(args, index, "--analyzer-command")?;
                if analyzer_command.replace(value.to_owned()).is_some() {
                    return Err("--analyzer-command may only be specified once".to_owned());
                }
            }
            "--analysis-jobs" => {
                index += 1;
                let value = option_value(args, index, "--analysis-jobs")?;
                if analysis_jobs.is_some() {
                    return Err("--analysis-jobs may only be specified once".to_owned());
                }
                let jobs = value
                    .parse::<usize>()
                    .map_err(|_| "--analysis-jobs must be a positive integer".to_owned())?;
                if jobs == 0 {
                    return Err("--analysis-jobs must be a positive integer".to_owned());
                }
                analysis_jobs = Some(jobs);
            }
            "--analyze-through" => {
                index += 1;
                let value = option_value(args, index, "--analyze-through")?;
                if analyze_through.is_some() {
                    return Err("--analyze-through may only be specified once".to_owned());
                }
                let chapter = value.parse::<u32>().map_err(|_| {
                    "--analyze-through must be a positive chapter number".to_owned()
                })?;
                if chapter == 0 {
                    return Err("--analyze-through must be a positive chapter number".to_owned());
                }
                analyze_through = Some(chapter);
            }
            "--force" => force = true,
            value if value.starts_with('-') => {
                return Err(format!("unknown compile option: {value}"));
            }
            value => {
                if path.replace(value.to_owned()).is_some() {
                    return Err("compile accepts exactly one EPUB path".to_owned());
                }
            }
        }
        index += 1;
    }

    let path = path.ok_or_else(|| "compile requires an EPUB path".to_owned())?;
    let out_dir = out_dir.ok_or_else(|| "compile requires --out <dir>".to_owned())?;
    let profile = match profile.as_str() {
        "basic" => Profile::Basic,
        "standard" => Profile::Standard,
        "deep" => Profile::Deep,
        other => return Err(format!("unknown profile: {other}")),
    };
    if analyzer_command.is_none() && analysis_jobs.is_some() {
        return Err("--analysis-jobs requires --analyzer-command".to_owned());
    }

    let bytes = fs::read(&path).map_err(|e| format!("cannot read {path}: {e}"))?;
    let source_hash = hex_encode(pagelet::core::ContentHash::from_bytes(&bytes).as_bytes());
    let out = PathBuf::from(&out_dir);

    if analyzer_command.is_some()
        && !force
        && cached_package_matches(&out, &source_hash, profile, analyze_through)
    {
        eprintln!("Reused cached package for source_hash {source_hash} at {out_dir}/");
        return Ok(());
    }

    let book_ir = epub_parser::parse_epub(&bytes)?;
    let book_ir_json = ir::book_ir_to_json(&book_ir);

    let compiled = normalizer::compile(&book_ir, profile, &source_hash);
    let structure_json = normalizer::structure_json(&compiled.structure);
    let blocks_jsonl = normalizer::blocks_jsonl(&book_ir.blocks);
    let manifest_json = normalizer::manifest_json(&compiled.manifest);

    fs::create_dir_all(&out).map_err(|e| format!("cannot create directory {out_dir}: {e}"))?;
    clear_enriched_outputs(&out)?;

    fs::write(out.join("book_ir.json"), &book_ir_json)
        .map_err(|e| format!("cannot write book_ir.json: {e}"))?;
    fs::write(out.join("structure.json"), &structure_json)
        .map_err(|e| format!("cannot write structure.json: {e}"))?;
    fs::write(out.join("blocks.jsonl"), &blocks_jsonl)
        .map_err(|e| format!("cannot write blocks.jsonl: {e}"))?;
    fs::write(out.join("manifest.json"), &manifest_json)
        .map_err(|e| format!("cannot write manifest.json: {e}"))?;

    let Some(command) = analyzer_command else {
        eprintln!(
            "Compiled {path} -> {out_dir}/ (4 files, {} blocks, {} chapters, profile: {})",
            book_ir.blocks.len(),
            book_ir.chapters.len(),
            chapter_analysis::profile_name(profile),
        );
        return Ok(());
    };

    let total_analyzable_chapter_count = book_ir
        .chapters
        .iter()
        .filter(|chapter| !chapter.is_noise && chapter.block_count > 0)
        .count();
    let mut status = CompileStatus {
        schema_version: book_analysis::BOOK_ANALYSIS_VERSION.to_owned(),
        source_hash: source_hash.clone(),
        profile: chapter_analysis::profile_name(profile).to_owned(),
        ready_stages: vec!["parse".to_owned(), "normalize".to_owned()],
        analyzed_through: analyze_through,
        analyzed_chapter_count: 0,
        total_analyzable_chapter_count,
        complete: false,
    };
    book_analysis::write_compile_status(&out, &status)?;

    let analyzer = CommandChapterAnalyzer::new(command);
    let analyses = chapter_analysis::analyze_book(
        &book_ir,
        &analyzer,
        AnalysisOptions {
            max_parallelism: analysis_jobs
                .unwrap_or_else(|| AnalysisOptions::default().max_parallelism),
            profile,
            analyze_through,
        },
    )
    .map_err(|error| error.to_string())?;
    chapter_analysis::write_chapter_analyses(&out, &analyses).map_err(|error| error.to_string())?;
    status.ready_stages.push("chapter_analysis".to_owned());
    status.analyzed_chapter_count = analyses.len();
    book_analysis::write_compile_status(&out, &status)?;

    let synthesis =
        book_analysis::synthesize_book(&book_ir, &analyses, &analyzer, profile, analyze_through)
            .map_err(|error| error.to_string())?;
    let documents = book_analysis::SynthesisDocuments::new(&source_hash, synthesis.clone());
    book_analysis::write_synthesis_documents(&out, &documents)
        .map_err(|error| error.to_string())?;
    status.ready_stages.push("book_synthesis".to_owned());
    book_analysis::write_compile_status(&out, &status)?;

    let report = book_analysis::validate_grounding(
        &source_hash,
        &book_analysis::grounding_blocks(&book_ir),
        &analyses,
        &synthesis,
    );
    book_analysis::write_eval_report(&out, &report).map_err(|error| error.to_string())?;
    if !report.valid {
        return Err(format!(
            "grounding validation failed; see {}/eval_report.json",
            out.display()
        ));
    }
    status.ready_stages.push("grounding_validation".to_owned());
    status.complete = true;
    book_analysis::write_compile_status(&out, &status)?;

    let analysis_count = analyses.len();
    let file_count = 12 + analysis_count;
    eprintln!(
        "Compiled {path} -> {out_dir}/ ({file_count} files, {} blocks, {} chapters, {analysis_count} analyses, profile: {}, grounding warnings: {})",
        book_ir.blocks.len(),
        book_ir.chapters.len(),
        chapter_analysis::profile_name(profile),
        report.issues.len(),
    );
    Ok(())
}

fn cached_package_matches(
    out: &Path,
    source_hash: &str,
    profile: Profile,
    analyze_through: Option<u32>,
) -> bool {
    let Ok(status) = book_analysis::read_compile_status(out) else {
        return false;
    };
    status.complete
        && status.source_hash == source_hash
        && status.profile == chapter_analysis::profile_name(profile)
        && status.analyzed_through == analyze_through
        && status
            .ready_stages
            .iter()
            .any(|stage| stage == "grounding_validation")
        && validate_package(out).is_ok()
}

fn clear_enriched_outputs(out: &Path) -> Result<(), String> {
    for name in [
        "compile_status.json",
        "book_map.json",
        "concepts.json",
        "claims.json",
        "entities.json",
        "checkpoints.json",
        "recall_cards.json",
        "eval_report.json",
    ] {
        let path = out.join(name);
        if path.exists() {
            fs::remove_file(&path)
                .map_err(|error| format!("cannot replace {}: {error}", path.display()))?;
        }
    }
    let chapters_dir = out.join("chapters");
    let Ok(entries) = fs::read_dir(&chapters_dir) else {
        return Ok(());
    };
    for entry in entries {
        let entry = entry.map_err(|error| format!("cannot read chapters directory: {error}"))?;
        let path = entry.path();
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".analysis.json"))
        {
            fs::remove_file(&path)
                .map_err(|error| format!("cannot replace {}: {error}", path.display()))?;
        }
    }
    Ok(())
}

fn cmd_validate(args: &[String]) -> Result<(), String> {
    let [dir] = args else {
        return Err("validate requires exactly one package directory".to_owned());
    };

    let dir_path = PathBuf::from(dir);
    if !dir_path.is_dir() {
        return Err(format!("{dir} is not a directory"));
    }

    let stats = validate_package(&dir_path)
        .map_err(|errors| format!("validation errors:\n{}", errors.join("\n")))?;
    for name in [
        "manifest.json",
        "structure.json",
        "book_ir.json",
        "blocks.jsonl",
    ] {
        eprintln!("  {name}: ok");
    }
    if dir_path.join("compile_status.json").is_file() {
        eprintln!("  analysis artifacts: ok");
    }
    eprintln!(
        "Package at {dir} is valid ({} blocks, {} chapters).",
        stats.block_count, stats.chapter_count
    );
    Ok(())
}

fn option_value<'a>(args: &'a [String], index: usize, option: &str) -> Result<&'a str, String> {
    args.get(index)
        .filter(|value| !value.starts_with('-'))
        .map(String::as_str)
        .ok_or_else(|| format!("{option} requires a value"))
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct PackageStats {
    block_count: usize,
    chapter_count: usize,
}

fn validate_package(dir: &Path) -> Result<PackageStats, Vec<String>> {
    let mut errors = Vec::new();
    let manifest = read_json_file(dir, "manifest.json", &mut errors);
    let structure = read_json_file(dir, "structure.json", &mut errors);
    let book_ir = read_json_file(dir, "book_ir.json", &mut errors);
    let blocks = read_jsonl_file(dir, "blocks.jsonl", &mut errors);

    let (Some(manifest), Some(structure), Some(book_ir), Some(blocks)) =
        (manifest, structure, book_ir, blocks)
    else {
        return Err(errors);
    };

    validate_package_values(&manifest, &structure, &book_ir, &blocks, &mut errors);
    validate_enriched_package(dir, &manifest, &blocks, &mut errors);
    if errors.is_empty() {
        Ok(PackageStats {
            block_count: blocks.len(),
            chapter_count: book_ir
                .get("chapters")
                .and_then(Value::as_array)
                .map_or(0, Vec::len),
        })
    } else {
        Err(errors)
    }
}

fn validate_enriched_package(
    dir: &Path,
    manifest: &Value,
    blocks: &[Value],
    errors: &mut Vec<String>,
) {
    if !dir.join("compile_status.json").is_file() {
        return;
    }
    let Some(status_value) = read_json_file(dir, "compile_status.json", errors) else {
        return;
    };
    let status: CompileStatus = match serde_json::from_value(status_value) {
        Ok(status) => status,
        Err(error) => {
            errors.push(format!("compile_status.json: invalid schema: {error}"));
            return;
        }
    };
    if status.schema_version != book_analysis::BOOK_ANALYSIS_VERSION {
        errors.push("compile_status.json: unsupported schema_version".to_owned());
    }
    if manifest.get("source_hash").and_then(Value::as_str) != Some(&status.source_hash) {
        errors.push("compile_status.json: source_hash does not match manifest.json".to_owned());
    }
    if manifest.get("profile").and_then(Value::as_str) != Some(&status.profile) {
        errors.push("compile_status.json: profile does not match manifest.json".to_owned());
    }
    let expected_stage_order = [
        "parse",
        "normalize",
        "chapter_analysis",
        "book_synthesis",
        "grounding_validation",
    ];
    if status
        .ready_stages
        .iter()
        .map(String::as_str)
        .ne(
            expected_stage_order[..status.ready_stages.len().min(expected_stage_order.len())]
                .iter()
                .copied(),
        )
        || status.ready_stages.len() > expected_stage_order.len()
    {
        errors.push("compile_status.json: ready_stages are not a valid pipeline prefix".to_owned());
    }
    if !status.complete {
        errors.push("compile_status.json: compilation is incomplete".to_owned());
    } else if status.ready_stages.last().map(String::as_str) != Some("grounding_validation") {
        errors.push(
            "compile_status.json: complete package must reach grounding_validation".to_owned(),
        );
    }

    let analyses = if status
        .ready_stages
        .iter()
        .any(|stage| stage == "chapter_analysis")
    {
        read_chapter_analyses(dir, status.analyzed_chapter_count, errors)
    } else {
        Vec::new()
    };
    if analyses
        .iter()
        .any(|analysis| analysis.analysis_profile != status.profile)
    {
        errors.push("chapter analyses: analysis_profile does not match compile status".to_owned());
    }

    let documents = if status
        .ready_stages
        .iter()
        .any(|stage| stage == "book_synthesis")
    {
        match book_analysis::read_synthesis_documents(dir) {
            Ok(documents) => {
                validate_document_metadata(&documents, &status, errors);
                if let Err(error) =
                    book_analysis::validate_synthesis_documents(&documents, &analyses)
                {
                    errors.push(format!("book synthesis: {error}"));
                }
                let expected = book_analysis::SynthesisDocuments::new(
                    &status.source_hash,
                    documents.generated(),
                );
                if expected.recall_cards != documents.recall_cards {
                    errors.push(
                        "recall_cards.json: records do not match checkpoints.json".to_owned(),
                    );
                }
                Some(documents)
            }
            Err(error) => {
                errors.push(error);
                None
            }
        }
    } else {
        None
    };

    if status
        .ready_stages
        .iter()
        .any(|stage| stage == "grounding_validation")
    {
        let Some(report_value) = read_json_file(dir, "eval_report.json", errors) else {
            return;
        };
        let report: EvalReport = match serde_json::from_value(report_value) {
            Ok(report) => report,
            Err(error) => {
                errors.push(format!("eval_report.json: invalid schema: {error}"));
                return;
            }
        };
        let Some(documents) = documents else {
            errors.push("eval_report.json: book synthesis documents are unavailable".to_owned());
            return;
        };
        let block_map = grounding_blocks_from_values(blocks, errors);
        let expected = book_analysis::validate_grounding(
            &status.source_hash,
            &block_map,
            &analyses,
            &documents.generated(),
        );
        if report != expected {
            errors.push("eval_report.json: report does not match package contents".to_owned());
        }
        if !report.valid {
            errors.push("eval_report.json: grounding validation failed".to_owned());
        }
    }
}

fn validate_document_metadata(
    documents: &book_analysis::SynthesisDocuments,
    status: &CompileStatus,
    errors: &mut Vec<String>,
) {
    for (name, version, source_hash) in [
        (
            "book_map.json",
            &documents.book_map.schema_version,
            &documents.book_map.source_hash,
        ),
        (
            "concepts.json",
            &documents.concepts.schema_version,
            &documents.concepts.source_hash,
        ),
        (
            "claims.json",
            &documents.claims.schema_version,
            &documents.claims.source_hash,
        ),
        (
            "entities.json",
            &documents.entities.schema_version,
            &documents.entities.source_hash,
        ),
        (
            "checkpoints.json",
            &documents.checkpoints.schema_version,
            &documents.checkpoints.source_hash,
        ),
        (
            "recall_cards.json",
            &documents.recall_cards.schema_version,
            &documents.recall_cards.source_hash,
        ),
    ] {
        if version != book_analysis::BOOK_ANALYSIS_VERSION {
            errors.push(format!("{name}: unsupported schema_version"));
        }
        if source_hash != &status.source_hash {
            errors.push(format!("{name}: source_hash does not match compile status"));
        }
    }
}

fn read_chapter_analyses(
    dir: &Path,
    expected_count: usize,
    errors: &mut Vec<String>,
) -> Vec<ChapterAnalysis> {
    let chapters_dir = dir.join("chapters");
    let entries = match fs::read_dir(&chapters_dir) {
        Ok(entries) => entries,
        Err(error) => {
            errors.push(format!("chapters: {error}"));
            return Vec::new();
        }
    };
    let mut paths = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(".analysis.json"))
        })
        .collect::<Vec<_>>();
    paths.sort();
    let mut analyses = Vec::new();
    for path in paths {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("chapter analysis");
        match fs::read(&path)
            .map_err(|error| error.to_string())
            .and_then(|bytes| {
                serde_json::from_slice::<ChapterAnalysis>(&bytes).map_err(|error| error.to_string())
            }) {
            Ok(analysis) => analyses.push(analysis),
            Err(error) => errors.push(format!("chapters/{name}: invalid schema: {error}")),
        }
    }
    if analyses.len() != expected_count {
        errors.push(format!(
            "chapter analysis count mismatch: status is {expected_count}, files are {}",
            analyses.len()
        ));
    }
    analyses
}

fn grounding_blocks_from_values(
    blocks: &[Value],
    errors: &mut Vec<String>,
) -> BTreeMap<String, GroundingBlock> {
    let mut result = BTreeMap::new();
    for (index, block) in blocks.iter().enumerate() {
        let path = format!("blocks.jsonl line {}", index + 1);
        let (Some(block_id), Some(chapter_index), Some(text), Some(text_fingerprint)) = (
            block.get("block_id").and_then(Value::as_str),
            block
                .get("chapter_index")
                .and_then(Value::as_u64)
                .and_then(|value| u32::try_from(value).ok()),
            block.get("text").and_then(Value::as_str),
            block.get("text_fingerprint").and_then(Value::as_str),
        ) else {
            errors.push(format!("{path}: cannot build grounding index"));
            continue;
        };
        result.insert(
            block_id.to_owned(),
            GroundingBlock {
                chapter_index,
                text: text.to_owned(),
                text_fingerprint: text_fingerprint.to_owned(),
            },
        );
    }
    result
}

fn read_json_file(dir: &Path, name: &str, errors: &mut Vec<String>) -> Option<Value> {
    let text = match fs::read_to_string(dir.join(name)) {
        Ok(text) => text,
        Err(error) => {
            errors.push(format!("{name}: {error}"));
            return None;
        }
    };
    match serde_json::from_str(&text) {
        Ok(Value::Object(object)) => Some(Value::Object(object)),
        Ok(_) => {
            errors.push(format!("{name}: top-level value must be an object"));
            None
        }
        Err(error) => {
            errors.push(format!("{name}: invalid JSON: {error}"));
            None
        }
    }
}

fn read_jsonl_file(dir: &Path, name: &str, errors: &mut Vec<String>) -> Option<Vec<Value>> {
    let text = match fs::read_to_string(dir.join(name)) {
        Ok(text) => text,
        Err(error) => {
            errors.push(format!("{name}: {error}"));
            return None;
        }
    };
    let mut values = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str(line) {
            Ok(Value::Object(object)) => values.push(Value::Object(object)),
            Ok(_) => errors.push(format!(
                "{name}: line {} must contain a JSON object",
                index + 1
            )),
            Err(error) => errors.push(format!("{name}: line {}: {error}", index + 1)),
        }
    }
    Some(values)
}

fn validate_package_values(
    manifest: &Value,
    structure: &Value,
    book_ir: &Value,
    blocks: &[Value],
    errors: &mut Vec<String>,
) {
    let format_version = required_str(manifest, "manifest.json", "format_version", errors);
    if format_version.is_some_and(|value| value != "0.1") {
        errors.push("manifest.json: unsupported format_version".to_owned());
    }
    let profile = required_str(manifest, "manifest.json", "profile", errors);
    if profile.is_some_and(|value| !matches!(value, "basic" | "standard" | "deep")) {
        errors.push("manifest.json: profile must be basic, standard, or deep".to_owned());
    }
    let source_hash = required_str(manifest, "manifest.json", "source_hash", errors);
    if source_hash.is_some_and(|value| {
        value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
    }) {
        errors.push("manifest.json: source_hash must be a 64-character hex digest".to_owned());
    }

    let manifest_block_count = required_usize(manifest, "manifest.json", "block_count", errors);
    let manifest_chapter_count = required_usize(manifest, "manifest.json", "chapter_count", errors);
    let structure_spine_count = required_usize(structure, "structure.json", "spine_count", errors);
    let structure_chapter_count =
        required_usize(structure, "structure.json", "chapter_count", errors);
    let structure_toc = required_array(structure, "structure.json", "toc", errors);
    let structure_spine = required_array(structure, "structure.json", "spine", errors);
    let structure_chapters = required_array(structure, "structure.json", "chapters", errors);
    let book_spine = required_array(book_ir, "book_ir.json", "spine", errors);
    let book_chapters = required_array(book_ir, "book_ir.json", "chapters", errors);
    let book_blocks = required_array(book_ir, "book_ir.json", "blocks", errors);
    let _ = structure_toc;

    compare_count(
        "manifest.json block_count",
        manifest_block_count,
        "blocks.jsonl records",
        Some(blocks.len()),
        errors,
    );
    compare_count(
        "manifest.json block_count",
        manifest_block_count,
        "book_ir.json blocks",
        book_blocks.map(<[Value]>::len),
        errors,
    );
    compare_count(
        "manifest.json chapter_count",
        manifest_chapter_count,
        "book_ir.json chapters",
        book_chapters.map(<[Value]>::len),
        errors,
    );
    compare_count(
        "structure.json chapter_count",
        structure_chapter_count,
        "structure.json chapters",
        structure_chapters.map(<[Value]>::len),
        errors,
    );
    compare_count(
        "manifest.json chapter_count",
        manifest_chapter_count,
        "structure.json chapter_count",
        structure_chapter_count,
        errors,
    );
    compare_count(
        "structure.json spine_count",
        structure_spine_count,
        "structure.json spine",
        structure_spine.map(<[Value]>::len),
        errors,
    );
    compare_count(
        "structure.json spine_count",
        structure_spine_count,
        "book_ir.json spine",
        book_spine.map(<[Value]>::len),
        errors,
    );

    if let Some(book_blocks) = book_blocks {
        if book_blocks != blocks {
            errors.push("blocks.jsonl: records do not match book_ir.json blocks".to_owned());
        }
    }
    validate_block_references(blocks, book_chapters.map_or(0, <[Value]>::len), errors);
    if let Some(chapters) = book_chapters {
        validate_chapter_boundaries(chapters, blocks, errors);
    }
}

fn validate_block_references(blocks: &[Value], chapter_count: usize, errors: &mut Vec<String>) {
    let mut by_id = BTreeMap::<String, &Value>::new();
    for (index, block) in blocks.iter().enumerate() {
        let path = format!("blocks.jsonl line {}", index + 1);
        let Some(block_id) = required_str(block, &path, "block_id", errors) else {
            continue;
        };
        if block_id.is_empty() {
            errors.push(format!("{path}: block_id must not be empty"));
        } else if by_id.insert(block_id.to_owned(), block).is_some() {
            errors.push(format!("{path}: duplicate block_id {block_id}"));
        }
        let chapter_index = required_usize(block, &path, "chapter_index", errors);
        if chapter_index.is_some_and(|value| value >= chapter_count) {
            errors.push(format!("{path}: chapter_index is out of range"));
        }
        let fingerprint = required_str(block, &path, "text_fingerprint", errors);
        if fingerprint.is_some_and(|value| {
            value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
        }) {
            errors.push(format!(
                "{path}: text_fingerprint must be a 64-character hex digest"
            ));
        }
        if !block.get("source_ref").is_some_and(Value::is_object) {
            errors.push(format!("{path}: source_ref must be an object"));
        }
    }

    for (index, block) in blocks.iter().enumerate() {
        let path = format!("blocks.jsonl line {}", index + 1);
        for field in ["footnote_refs", "referenced_by"] {
            let Some(references) = block.get(field).and_then(Value::as_array) else {
                errors.push(format!("{path}: {field} must be an array"));
                continue;
            };
            for reference in references {
                let Some(reference) = reference.as_str() else {
                    errors.push(format!("{path}: {field} entries must be strings"));
                    continue;
                };
                if !by_id.contains_key(reference) {
                    errors.push(format!(
                        "{path}: {field} references missing block {reference}"
                    ));
                }
            }
        }
    }
}

fn validate_chapter_boundaries(chapters: &[Value], blocks: &[Value], errors: &mut Vec<String>) {
    let ids = blocks
        .iter()
        .filter_map(|block| block.get("block_id").and_then(Value::as_str))
        .collect::<BTreeSet<_>>();
    for (index, chapter) in chapters.iter().enumerate() {
        for field in ["first_block_id", "last_block_id"] {
            let Some(value) = chapter.get(field) else {
                errors.push(format!("book_ir.json chapter {index}: missing {field}"));
                continue;
            };
            if value.is_null() {
                continue;
            }
            let Some(block_id) = value.as_str() else {
                errors.push(format!(
                    "book_ir.json chapter {index}: {field} must be a string or null"
                ));
                continue;
            };
            if !ids.contains(block_id) {
                errors.push(format!(
                    "book_ir.json chapter {index}: {field} references missing block {block_id}"
                ));
            }
        }
    }
}

fn required_array<'a>(
    value: &'a Value,
    file: &str,
    field: &str,
    errors: &mut Vec<String>,
) -> Option<&'a [Value]> {
    match value.get(field).and_then(Value::as_array) {
        Some(array) => Some(array),
        None => {
            errors.push(format!("{file}: {field} must be an array"));
            None
        }
    }
}

fn required_str<'a>(
    value: &'a Value,
    file: &str,
    field: &str,
    errors: &mut Vec<String>,
) -> Option<&'a str> {
    match value.get(field).and_then(Value::as_str) {
        Some(value) => Some(value),
        None => {
            errors.push(format!("{file}: {field} must be a string"));
            None
        }
    }
}

fn required_usize(
    value: &Value,
    file: &str,
    field: &str,
    errors: &mut Vec<String>,
) -> Option<usize> {
    match value
        .get(field)
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
    {
        Some(value) => Some(value),
        None => {
            errors.push(format!("{file}: {field} must be a non-negative integer"));
            None
        }
    }
}

fn compare_count(
    left_name: &str,
    left: Option<usize>,
    right_name: &str,
    right: Option<usize>,
    errors: &mut Vec<String>,
) {
    if let (Some(left), Some(right)) = (left, right) {
        if left != right {
            errors.push(format!(
                "count mismatch: {left_name} is {left}, but {right_name} is {right}"
            ));
        }
    }
}

fn print_help() {
    println!("codexia — Book Agent compiler");
    println!();
    println!("Usage:");
    println!("  codexia parse <input.epub> [--output <file>]");
    println!("  codexia compile <input.epub> --out <dir> [--profile basic|standard|deep]");
    println!("    [--analyzer-command <executable>] [--analysis-jobs <count>]");
    println!("    [--analyze-through <chapter-number>] [--force]");
    println!("  codexia validate <dir>");
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            use std::fmt::Write;
            let _ = write!(s, "{b:02x}");
            s
        })
}
