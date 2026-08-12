//! Provider-neutral chapter analysis pipeline.
//!
//! The core crate owns prompt construction, context boundaries, scheduling,
//! response validation, and package output. A model provider is an adapter at
//! the [`ChapterAnalyzer`] boundary; the built-in command adapter exchanges one
//! JSON request and response over stdin/stdout for every chapter.

use std::{
    fmt, fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    },
    thread,
};

use serde::{Deserialize, Serialize};

use crate::ir::{Block, BookIr, Chapter, TocEntry};

/// Version of the persisted chapter-analysis document and provider protocol.
pub const CHAPTER_ANALYSIS_VERSION: &str = "0.1";

/// System rules shared by every chapter-analysis prompt.
pub const CHAPTER_ANALYSIS_SYSTEM_PROMPT: &str = r#"You analyze exactly one chapter of a book. Use only the supplied chapter blocks, table of contents, and prior-chapter context. Define concepts as this book uses them. Every evidence-bearing claim, difficult passage, and entity appearance must cite supplied block IDs with character offsets. Do not cite later chapters, invent sources, or wrap the JSON response in Markdown. Return one JSON object matching output_schema."#;

const PROMPT_TASKS: &[PromptTask] = &[
    PromptTask {
        id: "summary",
        instruction: "Write one-sentence, short, and deep summaries, then explain this chapter's role in the book.",
    },
    PromptTask {
        id: "concepts",
        instruction: "Extract key concepts and define each according to its specific meaning in this book, not as a generic dictionary entry.",
    },
    PromptTask {
        id: "claims",
        instruction: "Extract claims and the argument flow, including evidence, assumptions, counterpoints, and relations between claims.",
    },
    PromptTask {
        id: "difficult_passages",
        instruction: "Identify passages likely to block understanding and explain both the difficulty and the passage clearly.",
    },
    PromptTask {
        id: "entities",
        instruction: "Extract people, organizations, places, and events, describing each entity's role in this chapter.",
    },
];

const OUTPUT_SCHEMA: &str = r#"{"summary":{"one_sentence":"string","short":"string","deep":"string","role_in_book":"string"},"key_ideas":["string"],"concepts":[{"concept_id":"string","name":"string","aliases":["string"],"definition_in_this_book":"string","source_refs":[{"block_id":"string","start_char":0,"end_char":0,"text_fingerprint":"string"}]}],"claims":[{"claim_id":"string","claim":"string","type":"thesis|supporting|counterclaim|inference","evidence":[{"text":"string","source_ref":{"block_id":"string","start_char":0,"end_char":0,"text_fingerprint":"string"}}],"assumptions":["string"],"counterpoints":["string"]}],"argument_flow":[{"from_claim_id":"string|null","to_claim_id":"string","relation":"supports|challenges|qualifies|follows_from","explanation":"string"}],"difficult_passages":[{"source_ref":{"block_id":"string","start_char":0,"end_char":0,"text_fingerprint":"string"},"reason":"string","explanation":"string"}],"entities":[{"entity_id":"string","name":"string","type":"person|organization|place|event","role_in_chapter":"string","source_refs":[{"block_id":"string","start_char":0,"end_char":0,"text_fingerprint":"string"}]}]}"#;

/// One independently testable instruction within the combined prompt.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize)]
pub struct PromptTask {
    pub id: &'static str,
    pub instruction: &'static str,
}

/// Prompt content sent to a provider for one chapter.
#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
pub struct ChapterAnalysisRequest {
    pub protocol_version: &'static str,
    pub task: &'static str,
    pub system_prompt: &'static str,
    pub prompt_tasks: &'static [PromptTask],
    pub context: ChapterAnalysisContext,
    pub output_schema: serde_json::Value,
}

/// Book and chapter material that the model may use.
#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
pub struct ChapterAnalysisContext {
    pub book_title: Option<String>,
    pub table_of_contents: Vec<TocContext>,
    pub prior_chapters: Vec<PriorChapterContext>,
    pub chapter: ChapterContext,
}

/// A table-of-contents entry preserved as a tree.
#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
pub struct TocContext {
    pub label: String,
    pub href: String,
    pub children: Vec<TocContext>,
}

/// Bounded context from a chapter before the one being analyzed.
#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
pub struct PriorChapterContext {
    pub chapter_id: String,
    pub title: String,
    pub condensed_summary: String,
}

/// Full normalized content for the current chapter.
#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
pub struct ChapterContext {
    pub chapter_id: String,
    pub spine_index: u32,
    pub title: String,
    pub content_hash: String,
    pub blocks: Vec<PromptBlock>,
}

/// A source-addressable block exposed to the model.
#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
pub struct PromptBlock {
    pub block_id: String,
    pub kind: String,
    pub text: String,
    pub text_fingerprint: String,
}

/// Model-generated fields returned by a provider adapter.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct GeneratedChapterAnalysis {
    pub summary: ChapterSummary,
    pub key_ideas: Vec<String>,
    pub concepts: Vec<AnalyzedConcept>,
    pub claims: Vec<AnalyzedClaim>,
    pub argument_flow: Vec<ArgumentStep>,
    pub difficult_passages: Vec<DifficultPassage>,
    pub entities: Vec<AnalyzedEntity>,
}

/// Layered chapter summary and its place in the whole book.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct ChapterSummary {
    pub one_sentence: String,
    pub short: String,
    pub deep: String,
    pub role_in_book: String,
}

/// A concept defined in this book's own terms.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct AnalyzedConcept {
    pub concept_id: String,
    pub name: String,
    pub aliases: Vec<String>,
    pub definition_in_this_book: String,
    pub source_refs: Vec<AnalysisSourceRef>,
}

/// A chapter claim with its evidence and qualifications.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct AnalyzedClaim {
    pub claim_id: String,
    pub claim: String,
    #[serde(rename = "type")]
    pub claim_type: ClaimType,
    pub evidence: Vec<ClaimEvidence>,
    pub assumptions: Vec<String>,
    pub counterpoints: Vec<String>,
}

/// Supported claim roles within an argument.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimType {
    Thesis,
    Supporting,
    Counterclaim,
    Inference,
}

/// Text supporting a claim and the block it came from.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct ClaimEvidence {
    pub text: String,
    pub source_ref: AnalysisSourceRef,
}

/// One directed step in the chapter's argument.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct ArgumentStep {
    pub from_claim_id: Option<String>,
    pub to_claim_id: String,
    pub relation: ArgumentRelation,
    pub explanation: String,
}

/// Relationship between two positions in the argument flow.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArgumentRelation {
    Supports,
    Challenges,
    Qualifies,
    FollowsFrom,
}

/// A passage needing extra explanation.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct DifficultPassage {
    pub source_ref: AnalysisSourceRef,
    pub reason: String,
    pub explanation: String,
}

/// A named entity and its role in this chapter.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct AnalyzedEntity {
    pub entity_id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub entity_type: EntityType,
    pub role_in_chapter: String,
    pub source_refs: Vec<AnalysisSourceRef>,
}

/// Entity classes extracted by the chapter prompt.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityType {
    Person,
    Organization,
    Place,
    Event,
}

/// Character-range citation into one normalized block.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct AnalysisSourceRef {
    pub block_id: String,
    pub start_char: usize,
    pub end_char: usize,
    pub text_fingerprint: String,
}

/// Persisted analysis envelope owned by Codexia rather than the provider.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct ChapterAnalysis {
    pub schema_version: String,
    pub chapter_id: String,
    pub spine_index: u32,
    pub chapter_title: String,
    pub source_content_hash: String,
    #[serde(flatten)]
    pub generated: GeneratedChapterAnalysis,
}

/// Abstraction over an LLM provider or deterministic test double.
pub trait ChapterAnalyzer: Sync {
    fn analyze(
        &self,
        request: &ChapterAnalysisRequest,
    ) -> Result<GeneratedChapterAnalysis, AnalysisError>;
}

/// Adapter for an executable that reads one request JSON from stdin and writes
/// one `GeneratedChapterAnalysis` JSON object to stdout.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CommandChapterAnalyzer {
    executable: PathBuf,
}

impl CommandChapterAnalyzer {
    #[must_use]
    pub fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
        }
    }
}

impl ChapterAnalyzer for CommandChapterAnalyzer {
    fn analyze(
        &self,
        request: &ChapterAnalysisRequest,
    ) -> Result<GeneratedChapterAnalysis, AnalysisError> {
        let mut child = Command::new(&self.executable)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| {
                AnalysisError::new(format!(
                    "cannot start analyzer command {}: {error}",
                    self.executable.display()
                ))
            })?;

        let request_json = serde_json::to_vec(request)
            .map_err(|error| AnalysisError::new(format!("cannot serialize request: {error}")))?;
        child
            .stdin
            .take()
            .ok_or_else(|| AnalysisError::new("analyzer command stdin is unavailable"))?
            .write_all(&request_json)
            .map_err(|error| {
                AnalysisError::new(format!("cannot send analyzer request: {error}"))
            })?;

        let output = child
            .wait_with_output()
            .map_err(|error| AnalysisError::new(format!("analyzer command failed: {error}")))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(AnalysisError::new(format!(
                "analyzer command exited with {}: {}",
                output.status,
                stderr.trim()
            )));
        }

        serde_json::from_slice(&output.stdout).map_err(|error| {
            AnalysisError::new(format!("analyzer command returned invalid JSON: {error}"))
        })
    }
}

/// Parallel execution settings for a chapter-analysis run.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct AnalysisOptions {
    pub max_parallelism: usize,
}

impl Default for AnalysisOptions {
    fn default() -> Self {
        let available = thread::available_parallelism().map_or(1, usize::from);
        Self {
            max_parallelism: available.min(4),
        }
    }
}

/// Analyze every non-noise chapter concurrently and return results in spine
/// order. All requests are built before fan-out, so no chapter can see output
/// synthesized from a later chapter.
pub fn analyze_book<A: ChapterAnalyzer>(
    book: &BookIr,
    analyzer: &A,
    options: AnalysisOptions,
) -> Result<Vec<ChapterAnalysis>, AnalysisError> {
    if options.max_parallelism == 0 {
        return Err(AnalysisError::new("max_parallelism must be at least 1"));
    }

    let requests = book
        .chapters
        .iter()
        .filter(|chapter| !chapter.is_noise && chapter.block_count > 0)
        .map(|chapter| build_request(book, chapter))
        .collect::<Result<Vec<_>, _>>()?;
    if requests.is_empty() {
        return Ok(Vec::new());
    }

    let next = AtomicUsize::new(0);
    let slots = Mutex::new(
        (0..requests.len())
            .map(|_| None)
            .collect::<Vec<Option<Result<ChapterAnalysis, AnalysisError>>>>(),
    );
    let worker_count = options.max_parallelism.min(requests.len());

    thread::scope(|scope| {
        for _ in 0..worker_count {
            scope.spawn(|| loop {
                let index = next.fetch_add(1, Ordering::Relaxed);
                let Some(request) = requests.get(index) else {
                    break;
                };
                let result = analyzer
                    .analyze(request)
                    .and_then(|generated| complete_analysis(request, generated));
                slots.lock().expect("analysis result lock")[index] = Some(result);
            });
        }
    });

    let mut analyses = Vec::with_capacity(requests.len());
    let mut failures = Vec::new();
    for (request, result) in requests
        .into_iter()
        .zip(slots.into_inner().expect("analysis results"))
    {
        match result.expect("worker stores every claimed job") {
            Ok(analysis) => analyses.push(analysis),
            Err(error) => failures.push(format!("{}: {error}", request.context.chapter.chapter_id)),
        }
    }
    if failures.is_empty() {
        Ok(analyses)
    } else {
        Err(AnalysisError::new(format!(
            "chapter analysis failed:\n{}",
            failures.join("\n")
        )))
    }
}

/// Write one stable, pretty-printed JSON document per analyzed chapter.
pub fn write_chapter_analyses(
    package_dir: &Path,
    analyses: &[ChapterAnalysis],
) -> Result<Vec<PathBuf>, AnalysisError> {
    let chapters_dir = package_dir.join("chapters");
    fs::create_dir_all(&chapters_dir).map_err(|error| {
        AnalysisError::new(format!("cannot create {}: {error}", chapters_dir.display()))
    })?;

    let mut paths = Vec::with_capacity(analyses.len());
    for analysis in analyses {
        let path = chapters_dir.join(format!("{}.analysis.json", analysis.chapter_id));
        let mut json = serde_json::to_vec_pretty(analysis).map_err(|error| {
            AnalysisError::new(format!("cannot serialize {}: {error}", analysis.chapter_id))
        })?;
        json.push(b'\n');
        fs::write(&path, json).map_err(|error| {
            AnalysisError::new(format!("cannot write {}: {error}", path.display()))
        })?;
        paths.push(path);
    }
    Ok(paths)
}

fn build_request(
    book: &BookIr,
    chapter: &Chapter,
) -> Result<ChapterAnalysisRequest, AnalysisError> {
    let chapter_position = book
        .chapters
        .iter()
        .position(|candidate| candidate.spine_index == chapter.spine_index)
        .ok_or_else(|| AnalysisError::new("chapter is not present in BookIR"))?;
    let current_chapter_id = chapter_id(chapter.spine_index);
    let blocks = book
        .blocks
        .iter()
        .filter(|block| block.chapter_index == chapter.spine_index)
        .map(prompt_block)
        .collect::<Vec<_>>();
    if blocks.is_empty() {
        return Err(AnalysisError::new(format!(
            "{current_chapter_id} has no normalized blocks"
        )));
    }

    let prior_chapters = book.chapters[..chapter_position]
        .iter()
        .filter(|prior| !prior.is_noise && prior.block_count > 0)
        .map(|prior| PriorChapterContext {
            chapter_id: chapter_id(prior.spine_index),
            title: prior.title.to_string(),
            condensed_summary: condense(&prior.visible_text, 600),
        })
        .collect();
    let output_schema = serde_json::from_str(OUTPUT_SCHEMA)
        .map_err(|error| AnalysisError::new(format!("invalid embedded output schema: {error}")))?;

    Ok(ChapterAnalysisRequest {
        protocol_version: CHAPTER_ANALYSIS_VERSION,
        task: "chapter_analysis",
        system_prompt: CHAPTER_ANALYSIS_SYSTEM_PROMPT,
        prompt_tasks: PROMPT_TASKS,
        context: ChapterAnalysisContext {
            book_title: book.metadata.title.as_deref().map(str::to_owned),
            table_of_contents: book.toc.iter().map(toc_context).collect(),
            prior_chapters,
            chapter: ChapterContext {
                chapter_id: current_chapter_id,
                spine_index: chapter.spine_index,
                title: chapter.title.to_string(),
                content_hash: chapter.content_hash.clone(),
                blocks,
            },
        },
        output_schema,
    })
}

fn complete_analysis(
    request: &ChapterAnalysisRequest,
    generated: GeneratedChapterAnalysis,
) -> Result<ChapterAnalysis, AnalysisError> {
    validate_generated(&generated)?;
    let chapter = &request.context.chapter;
    Ok(ChapterAnalysis {
        schema_version: CHAPTER_ANALYSIS_VERSION.to_owned(),
        chapter_id: chapter.chapter_id.clone(),
        spine_index: chapter.spine_index,
        chapter_title: chapter.title.clone(),
        source_content_hash: chapter.content_hash.clone(),
        generated,
    })
}

fn validate_generated(generated: &GeneratedChapterAnalysis) -> Result<(), AnalysisError> {
    for (name, value) in [
        ("summary.one_sentence", &generated.summary.one_sentence),
        ("summary.short", &generated.summary.short),
        ("summary.deep", &generated.summary.deep),
        ("summary.role_in_book", &generated.summary.role_in_book),
    ] {
        if value.trim().is_empty() {
            return Err(AnalysisError::new(format!("{name} must not be empty")));
        }
    }
    Ok(())
}

fn prompt_block(block: &Block) -> PromptBlock {
    PromptBlock {
        block_id: block.block_id.clone(),
        kind: block.kind.clone(),
        text: block.text.to_string(),
        text_fingerprint: block.text_fingerprint.clone(),
    }
}

fn toc_context(entry: &TocEntry) -> TocContext {
    TocContext {
        label: entry.label.to_string(),
        href: entry.href.to_string(),
        children: entry.children.iter().map(toc_context).collect(),
    }
}

fn chapter_id(spine_index: u32) -> String {
    format!("chapter_{:03}", spine_index.saturating_add(1))
}

fn condense(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let mut condensed = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        condensed.push('…');
    }
    condensed
}

/// Error returned by prompt construction, providers, scheduling, or output.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct AnalysisError {
    message: String,
}

impl AnalysisError {
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for AnalysisError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for AnalysisError {}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        sync::{Arc, Barrier, Mutex},
    };

    use super::*;
    use crate::ir::{Metadata, SourceRef, SpineEntry};

    #[derive(Debug)]
    struct RecordingAnalyzer {
        requests: Mutex<Vec<ChapterAnalysisRequest>>,
        barrier: Barrier,
    }

    impl ChapterAnalyzer for RecordingAnalyzer {
        fn analyze(
            &self,
            request: &ChapterAnalysisRequest,
        ) -> Result<GeneratedChapterAnalysis, AnalysisError> {
            self.requests
                .lock()
                .expect("requests")
                .push(request.clone());
            self.barrier.wait();
            Ok(generated_fixture(&request.context.chapter.chapter_id))
        }
    }

    #[test]
    fn builds_grounded_prompts_and_collects_parallel_results_in_spine_order() {
        let book = fixture_book();
        let analyzer = Arc::new(RecordingAnalyzer {
            requests: Mutex::new(Vec::new()),
            barrier: Barrier::new(2),
        });

        let analyses = analyze_book(
            &book,
            analyzer.as_ref(),
            AnalysisOptions { max_parallelism: 2 },
        )
        .expect("analyze book");

        assert_eq!(
            analyses
                .iter()
                .map(|analysis| analysis.chapter_id.as_str())
                .collect::<Vec<_>>(),
            ["chapter_001", "chapter_002"]
        );
        let by_id = analyzer
            .requests
            .lock()
            .expect("requests")
            .iter()
            .map(|request| (request.context.chapter.chapter_id.clone(), request.clone()))
            .collect::<BTreeMap<_, _>>();
        let second = &by_id["chapter_002"];
        assert_eq!(second.prompt_tasks.len(), 5);
        assert_eq!(second.context.prior_chapters.len(), 1);
        assert_eq!(second.context.prior_chapters[0].chapter_id, "chapter_001");
        assert_eq!(
            second.context.chapter.blocks[0].text,
            "Second chapter text."
        );
        assert!(second.output_schema.get("argument_flow").is_some());
    }

    #[test]
    fn rejects_invalid_provider_output_and_zero_parallelism() {
        struct InvalidAnalyzer;
        impl ChapterAnalyzer for InvalidAnalyzer {
            fn analyze(
                &self,
                _request: &ChapterAnalysisRequest,
            ) -> Result<GeneratedChapterAnalysis, AnalysisError> {
                let mut output = generated_fixture("chapter_001");
                output.summary.deep.clear();
                Ok(output)
            }
        }

        let book = fixture_book();
        let error = analyze_book(
            &book,
            &InvalidAnalyzer,
            AnalysisOptions { max_parallelism: 1 },
        )
        .expect_err("invalid output");
        assert!(error.to_string().contains("summary.deep must not be empty"));
        assert_eq!(
            analyze_book(
                &book,
                &InvalidAnalyzer,
                AnalysisOptions { max_parallelism: 0 },
            )
            .expect_err("zero workers")
            .to_string(),
            "max_parallelism must be at least 1"
        );
    }

    fn generated_fixture(chapter_id: &str) -> GeneratedChapterAnalysis {
        GeneratedChapterAnalysis {
            summary: ChapterSummary {
                one_sentence: format!("One sentence for {chapter_id}."),
                short: "Short summary.".to_owned(),
                deep: "Deep summary.".to_owned(),
                role_in_book: "Builds the foundation.".to_owned(),
            },
            key_ideas: vec!["A key idea".to_owned()],
            concepts: Vec::new(),
            claims: Vec::new(),
            argument_flow: Vec::new(),
            difficult_passages: Vec::new(),
            entities: Vec::new(),
        }
    }

    fn fixture_book() -> BookIr {
        let chapters = vec![fixture_chapter(0, "First"), fixture_chapter(1, "Second")];
        let blocks = vec![
            fixture_block(0, "first", "First chapter text."),
            fixture_block(1, "second", "Second chapter text."),
        ];
        BookIr {
            metadata: Metadata {
                title: Some(Arc::from("Fixture Book")),
                identifier: Some(Arc::from("fixture")),
                language: Some(Arc::from("en")),
                package_version: Arc::from("3.0"),
            },
            toc: vec![TocEntry {
                label: Arc::from("First"),
                href: Arc::from("first.xhtml"),
                children: Vec::new(),
            }],
            spine: vec![
                SpineEntry {
                    spine_index: 0,
                    idref: Arc::from("first"),
                    href: Some(Arc::from("first.xhtml")),
                    linear: true,
                },
                SpineEntry {
                    spine_index: 1,
                    idref: Arc::from("second"),
                    href: Some(Arc::from("second.xhtml")),
                    linear: true,
                },
            ],
            chapters,
            blocks,
        }
    }

    fn fixture_chapter(spine_index: u32, title: &str) -> Chapter {
        let text = if spine_index == 0 {
            "First chapter text."
        } else {
            "Second chapter text."
        };
        Chapter {
            spine_index,
            href: Arc::from(format!("{}.xhtml", title.to_lowercase())),
            title: Arc::from(title),
            block_count: 1,
            visible_text: Arc::from(text),
            content_hash: format!("hash-{spine_index}"),
            first_block_id: Some(format!("block-{spine_index}")),
            last_block_id: Some(format!("block-{spine_index}")),
            is_noise: false,
        }
    }

    fn fixture_block(chapter_index: u32, id: &str, text: &str) -> Block {
        Block {
            block_id: format!("block-{id}"),
            chapter_index,
            order: 0,
            kind: "paragraph".to_owned(),
            text: Arc::from(text),
            text_fingerprint: format!("fingerprint-{id}"),
            heading_level: None,
            merged_from: Vec::new(),
            footnote_id: None,
            footnote_refs: Vec::new(),
            referenced_by: Vec::new(),
            image: None,
            starts_chapter: true,
            ends_chapter: true,
            source_ref: SourceRef {
                chapter_href: Arc::from(format!("{id}.xhtml")),
                spine_index: chapter_index,
                node_id: 1,
                cfi: None,
            },
        }
    }
}
