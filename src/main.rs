//! codexia CLI — Book Agent compiler
//!
//! Usage:
//!   codexia parse <input.epub> [--output <file>]           Parse EPUB → BookIR
//!   codexia compile <input.epub> --out <dir>                Compile full Book Package
//!   codexia validate <dir>                                  Validate a compiled package
//!   codexia help                                            Show help

#![forbid(unsafe_code)]

use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

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

    let bytes = fs::read(&path).map_err(|e| format!("cannot read {path}: {e}"))?;
    let source_hash = hex_encode(pagelet::core::ContentHash::from_bytes(&bytes).as_bytes());

    let book_ir = epub_parser::parse_epub(&bytes)?;
    let book_ir_json = ir::book_ir_to_json(&book_ir);

    let compiled = normalizer::compile(&book_ir, profile, &source_hash);
    let structure_json = normalizer::structure_json(&compiled.structure);
    let blocks_jsonl = normalizer::blocks_jsonl(&book_ir.blocks);
    let manifest_json = normalizer::manifest_json(&compiled.manifest);

    let out = PathBuf::from(&out_dir);
    fs::create_dir_all(&out).map_err(|e| format!("cannot create directory {out_dir}: {e}"))?;

    fs::write(out.join("book_ir.json"), &book_ir_json)
        .map_err(|e| format!("cannot write book_ir.json: {e}"))?;
    fs::write(out.join("structure.json"), &structure_json)
        .map_err(|e| format!("cannot write structure.json: {e}"))?;
    fs::write(out.join("blocks.jsonl"), &blocks_jsonl)
        .map_err(|e| format!("cannot write blocks.jsonl: {e}"))?;
    fs::write(out.join("manifest.json"), &manifest_json)
        .map_err(|e| format!("cannot write manifest.json: {e}"))?;

    let file_count = 4;
    eprintln!(
        "Compiled {path} -> {out_dir}/ ({file_count} files, {} blocks, {} chapters, profile: {})",
        book_ir.blocks.len(),
        book_ir.chapters.len(),
        match profile {
            Profile::Basic => "basic",
            Profile::Standard => "standard",
            Profile::Deep => "deep",
        },
    );
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
