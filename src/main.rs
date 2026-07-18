//! codexia CLI — Book Agent compiler
//!
//! Usage:
//!   codexia parse <input.epub> [--output <file>]           Parse EPUB → BookIR
//!   codexia compile <input.epub> --out <dir>                Compile full Book Package
//!   codexia validate <dir>                                  Validate a compiled package
//!   codexia help                                            Show help

#![forbid(unsafe_code)]

use std::{env, fs, path::PathBuf, process::ExitCode};

use codexia::epub_parser;
use codexia::ir;
use codexia::ir::Profile;
use codexia::normalizer;

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
                output = Some(
                    args.get(index)
                        .ok_or_else(|| "--output requires a path".to_owned())?
                        .clone(),
                );
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
    let mut path = None;

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--out" | "-o" => {
                index += 1;
                out_dir = Some(
                    args.get(index)
                        .ok_or_else(|| "--out requires a directory".to_owned())?
                        .clone(),
                );
            }
            "--profile" | "-p" => {
                index += 1;
                profile = args
                    .get(index)
                    .ok_or_else(|| "--profile requires a value".to_owned())?
                    .clone();
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
    let dir = args
        .first()
        .ok_or_else(|| "validate requires a package directory".to_owned())?;

    let dir_path = PathBuf::from(dir);
    if !dir_path.is_dir() {
        return Err(format!("{dir} is not a directory"));
    }

    let mut errors = Vec::new();

    match fs::read_to_string(dir_path.join("manifest.json")) {
        Ok(_) => eprintln!("  manifest.json: ok"),
        Err(e) => errors.push(format!("manifest.json: {e}")),
    }
    match fs::read_to_string(dir_path.join("structure.json")) {
        Ok(_) => eprintln!("  structure.json: ok"),
        Err(e) => errors.push(format!("structure.json: {e}")),
    }
    match fs::read_to_string(dir_path.join("book_ir.json")) {
        Ok(_) => eprintln!("  book_ir.json: ok"),
        Err(e) => errors.push(format!("book_ir.json: {e}")),
    }
    match fs::read_to_string(dir_path.join("blocks.jsonl")) {
        Ok(_) => eprintln!("  blocks.jsonl: ok"),
        Err(e) => errors.push(format!("blocks.jsonl: {e}")),
    }

    if errors.is_empty() {
        eprintln!("Package at {dir} is valid.");
        Ok(())
    } else {
        Err(format!("validation errors:\n{}", errors.join("\n")))
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
