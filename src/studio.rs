//! Book Agent Studio package inspection, versioned corrections, and evaluation.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
};

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    book_analysis::{self, Claim, Concept, EvalReport, GroundingBlock, SynthesisDocuments},
    chapter_analysis::{
        self, ChapterAnalysis, CommandAnalyzer, GeneratedChapterAnalysis, PromptBlock,
    },
};

const STUDIO_VERSION: &str = "0.1";

#[derive(Debug, Clone)]
struct StudioPackage {
    source_hash: String,
    title: String,
    structure: Value,
    book_ir: Value,
    eval_report: EvalReport,
    analyses: BTreeMap<String, ChapterAnalysis>,
    documents: SynthesisDocuments,
    blocks: BTreeMap<String, GroundingBlock>,
}

impl StudioPackage {
    fn load(package_dir: &Path) -> Result<Self, String> {
        let manifest: Value = read_json(package_dir.join("manifest.json"))?;
        let structure = read_json(package_dir.join("structure.json"))?;
        let book_ir: Value = read_json(package_dir.join("book_ir.json"))?;
        let eval_report = read_json(package_dir.join("eval_report.json"))?;
        let source_hash = manifest
            .get("source_hash")
            .and_then(Value::as_str)
            .ok_or_else(|| "manifest.json: source_hash must be a string".to_owned())?
            .to_owned();
        let title = book_ir
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("Untitled Book")
            .to_owned();
        let documents = book_analysis::read_synthesis_documents(package_dir)?;
        let mut analyses = BTreeMap::new();
        let chapters_dir = package_dir.join("chapters");
        if chapters_dir.is_dir() {
            let mut paths = fs::read_dir(&chapters_dir)
                .map_err(|error| format!("chapters: {error}"))?
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.ends_with(".analysis.json"))
                })
                .collect::<Vec<_>>();
            paths.sort();
            for path in paths {
                let analysis: ChapterAnalysis = read_json(path)?;
                analyses.insert(analysis.chapter_id.clone(), analysis);
            }
        }
        let blocks = book_ir
            .get("blocks")
            .and_then(Value::as_array)
            .ok_or_else(|| "book_ir.json: blocks must be an array".to_owned())?
            .iter()
            .map(|block| {
                let block_id = required_str(block, "block_id")?.to_owned();
                Ok((
                    block_id,
                    GroundingBlock {
                        chapter_index: required_u32(block, "chapter_index")?,
                        text: required_str(block, "text")?.to_owned(),
                        text_fingerprint: required_str(block, "text_fingerprint")?.to_owned(),
                    },
                ))
            })
            .collect::<Result<BTreeMap<_, _>, String>>()?;
        Ok(Self {
            source_hash,
            title,
            structure,
            book_ir,
            eval_report,
            analyses,
            documents,
            blocks,
        })
    }

    fn chapter_title(&self, chapter_id: &str) -> Option<String> {
        let index = chapter_number(chapter_id)?.checked_sub(1)? as usize;
        self.book_ir
            .get("chapters")?
            .as_array()?
            .get(index)?
            .get("title")?
            .as_str()
            .map(str::to_owned)
    }

    fn chapter_blocks(&self, chapter_id: &str) -> Vec<PromptBlock> {
        let Some(index) = chapter_number(chapter_id).and_then(|value| value.checked_sub(1)) else {
            return Vec::new();
        };
        self.book_ir
            .get("blocks")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|block| {
                block.get("chapter_index").and_then(Value::as_u64) == Some(index.into())
            })
            .filter_map(|block| {
                Some(PromptBlock {
                    block_id: block.get("block_id")?.as_str()?.to_owned(),
                    kind: block.get("kind")?.as_str()?.to_owned(),
                    text: block.get("text")?.as_str()?.to_owned(),
                    text_fingerprint: block.get("text_fingerprint")?.as_str()?.to_owned(),
                })
            })
            .collect()
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
struct VersionedDocument {
    version_id: String,
    revision: u64,
    analyzer: String,
    value: Value,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
struct GoldenBook {
    golden_id: String,
    label: String,
    source_hash: String,
    annotations: Value,
    snapshot: Value,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
struct EvalMetrics {
    parse_quality: u16,
    source_ref_validity: u16,
    chapter_coverage: u16,
    claim_grounding: u16,
    concept_grounding: u16,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
struct EvalRun {
    run_id: String,
    revision: u64,
    valid: bool,
    metrics: EvalMetrics,
    findings: Vec<String>,
    regressions: Vec<String>,
    grounding_report: EvalReport,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
struct StudioState {
    schema_version: String,
    next_id: u64,
    revision: u64,
    chapter_versions: BTreeMap<String, Vec<VersionedDocument>>,
    concept_versions: BTreeMap<String, Vec<VersionedDocument>>,
    claim_versions: BTreeMap<String, Vec<VersionedDocument>>,
    golden_books: BTreeMap<String, GoldenBook>,
    eval_runs: Vec<EvalRun>,
}

impl Default for StudioState {
    fn default() -> Self {
        Self {
            schema_version: STUDIO_VERSION.to_owned(),
            next_id: 1,
            revision: 0,
            chapter_versions: BTreeMap::new(),
            concept_versions: BTreeMap::new(),
            claim_versions: BTreeMap::new(),
            golden_books: BTreeMap::new(),
            eval_runs: Vec::new(),
        }
    }
}

impl StudioState {
    fn id(&mut self, prefix: &str) -> String {
        let id = format!("{prefix}-{:08}", self.next_id);
        self.next_id = self.next_id.saturating_add(1);
        id
    }

    fn version(&mut self, analyzer: String, value: Value) -> VersionedDocument {
        self.revision = self.revision.saturating_add(1);
        VersionedDocument {
            version_id: self.id("version"),
            revision: self.revision,
            analyzer,
            value,
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
struct StudioSnapshot {
    schema_version: &'static str,
    source_hash: String,
    title: String,
    revision: u64,
    structure: Value,
    chapters: BTreeMap<String, ChapterAnalysis>,
    concepts: Vec<Concept>,
    claims: Vec<Claim>,
    entities: Value,
    source_ref_validation: Value,
    eval_report: EvalReport,
    package_eval_report: EvalReport,
    version_counts: Value,
    golden_books: Vec<GoldenBook>,
    eval_runs: Vec<EvalRun>,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize)]
struct ReanalyzeRequest {
    #[serde(default = "default_analyzer_label")]
    analyzer_label: String,
}

fn default_analyzer_label() -> String {
    "command".to_owned()
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
struct ReanalysisAgentRequest {
    protocol_version: &'static str,
    task: &'static str,
    analyzer_label: String,
    book_title: String,
    chapter_id: String,
    chapter_title: String,
    blocks: Vec<PromptBlock>,
    prior_summaries: Vec<Value>,
    existing_analysis: ChapterAnalysis,
    output_schema: Value,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize)]
struct CompareRequest {
    kind: DocumentKind,
    id: String,
    left_version: String,
    right_version: String,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DocumentKind {
    Chapter,
    Concept,
    Claim,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
struct DiffEntry {
    path: String,
    left: Value,
    right: Value,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize)]
struct GoldenCreateRequest {
    label: String,
    #[serde(default)]
    annotations: Value,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize)]
struct GoldenImportRequest {
    label: String,
    source_hash: String,
    #[serde(default)]
    annotations: Value,
    snapshot: Value,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize)]
struct AnnotationRequest {
    annotations: Value,
}

pub(crate) struct StudioRuntime {
    package: StudioPackage,
    state_path: PathBuf,
    state: Mutex<StudioState>,
    agent: Option<CommandAnalyzer>,
}

impl StudioRuntime {
    pub(crate) fn load(
        package_dir: &Path,
        state_dir: &Path,
        agent: Option<CommandAnalyzer>,
    ) -> Result<Self, String> {
        let package = StudioPackage::load(package_dir)?;
        let state_path = state_dir.join("studio_state.json");
        let state = if state_path.is_file() {
            let state: StudioState = read_json(&state_path)?;
            if state.schema_version != STUDIO_VERSION {
                return Err("studio_state.json: unsupported schema_version".to_owned());
            }
            state
        } else {
            StudioState::default()
        };
        Ok(Self {
            package,
            state_path,
            state: Mutex::new(state),
            agent,
        })
    }

    fn lock(&self) -> Result<MutexGuard<'_, StudioState>, StudioError> {
        self.state
            .lock()
            .map_err(|_| StudioError::internal("studio state lock is poisoned"))
    }

    fn save(&self, state: &StudioState) -> Result<(), StudioError> {
        let temporary = self.state_path.with_extension("json.tmp");
        let mut bytes = serde_json::to_vec_pretty(state).map_err(|error| {
            StudioError::internal(format!("cannot serialize Studio state: {error}"))
        })?;
        bytes.push(b'\n');
        fs::write(&temporary, bytes)
            .and_then(|()| fs::rename(&temporary, &self.state_path))
            .map_err(|error| StudioError::internal(format!("cannot persist Studio state: {error}")))
    }
}

fn read_json<T: DeserializeOwned>(path: impl AsRef<Path>) -> Result<T, String> {
    let path = path.as_ref();
    let bytes = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|error| format!("{}: {error}", path.display()))
}

fn required_str<'a>(value: &'a Value, field: &str) -> Result<&'a str, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{field} must be a string"))
}

fn required_u32(value: &Value, field: &str) -> Result<u32, String> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| format!("{field} must be a non-negative integer"))
}

fn chapter_number(chapter_id: &str) -> Option<u32> {
    chapter_id.strip_prefix("chapter_")?.parse().ok()
}

impl StudioRuntime {
    fn effective_analyses(&self, state: &StudioState) -> BTreeMap<String, ChapterAnalysis> {
        let mut analyses = self.package.analyses.clone();
        for (chapter_id, versions) in &state.chapter_versions {
            if let Some(value) = versions.last().map(|version| &version.value) {
                if let Ok(analysis) = serde_json::from_value(value.clone()) {
                    analyses.insert(chapter_id.clone(), analysis);
                }
            }
        }
        analyses
    }

    fn effective_concepts(&self, state: &StudioState) -> Vec<Concept> {
        self.package
            .documents
            .concepts
            .concepts
            .iter()
            .map(|concept| {
                state
                    .concept_versions
                    .get(&concept.concept_id)
                    .and_then(|versions| versions.last())
                    .and_then(|version| serde_json::from_value(version.value.clone()).ok())
                    .unwrap_or_else(|| concept.clone())
            })
            .collect()
    }

    fn effective_claims(&self, state: &StudioState) -> Vec<Claim> {
        self.package
            .documents
            .claims
            .claims
            .iter()
            .map(|claim| {
                state
                    .claim_versions
                    .get(&claim.claim_id)
                    .and_then(|versions| versions.last())
                    .and_then(|version| serde_json::from_value(version.value.clone()).ok())
                    .unwrap_or_else(|| claim.clone())
            })
            .collect()
    }

    fn effective_documents(&self, state: &StudioState) -> SynthesisDocuments {
        let mut documents = self.package.documents.clone();
        documents.concepts.concepts = self.effective_concepts(state);
        documents.claims.claims = self.effective_claims(state);
        documents
    }

    fn effective_grounding(&self, state: &StudioState) -> Result<EvalReport, StudioError> {
        let analyses = self
            .effective_analyses(state)
            .into_values()
            .collect::<Vec<_>>();
        let documents = self.effective_documents(state);
        book_analysis::validate_synthesis_documents(&documents, &analyses)
            .map_err(|error| StudioError::bad_request(error.to_string()))?;
        Ok(book_analysis::validate_grounding(
            &self.package.source_hash,
            &self.package.blocks,
            &analyses,
            &documents.generated(),
        ))
    }

    fn snapshot(&self) -> Result<StudioSnapshot, StudioError> {
        let state = self.lock()?;
        let analyses = self.effective_analyses(&state);
        let concepts = self.effective_concepts(&state);
        let claims = self.effective_claims(&state);
        let grounding = self.effective_grounding(&state)?;
        Ok(StudioSnapshot {
            schema_version: STUDIO_VERSION,
            source_hash: self.package.source_hash.clone(),
            title: self.package.title.clone(),
            revision: state.revision,
            structure: self.package.structure.clone(),
            chapters: analyses,
            concepts,
            claims,
            entities: serde_json::to_value(&self.package.documents.entities)
                .map_err(|error| StudioError::internal(error.to_string()))?,
            source_ref_validation: json!({
                "valid": grounding.valid,
                "stats": grounding.stats,
                "issues": grounding.issues,
            }),
            eval_report: grounding,
            package_eval_report: self.package.eval_report.clone(),
            version_counts: json!({
                "chapters": state.chapter_versions.values().map(Vec::len).sum::<usize>(),
                "concepts": state.concept_versions.values().map(Vec::len).sum::<usize>(),
                "claims": state.claim_versions.values().map(Vec::len).sum::<usize>(),
            }),
            golden_books: state.golden_books.values().cloned().collect(),
            eval_runs: state.eval_runs.clone(),
        })
    }

    fn source(&self, block_id: &str) -> Result<Value, StudioError> {
        let block = self
            .package
            .book_ir
            .get("blocks")
            .and_then(Value::as_array)
            .and_then(|blocks| {
                blocks
                    .iter()
                    .find(|block| block.get("block_id").and_then(Value::as_str) == Some(block_id))
            })
            .ok_or_else(|| StudioError::not_found("source block not found"))?;
        Ok(block.clone())
    }

    fn update_chapter(
        &self,
        chapter_id: &str,
        value: Value,
        analyzer: String,
    ) -> Result<VersionedDocument, StudioError> {
        let analysis: ChapterAnalysis = serde_json::from_value(value.clone()).map_err(|error| {
            StudioError::bad_request(format!("invalid chapter analysis: {error}"))
        })?;
        if analysis.chapter_id != chapter_id || !self.package.analyses.contains_key(chapter_id) {
            return Err(StudioError::bad_request(
                "chapter analysis does not match route chapter",
            ));
        }
        chapter_analysis::validate_generated_analysis(&analysis.generated)
            .map_err(|error| StudioError::bad_request(error.to_string()))?;
        let mut state = self.lock()?;
        let mut candidate = state.clone();
        let version = candidate.version(analyzer, value);
        candidate
            .chapter_versions
            .entry(chapter_id.to_owned())
            .or_default()
            .push(version.clone());
        let report = self.effective_grounding(&candidate)?;
        if !report.valid {
            return Err(StudioError::bad_request(
                "chapter correction violates grounding invariants",
            ));
        }
        *state = candidate;
        self.save(&state)?;
        Ok(version)
    }

    fn update_concept(
        &self,
        concept_id: &str,
        value: Value,
    ) -> Result<VersionedDocument, StudioError> {
        let concept: Concept = serde_json::from_value(value.clone())
            .map_err(|error| StudioError::bad_request(format!("invalid concept: {error}")))?;
        if concept.concept_id != concept_id
            || !self
                .package
                .documents
                .concepts
                .concepts
                .iter()
                .any(|existing| existing.concept_id == concept_id)
        {
            return Err(StudioError::bad_request(
                "concept does not match route concept",
            ));
        }
        self.store_synthesis_version(DocumentKind::Concept, concept_id, value)
    }

    fn update_claim(&self, claim_id: &str, value: Value) -> Result<VersionedDocument, StudioError> {
        let claim: Claim = serde_json::from_value(value.clone())
            .map_err(|error| StudioError::bad_request(format!("invalid claim: {error}")))?;
        if claim.claim_id != claim_id
            || !self
                .package
                .documents
                .claims
                .claims
                .iter()
                .any(|existing| existing.claim_id == claim_id)
        {
            return Err(StudioError::bad_request("claim does not match route claim"));
        }
        self.store_synthesis_version(DocumentKind::Claim, claim_id, value)
    }

    fn store_synthesis_version(
        &self,
        kind: DocumentKind,
        id: &str,
        value: Value,
    ) -> Result<VersionedDocument, StudioError> {
        let mut state = self.lock()?;
        let mut candidate = state.clone();
        let version = candidate.version("manual".to_owned(), value);
        match kind {
            DocumentKind::Concept => candidate
                .concept_versions
                .entry(id.to_owned())
                .or_default()
                .push(version.clone()),
            DocumentKind::Claim => candidate
                .claim_versions
                .entry(id.to_owned())
                .or_default()
                .push(version.clone()),
            DocumentKind::Chapter => unreachable!("chapter versions use update_chapter"),
        }
        let report = self.effective_grounding(&candidate)?;
        if !report.valid {
            return Err(StudioError::bad_request(
                "correction violates package grounding invariants",
            ));
        }
        *state = candidate;
        self.save(&state)?;
        Ok(version)
    }

    fn reanalyze(
        &self,
        chapter_id: &str,
        request: ReanalyzeRequest,
    ) -> Result<VersionedDocument, StudioError> {
        let agent = self
            .agent
            .as_ref()
            .ok_or_else(|| StudioError::conflict("reanalyze requires --agent-command"))?;
        let state = self.lock()?;
        let analyses = self.effective_analyses(&state);
        let existing = analyses
            .get(chapter_id)
            .cloned()
            .ok_or_else(|| StudioError::not_found("chapter analysis not found"))?;
        let prior_summaries = analyses
            .values()
            .filter(|analysis| analysis.spine_index < existing.spine_index)
            .map(|analysis| {
                json!({
                    "chapter_id": analysis.chapter_id,
                    "summary": analysis.generated.summary,
                })
            })
            .collect();
        drop(state);
        let agent_request = ReanalysisAgentRequest {
            protocol_version: STUDIO_VERSION,
            task: "chapter_reanalysis",
            analyzer_label: request.analyzer_label.clone(),
            book_title: self.package.title.clone(),
            chapter_id: chapter_id.to_owned(),
            chapter_title: self
                .package
                .chapter_title(chapter_id)
                .unwrap_or_else(|| existing.chapter_title.clone()),
            blocks: self.package.chapter_blocks(chapter_id),
            prior_summaries,
            existing_analysis: existing.clone(),
            output_schema: chapter_reanalysis_schema(),
        };
        let generated: GeneratedChapterAnalysis = agent
            .execute(&agent_request)
            .map_err(|error| StudioError::bad_gateway(error.to_string()))?;
        chapter_analysis::validate_generated_analysis(&generated)
            .map_err(|error| StudioError::bad_gateway(error.to_string()))?;
        let mut updated = existing;
        updated.generated = generated;
        let value = serde_json::to_value(updated)
            .map_err(|error| StudioError::internal(error.to_string()))?;
        self.update_chapter(chapter_id, value, request.analyzer_label)
    }

    fn versions(
        &self,
        kind: DocumentKind,
        id: &str,
    ) -> Result<Vec<VersionedDocument>, StudioError> {
        let state = self.lock()?;
        let base = self.base_value(kind, id)?;
        let mut versions = vec![VersionedDocument {
            version_id: "base".to_owned(),
            revision: 0,
            analyzer: "package".to_owned(),
            value: base,
        }];
        let stored = match kind {
            DocumentKind::Chapter => state.chapter_versions.get(id),
            DocumentKind::Concept => state.concept_versions.get(id),
            DocumentKind::Claim => state.claim_versions.get(id),
        };
        versions.extend(stored.into_iter().flatten().cloned());
        Ok(versions)
    }

    fn compare(&self, request: CompareRequest) -> Result<Vec<DiffEntry>, StudioError> {
        let versions = self.versions(request.kind, &request.id)?;
        let left = versions
            .iter()
            .find(|version| version.version_id == request.left_version)
            .ok_or_else(|| StudioError::not_found("left version not found"))?;
        let right = versions
            .iter()
            .find(|version| version.version_id == request.right_version)
            .ok_or_else(|| StudioError::not_found("right version not found"))?;
        let mut diffs = Vec::new();
        collect_diff("$", &left.value, &right.value, &mut diffs);
        diffs.truncate(500);
        Ok(diffs)
    }

    fn base_value(&self, kind: DocumentKind, id: &str) -> Result<Value, StudioError> {
        match kind {
            DocumentKind::Chapter => self
                .package
                .analyses
                .get(id)
                .map(serde_json::to_value)
                .transpose()
                .map_err(|error| StudioError::internal(error.to_string()))?,
            DocumentKind::Concept => self
                .package
                .documents
                .concepts
                .concepts
                .iter()
                .find(|concept| concept.concept_id == id)
                .map(serde_json::to_value)
                .transpose()
                .map_err(|error| StudioError::internal(error.to_string()))?,
            DocumentKind::Claim => self
                .package
                .documents
                .claims
                .claims
                .iter()
                .find(|claim| claim.claim_id == id)
                .map(serde_json::to_value)
                .transpose()
                .map_err(|error| StudioError::internal(error.to_string()))?,
        }
        .ok_or_else(|| StudioError::not_found("document not found"))
    }
}

fn collect_diff(path: &str, left: &Value, right: &Value, diffs: &mut Vec<DiffEntry>) {
    if left == right {
        return;
    }
    match (left, right) {
        (Value::Object(left), Value::Object(right)) => {
            let keys = left
                .keys()
                .chain(right.keys())
                .cloned()
                .collect::<BTreeSet<_>>();
            for key in keys {
                collect_diff(
                    &format!("{path}.{key}"),
                    left.get(&key).unwrap_or(&Value::Null),
                    right.get(&key).unwrap_or(&Value::Null),
                    diffs,
                );
            }
        }
        (Value::Array(left), Value::Array(right)) => {
            for index in 0..left.len().max(right.len()) {
                collect_diff(
                    &format!("{path}[{index}]"),
                    left.get(index).unwrap_or(&Value::Null),
                    right.get(index).unwrap_or(&Value::Null),
                    diffs,
                );
            }
        }
        _ => diffs.push(DiffEntry {
            path: path.to_owned(),
            left: left.clone(),
            right: right.clone(),
        }),
    }
}

fn chapter_reanalysis_schema() -> Value {
    serde_json::from_str(chapter_analysis::OUTPUT_SCHEMA).expect("embedded chapter analysis schema")
}

impl StudioRuntime {
    fn create_golden(&self, request: GoldenCreateRequest) -> Result<GoldenBook, StudioError> {
        if request.label.trim().is_empty() {
            return Err(StudioError::bad_request("golden label must not be empty"));
        }
        let snapshot = serde_json::to_value(self.snapshot()?)
            .map_err(|error| StudioError::internal(error.to_string()))?;
        let mut state = self.lock()?;
        let golden = GoldenBook {
            golden_id: state.id("golden"),
            label: request.label,
            source_hash: self.package.source_hash.clone(),
            annotations: request.annotations,
            snapshot,
        };
        state
            .golden_books
            .insert(golden.golden_id.clone(), golden.clone());
        self.save(&state)?;
        Ok(golden)
    }

    fn import_golden(&self, request: GoldenImportRequest) -> Result<GoldenBook, StudioError> {
        if request.label.trim().is_empty()
            || request.source_hash.trim().is_empty()
            || !request.snapshot.is_object()
        {
            return Err(StudioError::bad_request("invalid Golden Book import"));
        }
        let mut state = self.lock()?;
        let golden = GoldenBook {
            golden_id: state.id("golden"),
            label: request.label,
            source_hash: request.source_hash,
            annotations: request.annotations,
            snapshot: request.snapshot,
        };
        state
            .golden_books
            .insert(golden.golden_id.clone(), golden.clone());
        self.save(&state)?;
        Ok(golden)
    }

    fn update_annotations(
        &self,
        golden_id: &str,
        request: AnnotationRequest,
    ) -> Result<GoldenBook, StudioError> {
        let mut state = self.lock()?;
        let golden = state
            .golden_books
            .get_mut(golden_id)
            .ok_or_else(|| StudioError::not_found("Golden Book not found"))?;
        golden.annotations = request.annotations;
        let result = golden.clone();
        self.save(&state)?;
        Ok(result)
    }

    fn run_eval(&self) -> Result<EvalRun, StudioError> {
        let mut state = self.lock()?;
        let analyses = self.effective_analyses(&state);
        let documents = self.effective_documents(&state);
        let grounding = self.effective_grounding(&state)?;
        let chapter_values = self
            .package
            .book_ir
            .get("chapters")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let block_values = self
            .package
            .book_ir
            .get("blocks")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let readable_chapters = chapter_values
            .iter()
            .enumerate()
            .filter(|(_, chapter)| {
                !chapter
                    .get("is_noise")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            })
            .collect::<Vec<_>>();
        let parsed_chapters = readable_chapters
            .iter()
            .filter(|(index, chapter)| {
                chapter
                    .get("title")
                    .and_then(Value::as_str)
                    .is_some_and(|title| !title.trim().is_empty())
                    && block_values.iter().any(|block| {
                        block.get("chapter_index").and_then(Value::as_u64)
                            == u64::try_from(*index).ok()
                    })
            })
            .count();
        let valid_refs = grounding.stats.source_ref_count.saturating_sub(
            grounding
                .issues
                .iter()
                .filter(|issue| issue.severity == book_analysis::IssueSeverity::Error)
                .count(),
        );
        let grounded_claims = documents
            .claims
            .claims
            .iter()
            .filter(|claim| !claim.supporting_evidence.is_empty())
            .count();
        let grounded_concepts = documents
            .concepts
            .concepts
            .iter()
            .filter(|concept| !concept.appearances.is_empty())
            .count();
        let metrics = EvalMetrics {
            parse_quality: ratio(parsed_chapters, readable_chapters.len()),
            source_ref_validity: ratio(valid_refs, grounding.stats.source_ref_count),
            chapter_coverage: ratio(analyses.len(), readable_chapters.len()),
            claim_grounding: ratio(grounded_claims, documents.claims.claims.len()),
            concept_grounding: ratio(grounded_concepts, documents.concepts.concepts.len()),
        };
        let mut findings = Vec::new();
        for (name, value) in metric_pairs(&metrics) {
            if value < 8_000 {
                findings.push(format!("{name} is below 80%: {value} basis points"));
            }
        }
        findings.extend(
            grounding
                .issues
                .iter()
                .map(|issue| format!("{}: {}", issue.code, issue.message)),
        );
        let regressions = state
            .eval_runs
            .last()
            .map(|previous| metric_regressions(&previous.metrics, &metrics))
            .unwrap_or_default();
        let run = EvalRun {
            run_id: state.id("eval"),
            revision: state.revision,
            valid: grounding.valid,
            metrics,
            findings,
            regressions,
            grounding_report: grounding,
        };
        state.eval_runs.push(run.clone());
        self.save(&state)?;
        Ok(run)
    }

    pub(crate) fn dispatch(
        &self,
        method: &str,
        segments: &[&str],
        body: &[u8],
    ) -> Result<StudioResponse, StudioError> {
        match segments {
            ["snapshot"] if method == "GET" => StudioResponse::json(200, self.snapshot()?),
            ["sources", block_id] if method == "GET" => {
                StudioResponse::json(200, self.source(block_id)?)
            }
            ["chapters", chapter_id] if method == "PUT" => StudioResponse::json(
                200,
                self.update_chapter(chapter_id, json_body(body)?, "manual".to_owned())?,
            ),
            ["chapters", chapter_id, "reanalyze"] if method == "POST" => {
                StudioResponse::json(200, self.reanalyze(chapter_id, json_body(body)?)?)
            }
            ["chapters", chapter_id, "versions"] if method == "GET" => {
                StudioResponse::json(200, self.versions(DocumentKind::Chapter, chapter_id)?)
            }
            ["concepts", concept_id] if method == "PUT" => {
                StudioResponse::json(200, self.update_concept(concept_id, json_body(body)?)?)
            }
            ["concepts", concept_id, "versions"] if method == "GET" => {
                StudioResponse::json(200, self.versions(DocumentKind::Concept, concept_id)?)
            }
            ["claims", claim_id] if method == "PUT" => {
                StudioResponse::json(200, self.update_claim(claim_id, json_body(body)?)?)
            }
            ["claims", claim_id, "versions"] if method == "GET" => {
                StudioResponse::json(200, self.versions(DocumentKind::Claim, claim_id)?)
            }
            ["compare"] if method == "POST" => {
                StudioResponse::json(200, self.compare(json_body(body)?)?)
            }
            ["golden-books"] if method == "GET" => {
                let state = self.lock()?;
                StudioResponse::json(200, state.golden_books.values().collect::<Vec<_>>())
            }
            ["golden-books"] if method == "POST" => {
                StudioResponse::json(201, self.create_golden(json_body(body)?)?)
            }
            ["golden-books", "import"] if method == "POST" => {
                StudioResponse::json(201, self.import_golden(json_body(body)?)?)
            }
            ["golden-books", golden_id, "annotations"] if method == "PUT" => {
                StudioResponse::json(200, self.update_annotations(golden_id, json_body(body)?)?)
            }
            ["evals"] if method == "GET" => {
                let state = self.lock()?;
                StudioResponse::json(200, &state.eval_runs)
            }
            ["evals"] if method == "POST" => StudioResponse::json(201, self.run_eval()?),
            ["evals", run_id] if method == "GET" => {
                let state = self.lock()?;
                let run = state
                    .eval_runs
                    .iter()
                    .find(|run| run.run_id == *run_id)
                    .ok_or_else(|| StudioError::not_found("eval run not found"))?;
                StudioResponse::json(200, run)
            }
            _ => Err(StudioError::not_found("Studio route not found")),
        }
    }
}

fn ratio(numerator: usize, denominator: usize) -> u16 {
    numerator
        .saturating_mul(10_000)
        .checked_div(denominator)
        .unwrap_or(10_000)
        .min(10_000) as u16
}

fn metric_pairs(metrics: &EvalMetrics) -> [(&'static str, u16); 5] {
    [
        ("parse_quality", metrics.parse_quality),
        ("source_ref_validity", metrics.source_ref_validity),
        ("chapter_coverage", metrics.chapter_coverage),
        ("claim_grounding", metrics.claim_grounding),
        ("concept_grounding", metrics.concept_grounding),
    ]
}

fn metric_regressions(previous: &EvalMetrics, current: &EvalMetrics) -> Vec<String> {
    metric_pairs(previous)
        .into_iter()
        .zip(metric_pairs(current))
        .filter(|((_, before), (_, after))| after < before)
        .map(|((name, before), (_, after))| format!("{name} regressed from {before} to {after}"))
        .collect()
}

fn json_body<T: DeserializeOwned>(body: &[u8]) -> Result<T, StudioError> {
    serde_json::from_slice(body)
        .map_err(|error| StudioError::bad_request(format!("invalid JSON body: {error}")))
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct StudioResponse {
    pub(crate) status: u16,
    pub(crate) body: Value,
}

impl StudioResponse {
    fn json(status: u16, value: impl Serialize) -> Result<Self, StudioError> {
        Ok(Self {
            status,
            body: serde_json::to_value(value)
                .map_err(|error| StudioError::internal(error.to_string()))?,
        })
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct StudioError {
    pub(crate) status: u16,
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

impl StudioError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: 400,
            code: "bad_request",
            message: message.into(),
        }
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: 404,
            code: "not_found",
            message: message.into(),
        }
    }

    fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: 409,
            code: "agent_required",
            message: message.into(),
        }
    }

    fn bad_gateway(message: impl Into<String>) -> Self {
        Self {
            status: 502,
            code: "invalid_agent_response",
            message: message.into(),
        }
    }

    fn internal(message: impl Into<String>) -> Self {
        Self {
            status: 500,
            code: "internal_error",
            message: message.into(),
        }
    }
}
