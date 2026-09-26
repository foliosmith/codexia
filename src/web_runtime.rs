//! Local Book Agent runtime and dependency-free Web Reader reference client.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    book_analysis::{self, Checkpoint, Grounding, SynthesisDocuments},
    chapter_analysis::{AnalysisSourceRef, ChapterAnalysis, CommandAnalyzer},
    runtime_api::{
        CreateReaderSessionRequest, ReaderLocation, ReaderSession, ReaderState, SpoilerBoundary,
        SpoilerMode, UpdateReaderSessionRequest,
    },
    studio::StudioRuntime,
};

const WEB_READER_HTML: &str = include_str!("../web/index.html");
const STUDIO_HTML: &str = include_str!("../web/studio.html");
const MAX_REQUEST_BYTES: usize = 8 * 1024 * 1024;
const STATE_VERSION: &str = "0.1";

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
pub struct RuntimeBlock {
    pub block_id: String,
    pub chapter_id: String,
    pub chapter_index: u32,
    pub order: u32,
    pub kind: String,
    pub text: String,
    pub text_fingerprint: String,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
pub struct RuntimeChapter {
    pub chapter_id: String,
    pub spine_index: u32,
    pub title: String,
    pub is_noise: bool,
    pub blocks: Vec<RuntimeBlock>,
}

#[derive(Debug, Clone)]
struct RuntimeBook {
    package_dir: PathBuf,
    book_id: String,
    source_hash: String,
    title: String,
    manifest: Value,
    structure: Value,
    chapters: Vec<RuntimeChapter>,
    analyses: BTreeMap<String, ChapterAnalysis>,
    documents: SynthesisDocuments,
}

impl RuntimeBook {
    fn load(package_dir: &Path) -> Result<Self, String> {
        let manifest: Value = read_json(package_dir.join("manifest.json"))?;
        let structure: Value = read_json(package_dir.join("structure.json"))?;
        let book_ir: Value = read_json(package_dir.join("book_ir.json"))?;
        let source_hash = required_str(&manifest, "source_hash")?.to_owned();
        let book_id = source_hash.chars().take(16).collect::<String>();
        let title = book_ir
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("Untitled Book")
            .to_owned();
        let chapter_values = book_ir
            .get("chapters")
            .and_then(Value::as_array)
            .ok_or_else(|| "book_ir.json: chapters must be an array".to_owned())?;
        let block_values = book_ir
            .get("blocks")
            .and_then(Value::as_array)
            .ok_or_else(|| "book_ir.json: blocks must be an array".to_owned())?;
        let mut blocks_by_chapter = BTreeMap::<u32, Vec<RuntimeBlock>>::new();
        for block in block_values {
            let chapter_index = required_u32(block, "chapter_index")?;
            let block_id = required_str(block, "block_id")?.to_owned();
            blocks_by_chapter
                .entry(chapter_index)
                .or_default()
                .push(RuntimeBlock {
                    block_id,
                    chapter_id: chapter_id(chapter_index),
                    chapter_index,
                    order: required_u32(block, "order")?,
                    kind: required_str(block, "kind")?.to_owned(),
                    text: required_str(block, "text")?.to_owned(),
                    text_fingerprint: required_str(block, "text_fingerprint")?.to_owned(),
                });
        }
        let mut chapters = Vec::with_capacity(chapter_values.len());
        let mut analyses = BTreeMap::new();
        for chapter in chapter_values {
            let spine_index = required_u32(chapter, "spine_index")?;
            let current_chapter_id = chapter_id(spine_index);
            let mut blocks = blocks_by_chapter.remove(&spine_index).unwrap_or_default();
            blocks.sort_by_key(|block| block.order);
            let analysis_path = package_dir
                .join("chapters")
                .join(format!("{current_chapter_id}.analysis.json"));
            if analysis_path.is_file() {
                let analysis = read_json::<ChapterAnalysis>(analysis_path)?;
                analyses.insert(current_chapter_id.clone(), analysis);
            }
            chapters.push(RuntimeChapter {
                chapter_id: current_chapter_id,
                spine_index,
                title: required_str(chapter, "title")?.to_owned(),
                is_noise: chapter
                    .get("is_noise")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                blocks,
            });
        }
        let documents = book_analysis::read_synthesis_documents(package_dir)?;
        for (name, document_hash) in [
            ("book_map.json", &documents.book_map.source_hash),
            ("concepts.json", &documents.concepts.source_hash),
            ("claims.json", &documents.claims.source_hash),
            ("checkpoints.json", &documents.checkpoints.source_hash),
        ] {
            if document_hash != &source_hash {
                return Err(format!("{name}: source_hash does not match manifest"));
            }
        }
        Ok(Self {
            package_dir: package_dir.to_owned(),
            book_id,
            source_hash,
            title,
            manifest,
            structure,
            chapters,
            analyses,
            documents,
        })
    }

    fn chapter(&self, chapter_id: &str) -> Result<&RuntimeChapter, RuntimeError> {
        self.chapters
            .iter()
            .find(|chapter| chapter.chapter_id == chapter_id)
            .ok_or_else(|| RuntimeError::not_found(format!("unknown chapter: {chapter_id}")))
    }

    fn block(&self, block_id: &str) -> Option<&RuntimeBlock> {
        self.chapters
            .iter()
            .flat_map(|chapter| &chapter.blocks)
            .find(|block| block.block_id == block_id)
    }

    fn chapter_ids(&self) -> Vec<String> {
        self.chapters
            .iter()
            .filter(|chapter| !chapter.is_noise)
            .map(|chapter| chapter.chapter_id.clone())
            .collect()
    }

    fn package_entry(&self, name: &str) -> Result<(&'static str, Vec<u8>), RuntimeError> {
        let content_type = match name {
            "manifest.json" | "structure.json" | "book_ir.json" | "book_map.json"
            | "concepts.json" | "claims.json" | "entities.json" | "checkpoints.json"
            | "recall_cards.json" | "eval_report.json" => "application/json",
            "blocks.jsonl" => "application/x-ndjson",
            _ => return Err(RuntimeError::not_found("unknown package entry")),
        };
        let bytes = fs::read(self.package_dir.join(name)).map_err(|error| {
            RuntimeError::internal(format!("cannot read package entry: {error}"))
        })?;
        Ok((content_type, bytes))
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
struct Highlight {
    highlight_id: String,
    session_id: String,
    chapter_id: String,
    block_id: String,
    start_char: usize,
    end_char: usize,
    color: String,
    text: String,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
struct Note {
    note_id: String,
    session_id: String,
    chapter_id: String,
    block_id: Option<String>,
    text: String,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
struct ReflectionRecord {
    reflection_id: String,
    session_id: String,
    chapter_id: String,
    question_id: String,
    answer: String,
    score_basis_points: u16,
    feedback: String,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
struct ExportRecord {
    export_id: String,
    format: ExportFormat,
    scope: ExportScope,
    file_name: String,
    path: String,
    downloadable: bool,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
struct PersistedState {
    schema_version: String,
    next_id: u64,
    sessions: BTreeMap<String, ReaderSession>,
    highlights: Vec<Highlight>,
    notes: Vec<Note>,
    reflections: Vec<ReflectionRecord>,
    exports: BTreeMap<String, ExportRecord>,
}

impl Default for PersistedState {
    fn default() -> Self {
        Self {
            schema_version: STATE_VERSION.to_owned(),
            next_id: 1,
            sessions: BTreeMap::new(),
            highlights: Vec::new(),
            notes: Vec::new(),
            reflections: Vec::new(),
            exports: BTreeMap::new(),
        }
    }
}

impl PersistedState {
    fn id(&mut self, prefix: &str) -> String {
        let id = format!("{prefix}-{:08}", self.next_id);
        self.next_id = self.next_id.saturating_add(1);
        id
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RuntimeCardType {
    BookMap,
    ChapterSummary,
    Explanation,
    Answer,
    Concept,
    Claim,
    Checkpoint,
    Flashcard,
    Question,
    Reflection,
    Export,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SpoilerStatus {
    WithinBoundary,
    FullBookAllowed,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
struct FollowUpAction {
    action: String,
    label: String,
    payload: Value,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
struct RuntimeCard {
    card_type: RuntimeCardType,
    card_id: String,
    title: String,
    content: Value,
    source_refs: Vec<AnalysisSourceRef>,
    confidence_basis_points: u16,
    grounding: Grounding,
    spoiler_status: SpoilerStatus,
    follow_up_actions: Vec<FollowUpAction>,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
struct RuntimeCardResponse {
    cards: Vec<RuntimeCard>,
    spoiler_boundary: SpoilerBoundary,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize)]
struct AgentResponse {
    cards: Vec<RuntimeCard>,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
struct AssembledContext {
    book_title: String,
    selected_block: Option<RuntimeBlock>,
    nearby_blocks: Vec<RuntimeBlock>,
    chapter_analysis: Option<ChapterAnalysis>,
    related_concepts: Vec<Value>,
    argument_flow: Vec<Value>,
    allowed_chapter_ids: Vec<String>,
    excluded_chapter_ids: Vec<String>,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
struct AgentRequest {
    protocol_version: &'static str,
    task: String,
    instruction: &'static str,
    input: Value,
    context: AssembledContext,
    output_schema: Value,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize)]
struct ExplainActionRequest {
    selected_text: String,
    source_ref: AnalysisSourceRef,
    reader_state: ReaderState,
    spoiler_mode: SpoilerMode,
    #[serde(default = "default_explain_intent")]
    intent: String,
}

fn default_explain_intent() -> String {
    "explain".to_owned()
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize)]
struct AskActionRequest {
    question: String,
    reader_state: ReaderState,
    spoiler_mode: SpoilerMode,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize)]
struct ReflectActionRequest {
    checkpoint_id: String,
    question_id: String,
    answer: String,
    reader_state: ReaderState,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize)]
struct HighlightRequest {
    chapter_id: String,
    block_id: String,
    start_char: usize,
    end_char: usize,
    #[serde(default = "default_highlight_color")]
    color: String,
    text: String,
}

fn default_highlight_color() -> String {
    "yellow".to_owned()
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize)]
struct NoteRequest {
    chapter_id: String,
    block_id: Option<String>,
    text: String,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ExportFormat {
    Json,
    Markdown,
    Obsidian,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ExportScope {
    WholeBook,
    Chapters,
    ReadRange,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize)]
struct ExportRequest {
    format: ExportFormat,
    scope: ExportScope,
    #[serde(default)]
    chapter_ids: Vec<String>,
    session_id: Option<String>,
}

pub struct WebRuntime {
    book: RuntimeBook,
    state_dir: PathBuf,
    state: Mutex<PersistedState>,
    agent: Option<CommandAnalyzer>,
    studio: StudioRuntime,
}

impl WebRuntime {
    pub fn load(
        package_dir: impl AsRef<Path>,
        state_dir: impl AsRef<Path>,
        agent_command: Option<PathBuf>,
    ) -> Result<Self, String> {
        let book = RuntimeBook::load(package_dir.as_ref())?;
        let state_dir = state_dir.as_ref().to_owned();
        fs::create_dir_all(&state_dir)
            .map_err(|error| format!("cannot create {}: {error}", state_dir.display()))?;
        let state_path = state_dir.join("reader_state.json");
        let state = if state_path.is_file() {
            let state: PersistedState = read_json(&state_path)?;
            if state.schema_version != STATE_VERSION {
                return Err("reader_state.json: unsupported schema_version".to_owned());
            }
            state
        } else {
            PersistedState::default()
        };
        let agent = agent_command.map(CommandAnalyzer::new);
        let studio = StudioRuntime::load(package_dir.as_ref(), &state_dir, agent.clone())?;
        Ok(Self {
            book,
            state_dir,
            state: Mutex::new(state),
            agent,
            studio,
        })
    }

    fn save_state(&self, state: &PersistedState) -> Result<(), RuntimeError> {
        let path = self.state_dir.join("reader_state.json");
        let temporary = self.state_dir.join("reader_state.json.tmp");
        let mut json = serde_json::to_vec_pretty(state)
            .map_err(|error| RuntimeError::internal(format!("cannot serialize state: {error}")))?;
        json.push(b'\n');
        fs::write(&temporary, json)
            .and_then(|()| fs::rename(&temporary, &path))
            .map_err(|error| RuntimeError::internal(format!("cannot persist state: {error}")))
    }

    fn lock_state(&self) -> Result<std::sync::MutexGuard<'_, PersistedState>, RuntimeError> {
        self.state
            .lock()
            .map_err(|_| RuntimeError::internal("reader state lock is poisoned"))
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

fn chapter_id(spine_index: u32) -> String {
    format!("chapter_{:03}", spine_index.saturating_add(1))
}

impl WebRuntime {
    fn bootstrap(&self) -> Result<Value, RuntimeError> {
        let state = self.lock_state()?;
        let session = state
            .sessions
            .values()
            .find(|session| session.book_id == self.book.book_id)
            .cloned();
        let weak_chapters = state
            .reflections
            .iter()
            .filter(|reflection| reflection.score_basis_points < 7_000)
            .map(|reflection| reflection.chapter_id.as_str())
            .collect::<BTreeSet<_>>();
        let weak_concept_ids = self
            .book
            .documents
            .checkpoints
            .checkpoints
            .iter()
            .filter(|checkpoint| weak_chapters.contains(checkpoint.chapter_id.as_str()))
            .flat_map(|checkpoint| checkpoint.flashcards.iter())
            .flat_map(|flashcard| flashcard.concept_ids.iter().map(String::as_str))
            .collect::<BTreeSet<_>>();
        let unmastered_concepts = self
            .book
            .documents
            .concepts
            .concepts
            .iter()
            .filter(|concept| weak_concept_ids.contains(concept.concept_id.as_str()))
            .map(|concept| json!({"concept_id": concept.concept_id, "name": concept.name}))
            .collect::<Vec<_>>();
        Ok(json!({
            "book": {
                "book_id": self.book.book_id,
                "title": self.book.title,
                "source_hash": self.book.source_hash,
                "status": "ready",
                "profile": self.book.manifest.get("profile"),
                "chapter_count": self.book.chapters.iter().filter(|chapter| !chapter.is_noise).count(),
                "concept_count": self.book.documents.concepts.concepts.len(),
                "claim_count": self.book.documents.claims.claims.len(),
                "checkpoint_count": self.book.documents.checkpoints.checkpoints.len(),
            },
            "chapters": self.book.chapters.iter().filter(|chapter| !chapter.is_noise).map(|chapter| json!({
                "chapter_id": chapter.chapter_id,
                "title": chapter.title,
                "block_count": chapter.blocks.len(),
                "has_analysis": self.book.analyses.contains_key(&chapter.chapter_id),
            })).collect::<Vec<_>>(),
            "book_map": self.book.documents.book_map,
            "concepts": self.book.documents.concepts.concepts,
            "session": session,
            "highlights": state.highlights.iter().filter(|highlight| session.as_ref().is_some_and(|session| highlight.session_id == session.session_id)).collect::<Vec<_>>(),
            "notes": state.notes.iter().filter(|note| session.as_ref().is_some_and(|session| note.session_id == session.session_id)).collect::<Vec<_>>(),
            "reflections": state.reflections.iter().filter(|reflection| session.as_ref().is_some_and(|session| reflection.session_id == session.session_id)).collect::<Vec<_>>(),
            "unmastered_concepts": unmastered_concepts,
        }))
    }

    fn create_session(
        &self,
        request: CreateReaderSessionRequest,
    ) -> Result<ReaderSession, RuntimeError> {
        request
            .validate()
            .map_err(|error| RuntimeError::bad_request(format!("invalid session: {error:?}")))?;
        if request.book_id != self.book.book_id {
            return Err(RuntimeError::not_found("unknown book"));
        }
        self.book.chapter(&request.current_location.chapter_id)?;
        let mut state = self.lock_state()?;
        if let Some(session) = state
            .sessions
            .values()
            .find(|session| session.book_id == request.book_id)
            .cloned()
        {
            return Ok(session);
        }
        let session_id = state.id("session");
        let session = ReaderSession {
            session_id: session_id.clone(),
            book_id: request.book_id,
            current_location: request.current_location.clone(),
            read_until: request.current_location,
            progress_basis_points: 0,
            spoiler_mode: request.spoiler_mode,
            revision: 1,
        };
        state.sessions.insert(session_id, session.clone());
        self.save_state(&state)?;
        Ok(session)
    }

    fn update_session(
        &self,
        session_id: &str,
        request: UpdateReaderSessionRequest,
    ) -> Result<ReaderSession, RuntimeError> {
        request.validate().map_err(|error| {
            RuntimeError::bad_request(format!("invalid session patch: {error:?}"))
        })?;
        if let Some(location) = &request.current_location {
            self.book.chapter(&location.chapter_id)?;
        }
        if let Some(location) = &request.read_until {
            self.book.chapter(&location.chapter_id)?;
        }
        let requested_read_until = request
            .read_until
            .as_ref()
            .map(|location| self.location_key(location))
            .transpose()?;
        let mut state = self.lock_state()?;
        let session = state
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| RuntimeError::not_found("unknown reader session"))?;
        if requested_read_until.is_some_and(|requested| {
            self.location_key(&session.read_until)
                .is_ok_and(|current| requested < current)
        }) {
            return Err(RuntimeError::bad_request(
                "read_until must not move backwards",
            ));
        }
        if let Some(location) = request.current_location {
            session.current_location = location;
        }
        if let Some(location) = request.read_until {
            session.read_until = location;
        }
        if let Some(progress) = request.progress_basis_points {
            session.progress_basis_points = session.progress_basis_points.max(progress);
        }
        if let Some(mode) = request.spoiler_mode {
            session.spoiler_mode = mode;
        }
        session.revision = session.revision.saturating_add(1);
        let result = session.clone();
        self.save_state(&state)?;
        Ok(result)
    }

    fn add_highlight(
        &self,
        session_id: &str,
        request: HighlightRequest,
    ) -> Result<Highlight, RuntimeError> {
        let chapter = self.book.chapter(&request.chapter_id)?;
        let block = chapter
            .blocks
            .iter()
            .find(|block| block.block_id == request.block_id)
            .ok_or_else(|| RuntimeError::bad_request("highlight block is outside chapter"))?;
        let char_count = block.text.chars().count();
        if request.start_char >= request.end_char || request.end_char > char_count {
            return Err(RuntimeError::bad_request(
                "invalid highlight character range",
            ));
        }
        let selected = block
            .text
            .chars()
            .skip(request.start_char)
            .take(request.end_char - request.start_char)
            .collect::<String>();
        if selected != request.text {
            return Err(RuntimeError::bad_request(
                "highlight text does not match source range",
            ));
        }
        let mut state = self.lock_state()?;
        if !state.sessions.contains_key(session_id) {
            return Err(RuntimeError::not_found("unknown reader session"));
        }
        let highlight = Highlight {
            highlight_id: state.id("highlight"),
            session_id: session_id.to_owned(),
            chapter_id: request.chapter_id,
            block_id: request.block_id,
            start_char: request.start_char,
            end_char: request.end_char,
            color: request.color,
            text: request.text,
        };
        state.highlights.push(highlight.clone());
        self.save_state(&state)?;
        Ok(highlight)
    }

    fn add_note(&self, session_id: &str, request: NoteRequest) -> Result<Note, RuntimeError> {
        self.book.chapter(&request.chapter_id)?;
        if request.text.trim().is_empty() {
            return Err(RuntimeError::bad_request("note text must not be empty"));
        }
        if let Some(block_id) = &request.block_id {
            if self.book.block(block_id).is_none() {
                return Err(RuntimeError::bad_request("unknown note block"));
            }
        }
        let mut state = self.lock_state()?;
        if !state.sessions.contains_key(session_id) {
            return Err(RuntimeError::not_found("unknown reader session"));
        }
        let note = Note {
            note_id: state.id("note"),
            session_id: session_id.to_owned(),
            chapter_id: request.chapter_id,
            block_id: request.block_id,
            text: request.text,
        };
        state.notes.push(note.clone());
        self.save_state(&state)?;
        Ok(note)
    }

    fn boundary(
        &self,
        reader_state: &ReaderState,
        mode: SpoilerMode,
    ) -> Result<(SpoilerBoundary, BTreeSet<String>), RuntimeError> {
        reader_state.validate().map_err(|error| {
            RuntimeError::bad_request(format!("invalid reader state: {error:?}"))
        })?;
        if let Some(session_id) = &reader_state.session_id {
            let state = self.lock_state()?;
            let session = state
                .sessions
                .get(session_id)
                .ok_or_else(|| RuntimeError::not_found("unknown reader session"))?;
            if session.current_location != reader_state.current_location
                || session.read_until != reader_state.read_until
                || session.progress_basis_points != reader_state.progress_basis_points
            {
                return Err(RuntimeError::bad_request(
                    "reader_state does not match persisted session",
                ));
            }
        }
        let all_ids = self.book.chapter_ids();
        let limit_id = match mode {
            SpoilerMode::ReadRange => &reader_state.read_until.chapter_id,
            SpoilerMode::CurrentChapter => &reader_state.current_location.chapter_id,
            SpoilerMode::FullBook => all_ids
                .last()
                .ok_or_else(|| RuntimeError::bad_request("book has no chapters"))?,
        };
        let limit = all_ids
            .iter()
            .position(|chapter_id| chapter_id == limit_id)
            .ok_or_else(|| RuntimeError::bad_request("reader boundary chapter does not exist"))?;
        let allowed = all_ids[..=limit].iter().cloned().collect::<BTreeSet<_>>();
        let excluded_chapter_ids = all_ids[limit + 1..].to_vec();
        Ok((
            SpoilerBoundary {
                mode,
                read_until: match mode {
                    SpoilerMode::ReadRange => Some(reader_state.read_until.clone()),
                    SpoilerMode::CurrentChapter => Some(reader_state.current_location.clone()),
                    SpoilerMode::FullBook => None,
                },
                excluded_chapter_ids,
            },
            allowed,
        ))
    }

    fn assemble_context(
        &self,
        source_ref: Option<&AnalysisSourceRef>,
        reader_state: &ReaderState,
        mode: SpoilerMode,
    ) -> Result<(AssembledContext, SpoilerBoundary), RuntimeError> {
        let (boundary, allowed) = self.boundary(reader_state, mode)?;
        let mut selected_block = source_ref
            .map(|reference| self.validate_source_ref(reference, &allowed))
            .transpose()?
            .cloned();
        if mode == SpoilerMode::ReadRange {
            if let Some(reference) = source_ref {
                self.validate_read_limit(reference, &reader_state.read_until)?;
            }
        }
        let current_chapter_id = selected_block
            .as_ref()
            .map_or(&reader_state.current_location.chapter_id, |block| {
                &block.chapter_id
            });
        let chapter = self.book.chapter(current_chapter_id)?;
        let mut nearby_blocks = if let Some(selected) = &selected_block {
            let position = chapter
                .blocks
                .iter()
                .position(|block| block.block_id == selected.block_id)
                .unwrap_or(0);
            let start = position.saturating_sub(1);
            chapter.blocks[start..(position + 2).min(chapter.blocks.len())].to_vec()
        } else {
            chapter.blocks.iter().take(3).cloned().collect()
        };
        if !allowed.contains(current_chapter_id) {
            return Err(RuntimeError::forbidden(
                "current chapter crosses spoiler boundary",
            ));
        }
        let limit = if mode == SpoilerMode::ReadRange {
            reader_state
                .read_until
                .block_id
                .as_ref()
                .map(|id| {
                    self.book
                        .block(id)
                        .filter(|block| block.chapter_id == reader_state.read_until.chapter_id)
                        .ok_or_else(|| RuntimeError::bad_request("read_until block is invalid"))
                })
                .transpose()?
        } else {
            None
        };
        let visible_block = |mut block: RuntimeBlock| {
            if let Some(limit) = limit {
                if block.chapter_id == limit.chapter_id {
                    if block.order > limit.order {
                        return None;
                    }
                    if block.order == limit.order {
                        if let Some(offset) = reader_state.read_until.char_offset {
                            block.text = block.text.chars().take(offset as usize).collect();
                        }
                    }
                }
            }
            (!block.text.is_empty()).then_some(block)
        };
        let chapter_fully_read = chapter.blocks.last().is_none_or(|block| {
            visible_block(block.clone()).is_some_and(|visible| visible.text == block.text)
        });
        nearby_blocks = nearby_blocks
            .into_iter()
            .filter_map(visible_block)
            .collect();
        let chapter_block_ids = chapter
            .blocks
            .iter()
            .map(|block| block.block_id.as_str())
            .collect::<BTreeSet<_>>();
        let related_concepts = self
            .book
            .documents
            .concepts
            .concepts
            .iter()
            .filter(|concept| {
                !concept.appearances.is_empty()
                    && concept.appearances.iter().all(|reference| {
                        self.validate_source_ref(reference, &allowed).is_ok()
                            && (mode != SpoilerMode::ReadRange
                                || self
                                    .validate_read_limit(reference, &reader_state.read_until)
                                    .is_ok())
                    })
            })
            .filter(|concept| {
                concept
                    .appearances
                    .iter()
                    .any(|reference| chapter_block_ids.contains(reference.block_id.as_str()))
            })
            .filter_map(|concept| serde_json::to_value(concept).ok())
            .collect();
        let chapter_analysis = chapter_fully_read
            .then(|| self.book.analyses.get(current_chapter_id).cloned())
            .flatten();
        let argument_flow = chapter_analysis
            .as_ref()
            .map(|analysis| {
                analysis
                    .generated
                    .argument_flow
                    .iter()
                    .filter_map(|step| serde_json::to_value(step).ok())
                    .collect()
            })
            .unwrap_or_default();
        selected_block = selected_block.and_then(visible_block);
        Ok((
            AssembledContext {
                book_title: self.book.title.clone(),
                selected_block,
                nearby_blocks,
                chapter_analysis,
                related_concepts,
                argument_flow,
                allowed_chapter_ids: allowed.iter().cloned().collect(),
                excluded_chapter_ids: boundary.excluded_chapter_ids.clone(),
            },
            boundary,
        ))
    }

    fn validate_source_ref<'a>(
        &'a self,
        reference: &AnalysisSourceRef,
        allowed_chapter_ids: &BTreeSet<String>,
    ) -> Result<&'a RuntimeBlock, RuntimeError> {
        let block = self
            .book
            .block(&reference.block_id)
            .ok_or_else(|| RuntimeError::bad_request("source_ref block does not exist"))?;
        if !allowed_chapter_ids.contains(&block.chapter_id) {
            return Err(RuntimeError::forbidden(
                "source_ref crosses spoiler boundary",
            ));
        }
        let char_count = block.text.chars().count();
        if reference.start_char >= reference.end_char || reference.end_char > char_count {
            return Err(RuntimeError::bad_request("source_ref range is invalid"));
        }
        if reference.text_fingerprint != block.text_fingerprint {
            return Err(RuntimeError::bad_request(
                "source_ref fingerprint does not match block",
            ));
        }
        Ok(block)
    }

    fn validate_read_limit(
        &self,
        reference: &AnalysisSourceRef,
        read_until: &ReaderLocation,
    ) -> Result<(), RuntimeError> {
        let block = self
            .book
            .block(&reference.block_id)
            .ok_or_else(|| RuntimeError::bad_request("source_ref block does not exist"))?;
        if block.chapter_id != read_until.chapter_id {
            return Ok(());
        }
        let Some(limit_block_id) = &read_until.block_id else {
            return Ok(());
        };
        let limit = self
            .book
            .block(limit_block_id)
            .filter(|limit| limit.chapter_id == read_until.chapter_id)
            .ok_or_else(|| RuntimeError::bad_request("read_until block is invalid"))?;
        if block.order > limit.order
            || (block.order == limit.order
                && read_until
                    .char_offset
                    .is_some_and(|offset| reference.end_char > offset as usize))
        {
            return Err(RuntimeError::forbidden(
                "source_ref is beyond the read_until location",
            ));
        }
        Ok(())
    }

    fn location_key(&self, location: &ReaderLocation) -> Result<(u32, u32, u32), RuntimeError> {
        let chapter = self.book.chapter(&location.chapter_id)?;
        let Some(block_id) = &location.block_id else {
            return Ok((chapter.spine_index, 0, location.char_offset.unwrap_or(0)));
        };
        let block = chapter
            .blocks
            .iter()
            .find(|block| &block.block_id == block_id)
            .ok_or_else(|| RuntimeError::bad_request("reader location block is invalid"))?;
        Ok((
            chapter.spine_index,
            block.order,
            location.char_offset.unwrap_or(0),
        ))
    }

    fn explain(&self, request: ExplainActionRequest) -> Result<RuntimeCardResponse, RuntimeError> {
        if request.selected_text.trim().is_empty() {
            return Err(RuntimeError::bad_request("selected_text must not be empty"));
        }
        let (context, boundary) = self.assemble_context(
            Some(&request.source_ref),
            &request.reader_state,
            request.spoiler_mode,
        )?;
        let block = context
            .selected_block
            .as_ref()
            .ok_or_else(|| RuntimeError::bad_request("selected block is unavailable"))?;
        let selected = block
            .text
            .chars()
            .skip(request.source_ref.start_char)
            .take(request.source_ref.end_char - request.source_ref.start_char)
            .collect::<String>();
        if selected != request.selected_text {
            return Err(RuntimeError::bad_request(
                "selected_text does not match source_ref range",
            ));
        }
        let input = json!({
            "intent": request.intent,
            "selected_text": request.selected_text,
            "source_ref": request.source_ref,
            "answer_policy": {"must_cite": true, "use_only_read_range": request.spoiler_mode == SpoilerMode::ReadRange},
        });
        let cards =
            if let Some(cards) = self.run_agent("explain_passage", input, &context, &boundary)? {
                cards
            } else {
                vec![self.fallback_explanation(&request, &context, &boundary)]
            };
        self.validate_cards(&cards, &boundary)?;
        Ok(RuntimeCardResponse {
            cards,
            spoiler_boundary: boundary,
        })
    }

    fn ask(&self, request: AskActionRequest) -> Result<RuntimeCardResponse, RuntimeError> {
        if request.question.trim().is_empty() {
            return Err(RuntimeError::bad_request("question must not be empty"));
        }
        let (context, boundary) =
            self.assemble_context(None, &request.reader_state, request.spoiler_mode)?;
        let input = json!({
            "question": request.question,
            "answer_policy": {"must_cite": true, "use_only_read_range": request.spoiler_mode == SpoilerMode::ReadRange},
        });
        let cards = if let Some(cards) = self.run_agent("ask_book", input, &context, &boundary)? {
            cards
        } else {
            vec![self.fallback_answer(&request.question, &context, &boundary)]
        };
        self.validate_cards(&cards, &boundary)?;
        Ok(RuntimeCardResponse {
            cards,
            spoiler_boundary: boundary,
        })
    }

    fn chapter_analysis_card(&self, chapter_id: &str) -> Result<RuntimeCard, RuntimeError> {
        let analysis = self
            .book
            .analyses
            .get(chapter_id)
            .ok_or_else(|| RuntimeError::not_found("chapter analysis is unavailable"))?;
        let source_refs = chapter_analysis_refs(analysis);
        Ok(RuntimeCard {
            card_type: RuntimeCardType::ChapterSummary,
            card_id: format!("summary-{chapter_id}"),
            title: analysis.chapter_title.clone(),
            content: serde_json::to_value(analysis)
                .map_err(|error| RuntimeError::internal(error.to_string()))?,
            confidence_basis_points: if source_refs.is_empty() { 6_000 } else { 9_000 },
            grounding: if source_refs.is_empty() {
                Grounding::Inferred
            } else {
                Grounding::Grounded
            },
            spoiler_status: SpoilerStatus::WithinBoundary,
            follow_up_actions: vec![FollowUpAction {
                action: "quiz".to_owned(),
                label: "考考我".to_owned(),
                payload: json!({"chapter_id": chapter_id}),
            }],
            source_refs,
        })
    }

    fn checkpoint_response(
        &self,
        chapter_id: &str,
        reader_state: &ReaderState,
        mode: SpoilerMode,
    ) -> Result<RuntimeCardResponse, RuntimeError> {
        let (boundary, allowed) = self.boundary(reader_state, mode)?;
        if !allowed.contains(chapter_id) {
            return Err(RuntimeError::forbidden(
                "checkpoint crosses spoiler boundary",
            ));
        }
        if mode == SpoilerMode::ReadRange {
            if let Some(last) = self.book.chapter(chapter_id)?.blocks.last() {
                self.validate_read_limit(
                    &AnalysisSourceRef {
                        block_id: last.block_id.clone(),
                        start_char: 0,
                        end_char: last.text.chars().count(),
                        text_fingerprint: last.text_fingerprint.clone(),
                    },
                    &reader_state.read_until,
                )?;
            }
        }
        let checkpoint = self
            .book
            .documents
            .checkpoints
            .checkpoints
            .iter()
            .find(|checkpoint| checkpoint.chapter_id == chapter_id)
            .ok_or_else(|| RuntimeError::not_found("checkpoint is unavailable"))?;
        let card = RuntimeCard {
            card_type: RuntimeCardType::Checkpoint,
            card_id: format!("checkpoint-card-{}", checkpoint.checkpoint_id),
            title: "章节复盘".to_owned(),
            content: serde_json::to_value(checkpoint)
                .map_err(|error| RuntimeError::internal(error.to_string()))?,
            source_refs: checkpoint.source_refs.clone(),
            confidence_basis_points: if checkpoint.source_refs.is_empty() {
                6_000
            } else {
                9_000
            },
            grounding: checkpoint.grounding,
            spoiler_status: SpoilerStatus::WithinBoundary,
            follow_up_actions: checkpoint
                .recall_questions
                .iter()
                .map(|question| FollowUpAction {
                    action: "reflect".to_owned(),
                    label: "提交复述".to_owned(),
                    payload: json!({
                        "checkpoint_id": checkpoint.checkpoint_id,
                        "question_id": question.question_id,
                    }),
                })
                .collect(),
        };
        self.validate_cards(std::slice::from_ref(&card), &boundary)?;
        Ok(RuntimeCardResponse {
            cards: vec![card],
            spoiler_boundary: boundary,
        })
    }

    fn reflect(&self, request: ReflectActionRequest) -> Result<RuntimeCardResponse, RuntimeError> {
        if request.answer.trim().is_empty() {
            return Err(RuntimeError::bad_request("answer must not be empty"));
        }
        let checkpoint = self
            .book
            .documents
            .checkpoints
            .checkpoints
            .iter()
            .find(|checkpoint| checkpoint.checkpoint_id == request.checkpoint_id)
            .ok_or_else(|| RuntimeError::not_found("checkpoint is unavailable"))?;
        self.checkpoint_response(
            &checkpoint.chapter_id,
            &request.reader_state,
            SpoilerMode::ReadRange,
        )?;
        let question = checkpoint
            .recall_questions
            .iter()
            .chain(&checkpoint.reflection_questions)
            .find(|question| question.question_id == request.question_id)
            .ok_or_else(|| RuntimeError::not_found("question is unavailable"))?;
        let (context, boundary) = self.assemble_context(
            checkpoint.source_refs.first(),
            &request.reader_state,
            SpoilerMode::ReadRange,
        )?;
        let input = json!({
            "checkpoint_id": request.checkpoint_id,
            "question": question.prompt,
            "expected_points": question.expected_points,
            "answer": request.answer,
        });
        let cards =
            if let Some(cards) = self.run_agent("reflect_on_answer", input, &context, &boundary)? {
                cards
            } else {
                vec![self.fallback_reflection(&request, checkpoint, question)]
            };
        self.validate_cards(&cards, &boundary)?;
        if let Some(session_id) = &request.reader_state.session_id {
            let card = cards
                .first()
                .ok_or_else(|| RuntimeError::internal("reflection returned no cards"))?;
            let mut state = self.lock_state()?;
            if state.sessions.contains_key(session_id) {
                let record = ReflectionRecord {
                    reflection_id: state.id("reflection"),
                    session_id: session_id.clone(),
                    chapter_id: checkpoint.chapter_id.clone(),
                    question_id: request.question_id,
                    answer: request.answer,
                    score_basis_points: card
                        .content
                        .get("score_basis_points")
                        .and_then(Value::as_u64)
                        .and_then(|score| u16::try_from(score).ok())
                        .unwrap_or(card.confidence_basis_points),
                    feedback: card
                        .content
                        .get("feedback")
                        .and_then(Value::as_str)
                        .unwrap_or("已记录")
                        .to_owned(),
                };
                state.reflections.push(record);
                self.save_state(&state)?;
            }
        }
        Ok(RuntimeCardResponse {
            cards,
            spoiler_boundary: boundary,
        })
    }

    fn run_agent(
        &self,
        task: &str,
        input: Value,
        context: &AssembledContext,
        boundary: &SpoilerBoundary,
    ) -> Result<Option<Vec<RuntimeCard>>, RuntimeError> {
        let Some(agent) = &self.agent else {
            return Ok(None);
        };
        let request = AgentRequest {
            protocol_version: STATE_VERSION,
            task: task.to_owned(),
            instruction: "Use only supplied context. Cite source_refs, honor spoiler boundary, and return cards matching output_schema.",
            input,
            context: context.clone(),
            output_schema: runtime_card_schema(),
        };
        let response: AgentResponse = agent
            .execute(&request)
            .map_err(|error| RuntimeError::bad_gateway(error.to_string()))?;
        self.validate_cards(&response.cards, boundary)?;
        Ok(Some(response.cards))
    }

    fn validate_cards(
        &self,
        cards: &[RuntimeCard],
        boundary: &SpoilerBoundary,
    ) -> Result<(), RuntimeError> {
        if cards.is_empty() {
            return Err(RuntimeError::bad_gateway("agent returned no cards"));
        }
        let (_, allowed) = self.boundary(
            &ReaderState {
                session_id: None,
                current_location: boundary
                    .read_until
                    .clone()
                    .unwrap_or_else(|| last_location(&self.book)),
                read_until: boundary
                    .read_until
                    .clone()
                    .unwrap_or_else(|| last_location(&self.book)),
                completed_chapter_ids: Vec::new(),
                progress_basis_points: 0,
            },
            boundary.mode,
        )?;
        for card in cards {
            if card.card_id.trim().is_empty() || card.title.trim().is_empty() {
                return Err(RuntimeError::bad_gateway(
                    "agent card id and title must not be empty",
                ));
            }
            if card.confidence_basis_points > 10_000 {
                return Err(RuntimeError::bad_gateway(
                    "agent confidence_basis_points is out of range",
                ));
            }
            if card.grounding == Grounding::Grounded && card.source_refs.is_empty() {
                return Err(RuntimeError::bad_gateway(
                    "grounded agent card requires source_refs",
                ));
            }
            for reference in &card.source_refs {
                self.validate_source_ref(reference, &allowed)
                    .map_err(|error| RuntimeError::bad_gateway(error.message))?;
                if boundary.mode == SpoilerMode::ReadRange {
                    if let Some(read_until) = &boundary.read_until {
                        self.validate_read_limit(reference, read_until)
                            .map_err(|error| RuntimeError::bad_gateway(error.message))?;
                    }
                }
            }
            let expected_status = if boundary.mode == SpoilerMode::FullBook {
                SpoilerStatus::FullBookAllowed
            } else {
                SpoilerStatus::WithinBoundary
            };
            if card.spoiler_status != expected_status {
                return Err(RuntimeError::bad_gateway(
                    "agent card spoiler_status does not match boundary",
                ));
            }
            if card.follow_up_actions.iter().any(|action| {
                !matches!(
                    action.action.as_str(),
                    "open_source" | "ask" | "explain" | "quiz" | "note" | "export" | "reflect"
                )
            }) {
                return Err(RuntimeError::bad_gateway(
                    "agent card contains an unknown follow-up action",
                ));
            }
        }
        Ok(())
    }

    fn fallback_explanation(
        &self,
        request: &ExplainActionRequest,
        context: &AssembledContext,
        boundary: &SpoilerBoundary,
    ) -> RuntimeCard {
        let analysis = context.chapter_analysis.as_ref();
        let difficult = analysis.and_then(|analysis| {
            analysis
                .generated
                .difficult_passages
                .iter()
                .find(|passage| passage.source_ref.block_id == request.source_ref.block_id)
        });
        let explanation = difficult.map_or_else(
            || {
                analysis.map_or_else(
                    || "该段暂无章节分析。".to_owned(),
                    |analysis| analysis.generated.summary.deep.clone(),
                )
            },
            |passage| passage.explanation.clone(),
        );
        let summary = analysis.map(|analysis| &analysis.generated.summary);
        let content = json!({
            "intent": request.intent,
            "passage": request.selected_text,
            "explanation": explanation,
            "simplified": summary.map(|summary| summary.short.clone()),
            "why_it_matters": summary.map(|summary| summary.role_in_book.clone()),
            "connections_to_prior_text": context.argument_flow,
            "related_concepts": context.related_concepts,
        });
        RuntimeCard {
            card_type: RuntimeCardType::Explanation,
            card_id: format!("explanation-{}", request.source_ref.block_id),
            title: "段落解释".to_owned(),
            content,
            source_refs: vec![request.source_ref.clone()],
            confidence_basis_points: if difficult.is_some() { 9_000 } else { 6_500 },
            grounding: Grounding::Grounded,
            spoiler_status: spoiler_status(boundary),
            follow_up_actions: vec![
                FollowUpAction {
                    action: "open_source".to_owned(),
                    label: "回到原文".to_owned(),
                    payload: json!({"block_id": request.source_ref.block_id}),
                },
                FollowUpAction {
                    action: "quiz".to_owned(),
                    label: "考考我".to_owned(),
                    payload: json!({"chapter_id": context.selected_block.as_ref().map(|block| &block.chapter_id)}),
                },
            ],
        }
    }

    fn fallback_answer(
        &self,
        question: &str,
        context: &AssembledContext,
        boundary: &SpoilerBoundary,
    ) -> RuntimeCard {
        // ponytail: deterministic keyword matching keeps offline mode useful;
        // use --agent-command when semantic retrieval quality matters.
        let normalized = question.to_lowercase();
        let allowed = context.allowed_chapter_ids.iter().cloned().collect();
        let visible_reference = |reference: &AnalysisSourceRef| {
            self.validate_source_ref(reference, &allowed).is_ok()
                && (boundary.mode != SpoilerMode::ReadRange
                    || boundary
                        .read_until
                        .as_ref()
                        .is_none_or(|limit| self.validate_read_limit(reference, limit).is_ok()))
        };
        let concept = self
            .book
            .documents
            .concepts
            .concepts
            .iter()
            .filter(|concept| {
                !concept.appearances.is_empty() && concept.appearances.iter().all(visible_reference)
            })
            .find(|concept| {
                normalized.contains(&concept.name.to_lowercase())
                    || concept
                        .aliases
                        .iter()
                        .any(|alias| normalized.contains(&alias.to_lowercase()))
            });
        let (answer, refs) = if let Some(concept) = concept {
            (
                concept.definition_in_this_book.clone(),
                concept.appearances.clone(),
            )
        } else if let Some(claim) = self.book.documents.claims.claims.iter().find(|claim| {
            !claim.supporting_evidence.is_empty()
                && claim
                    .supporting_evidence
                    .iter()
                    .all(|evidence| visible_reference(&evidence.source_ref))
        }) {
            (
                claim.claim.clone(),
                claim
                    .supporting_evidence
                    .iter()
                    .map(|evidence| evidence.source_ref.clone())
                    .collect(),
            )
        } else {
            (
                context.chapter_analysis.as_ref().map_or_else(
                    || "当前已读范围内没有可用分析。".to_owned(),
                    |analysis| analysis.generated.summary.short.clone(),
                ),
                Vec::new(),
            )
        };
        let grounded = !refs.is_empty();
        RuntimeCard {
            card_type: RuntimeCardType::Answer,
            card_id: "answer-current".to_owned(),
            title: "回答".to_owned(),
            content: json!({"question": question, "answer": answer}),
            source_refs: refs,
            confidence_basis_points: if grounded { 8_000 } else { 5_000 },
            grounding: if grounded {
                Grounding::Grounded
            } else {
                Grounding::Inferred
            },
            spoiler_status: spoiler_status(boundary),
            follow_up_actions: Vec::new(),
        }
    }

    fn fallback_reflection(
        &self,
        request: &ReflectActionRequest,
        checkpoint: &Checkpoint,
        question: &book_analysis::Question,
    ) -> RuntimeCard {
        // ponytail: lexical overlap is the offline floor; delegate to an agent
        // when nuanced comprehension grading is required.
        let answer_terms = terms(&request.answer);
        let expected_terms = question
            .expected_points
            .iter()
            .flat_map(|point| terms(point))
            .collect::<BTreeSet<_>>();
        let matched = answer_terms.intersection(&expected_terms).count();
        let score = matched
            .saturating_mul(10_000)
            .checked_div(expected_terms.len())
            .unwrap_or(0)
            .min(10_000) as u16;
        let feedback = if score >= 7_000 {
            "复述覆盖了主要要点。"
        } else if score >= 3_000 {
            "已抓到部分要点，可以补充遗漏部分。"
        } else {
            "建议重看本章要点后再复述一次。"
        };
        RuntimeCard {
            card_type: RuntimeCardType::Reflection,
            card_id: format!("reflection-{}", question.question_id),
            title: "复述反馈".to_owned(),
            content: json!({
                "score_basis_points": score,
                "feedback": feedback,
                "expected_points": question.expected_points,
            }),
            source_refs: checkpoint.source_refs.clone(),
            confidence_basis_points: 5_000,
            grounding: checkpoint.grounding,
            spoiler_status: SpoilerStatus::WithinBoundary,
            follow_up_actions: vec![FollowUpAction {
                action: "open_source".to_owned(),
                label: "复习原文".to_owned(),
                payload: json!({"chapter_id": checkpoint.chapter_id}),
            }],
        }
    }

    fn export(&self, request: ExportRequest) -> Result<ExportRecord, RuntimeError> {
        let chapter_ids = self.export_chapter_ids(&request)?;
        let export_id = {
            let mut state = self.lock_state()?;
            state.id("export")
        };
        let exports_dir = self.state_dir.join("exports");
        fs::create_dir_all(&exports_dir)
            .map_err(|error| RuntimeError::internal(format!("cannot create exports: {error}")))?;
        let exports_dir = exports_dir.join(&export_id);
        fs::create_dir(&exports_dir)
            .map_err(|error| RuntimeError::internal(format!("cannot create export: {error}")))?;
        let (file_name, path, downloadable) = match request.format {
            ExportFormat::Json => {
                let file_name = format!("{}-package.json", safe_name(&self.book.title));
                let path = exports_dir.join(&file_name);
                let payload = self.json_export(&chapter_ids)?;
                write_json_file(&path, &payload)?;
                (file_name, path, true)
            }
            ExportFormat::Markdown => {
                let file_name = format!("{}-notes.md", safe_name(&self.book.title));
                let path = exports_dir.join(&file_name);
                let markdown = self.markdown_export(&chapter_ids, request.session_id.as_deref())?;
                fs::write(&path, markdown).map_err(|error| {
                    RuntimeError::internal(format!("cannot write export: {error}"))
                })?;
                (file_name, path, true)
            }
            ExportFormat::Obsidian => {
                let file_name = format!("{}-obsidian", safe_name(&self.book.title));
                let path = exports_dir.join(&file_name);
                fs::create_dir_all(&path).map_err(|error| {
                    RuntimeError::internal(format!("cannot create Obsidian export: {error}"))
                })?;
                self.write_obsidian_export(&path, &chapter_ids, request.session_id.as_deref())?;
                (file_name, path, false)
            }
        };
        let record = ExportRecord {
            export_id: export_id.clone(),
            format: request.format,
            scope: request.scope,
            file_name,
            path: path.display().to_string(),
            downloadable,
        };
        let mut state = self.lock_state()?;
        state.exports.insert(export_id, record.clone());
        self.save_state(&state)?;
        Ok(record)
    }

    fn export_chapter_ids(&self, request: &ExportRequest) -> Result<Vec<String>, RuntimeError> {
        let all_ids = self.book.chapter_ids();
        match request.scope {
            ExportScope::WholeBook => Ok(all_ids),
            ExportScope::Chapters => {
                if request.chapter_ids.is_empty() {
                    return Err(RuntimeError::bad_request(
                        "chapter export requires chapter_ids",
                    ));
                }
                for chapter_id in &request.chapter_ids {
                    self.book.chapter(chapter_id)?;
                }
                Ok(request.chapter_ids.clone())
            }
            ExportScope::ReadRange => {
                let session_id = request.session_id.as_deref().ok_or_else(|| {
                    RuntimeError::bad_request("read_range export requires session_id")
                })?;
                let state = self.lock_state()?;
                let session = state
                    .sessions
                    .get(session_id)
                    .ok_or_else(|| RuntimeError::not_found("unknown reader session"))?;
                let position = all_ids
                    .iter()
                    .position(|chapter_id| chapter_id == &session.read_until.chapter_id)
                    .ok_or_else(|| RuntimeError::bad_request("session read range is invalid"))?;
                Ok(all_ids[..=position].to_vec())
            }
        }
    }

    fn json_export(&self, chapter_ids: &[String]) -> Result<Value, RuntimeError> {
        let selected = chapter_ids.iter().cloned().collect::<BTreeSet<_>>();
        let chapters = self
            .book
            .chapters
            .iter()
            .filter(|chapter| selected.contains(&chapter.chapter_id))
            .collect::<Vec<_>>();
        let analyses = self
            .book
            .analyses
            .iter()
            .filter(|(chapter_id, _)| selected.contains(*chapter_id))
            .map(|(chapter_id, analysis)| (chapter_id.clone(), analysis.clone()))
            .collect::<BTreeMap<_, _>>();
        let checkpoints = self
            .book
            .documents
            .checkpoints
            .checkpoints
            .iter()
            .filter(|checkpoint| selected.contains(&checkpoint.chapter_id))
            .collect::<Vec<_>>();
        Ok(json!({
            "format_version": "0.1",
            "book_id": self.book.book_id,
            "manifest": self.book.manifest,
            "structure": self.book.structure,
            "chapters": chapters,
            "chapter_analyses": analyses,
            "book_map": self.book.documents.book_map,
            "concepts": self.book.documents.concepts,
            "claims": self.book.documents.claims,
            "entities": self.book.documents.entities,
            "checkpoints": checkpoints,
        }))
    }

    fn markdown_export(
        &self,
        chapter_ids: &[String],
        session_id: Option<&str>,
    ) -> Result<String, RuntimeError> {
        let notes = self.session_notes(session_id)?;
        let mut output = format!("# {}\n\n", self.book.title);
        output.push_str(&format!(
            "> {}\n\n## 核心主张\n\n{}\n\n",
            self.book.documents.book_map.book_map.central_question,
            self.book.documents.book_map.book_map.thesis,
        ));
        for chapter_id in chapter_ids {
            let chapter = self.book.chapter(chapter_id)?;
            output.push_str(&format!("## {}\n\n", chapter.title));
            if let Some(analysis) = self.book.analyses.get(chapter_id) {
                output.push_str(&analysis.generated.summary.short);
                output.push_str("\n\n");
                for idea in &analysis.generated.key_ideas {
                    output.push_str(&format!("- {idea}\n"));
                }
                output.push('\n');
            }
            for note in notes.iter().filter(|note| &note.chapter_id == chapter_id) {
                output.push_str(&format!("- 笔记：{}\n", note.text));
            }
            output.push('\n');
        }
        Ok(output)
    }

    fn write_obsidian_export(
        &self,
        directory: &Path,
        chapter_ids: &[String],
        session_id: Option<&str>,
    ) -> Result<(), RuntimeError> {
        let notes = self.session_notes(session_id)?;
        let mut index = format!("# {}\n\n## Chapters\n\n", self.book.title);
        for chapter_id in chapter_ids {
            let chapter = self.book.chapter(chapter_id)?;
            let name = format!("{}-{}", chapter_id, safe_name(&chapter.title));
            index.push_str(&format!("- [[{name}]]\n"));
            let mut markdown = format!(
                "---\nbook_id: {}\nchapter_id: {}\n---\n\n# {}\n\n",
                self.book.book_id, chapter_id, chapter.title
            );
            if let Some(analysis) = self.book.analyses.get(chapter_id) {
                markdown.push_str(&analysis.generated.summary.deep);
                markdown.push_str("\n\n## Key ideas\n\n");
                for idea in &analysis.generated.key_ideas {
                    markdown.push_str(&format!("- {idea}\n"));
                }
            }
            let chapter_notes = notes
                .iter()
                .filter(|note| &note.chapter_id == chapter_id)
                .collect::<Vec<_>>();
            if !chapter_notes.is_empty() {
                markdown.push_str("\n## Notes\n\n");
                for note in chapter_notes {
                    markdown.push_str(&format!("- {}\n", note.text));
                }
            }
            fs::write(directory.join(format!("{name}.md")), markdown).map_err(|error| {
                RuntimeError::internal(format!("cannot write Obsidian chapter: {error}"))
            })?;
        }
        fs::write(directory.join("Book.md"), index).map_err(|error| {
            RuntimeError::internal(format!("cannot write Obsidian index: {error}"))
        })
    }

    fn session_notes(&self, session_id: Option<&str>) -> Result<Vec<Note>, RuntimeError> {
        let Some(session_id) = session_id else {
            return Ok(Vec::new());
        };
        let state = self.lock_state()?;
        if !state.sessions.contains_key(session_id) {
            return Err(RuntimeError::not_found("unknown reader session"));
        }
        Ok(state
            .notes
            .iter()
            .filter(|note| note.session_id == session_id)
            .cloned()
            .collect())
    }

    fn export_download(&self, export_id: &str) -> Result<(String, Vec<u8>), RuntimeError> {
        if !valid_id(export_id) {
            return Err(RuntimeError::bad_request("invalid export id"));
        }
        let state = self.lock_state()?;
        let export = state
            .exports
            .get(export_id)
            .ok_or_else(|| RuntimeError::not_found("unknown export"))?;
        if !export.downloadable {
            return Err(RuntimeError::bad_request(
                "folder exports are available at their local path",
            ));
        }
        let bytes = fs::read(&export.path)
            .map_err(|error| RuntimeError::internal(format!("cannot read export: {error}")))?;
        let content_type = match export.format {
            ExportFormat::Json => "application/json",
            ExportFormat::Markdown => "text/markdown; charset=utf-8",
            ExportFormat::Obsidian => "application/octet-stream",
        };
        Ok((content_type.to_owned(), bytes))
    }
}

fn write_json_file(path: &Path, value: &Value) -> Result<(), RuntimeError> {
    let mut bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| RuntimeError::internal(format!("cannot serialize export: {error}")))?;
    bytes.push(b'\n');
    fs::write(path, bytes)
        .map_err(|error| RuntimeError::internal(format!("cannot write export: {error}")))
}

fn safe_name(value: &str) -> String {
    let name = value
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    name.trim_matches('-').chars().take(80).collect()
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize)]
struct CheckpointActionRequest {
    reader_state: ReaderState,
    spoiler_mode: SpoilerMode,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct RuntimeError {
    status: u16,
    code: &'static str,
    message: String,
}

impl RuntimeError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: 400,
            code: "bad_request",
            message: message.into(),
        }
    }

    fn forbidden(message: impl Into<String>) -> Self {
        Self {
            status: 403,
            code: "spoiler_boundary",
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

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct HttpResponse {
    status: u16,
    content_type: String,
    body: Vec<u8>,
}

impl HttpResponse {
    pub(crate) fn json(status: u16, value: impl Serialize) -> Result<Self, RuntimeError> {
        let body = serde_json::to_vec(&value).map_err(|error| {
            RuntimeError::internal(format!("cannot serialize response: {error}"))
        })?;
        Ok(Self {
            status,
            content_type: "application/json; charset=utf-8".to_owned(),
            body,
        })
    }

    fn html() -> Self {
        Self {
            status: 200,
            content_type: "text/html; charset=utf-8".to_owned(),
            body: WEB_READER_HTML.as_bytes().to_vec(),
        }
    }

    fn studio_html() -> Self {
        Self {
            status: 200,
            content_type: "text/html; charset=utf-8".to_owned(),
            body: STUDIO_HTML.as_bytes().to_vec(),
        }
    }

    pub(crate) fn bytes(status: u16, content_type: impl Into<String>, body: Vec<u8>) -> Self {
        Self {
            status,
            content_type: content_type.into(),
            body,
        }
    }

    fn error(error: RuntimeError) -> Self {
        Self::json(
            error.status,
            json!({"error": {"code": error.code, "message": error.message, "retryable": false}}),
        )
        .unwrap_or_else(|_| Self::bytes(500, "text/plain", b"internal error".to_vec()))
    }

    #[must_use]
    pub const fn status(&self) -> u16 {
        self.status
    }

    #[must_use]
    pub fn body(&self) -> &[u8] {
        &self.body
    }

    #[must_use]
    pub fn content_type(&self) -> &str {
        &self.content_type
    }
}

impl WebRuntime {
    #[must_use]
    pub fn dispatch(&self, method: &str, target: &str, body: &[u8]) -> HttpResponse {
        match self.route(method, target, body) {
            Ok(response) => response,
            Err(error) => HttpResponse::error(error),
        }
    }

    fn route(&self, method: &str, target: &str, body: &[u8]) -> Result<HttpResponse, RuntimeError> {
        let (path, query) = target
            .split_once('?')
            .map_or((target, ""), |(path, query)| (path, query));
        if method == "GET" && path == "/studio" {
            return Ok(HttpResponse::studio_html());
        }
        if method == "GET" && matches!(path, "/" | "/book" | "/reader" | "/report") {
            return Ok(HttpResponse::html());
        }
        if method == "GET" && path == "/v1/bootstrap" {
            return HttpResponse::json(200, self.bootstrap()?);
        }
        let segments = path
            .split('/')
            .filter(|segment| !segment.is_empty())
            .collect::<Vec<_>>();
        match segments.as_slice() {
            ["v1", "studio", rest @ ..] => {
                let response =
                    self.studio
                        .dispatch(method, rest, body)
                        .map_err(|error| RuntimeError {
                            status: error.status,
                            code: error.code,
                            message: error.message,
                        })?;
                HttpResponse::json(response.status, response.body)
            }
            ["v1", "books", book_id, rest @ ..] => {
                self.require_book(book_id)?;
                self.route_book(method, rest, query, body)
            }
            ["v1", "reader-sessions"] if method == "POST" => {
                let request = json_body(body)?;
                HttpResponse::json(201, self.create_session(request)?)
            }
            ["v1", "reader-sessions", session_id] if method == "GET" => {
                let state = self.lock_state()?;
                let session = state
                    .sessions
                    .get(*session_id)
                    .ok_or_else(|| RuntimeError::not_found("unknown reader session"))?;
                HttpResponse::json(200, session)
            }
            ["v1", "reader-sessions", session_id] if method == "PATCH" => {
                let request = json_body(body)?;
                HttpResponse::json(200, self.update_session(session_id, request)?)
            }
            ["v1", "reader-sessions", session_id, "highlights"] if method == "POST" => {
                let request = json_body(body)?;
                HttpResponse::json(201, self.add_highlight(session_id, request)?)
            }
            ["v1", "reader-sessions", session_id, "notes"] if method == "POST" => {
                let request = json_body(body)?;
                HttpResponse::json(201, self.add_note(session_id, request)?)
            }
            ["v1", "exports", export_id] if method == "GET" => {
                let (content_type, bytes) = self.export_download(export_id)?;
                Ok(HttpResponse::bytes(200, content_type, bytes))
            }
            _ => Err(RuntimeError::not_found("route not found")),
        }
    }

    fn route_book(
        &self,
        method: &str,
        segments: &[&str],
        _query: &str,
        body: &[u8],
    ) -> Result<HttpResponse, RuntimeError> {
        match segments {
            ["status"] if method == "GET" => HttpResponse::json(
                200,
                json!({
                    "book_id": self.book.book_id,
                    "state": "ready",
                    "current_stage": null,
                    "completed_stages": ["parse", "normalize", "chapter_analysis", "book_synthesis", "grounding_validation"],
                    "progress_basis_points": 10_000,
                }),
            ),
            ["package"] if method == "GET" => {
                let entries = [
                    ("manifest.json", "application/json"),
                    ("structure.json", "application/json"),
                    ("book_ir.json", "application/json"),
                    ("blocks.jsonl", "application/x-ndjson"),
                    ("book_map.json", "application/json"),
                    ("concepts.json", "application/json"),
                    ("claims.json", "application/json"),
                    ("entities.json", "application/json"),
                    ("checkpoints.json", "application/json"),
                    ("recall_cards.json", "application/json"),
                    ("eval_report.json", "application/json"),
                ]
                .into_iter()
                .filter(|(name, _)| self.book.package_dir.join(name).is_file())
                .map(|(name, media_type)| {
                    json!({
                        "name": name,
                        "media_type": media_type,
                        "href": format!("/v1/books/{}/files/{name}", self.book.book_id),
                        "source_hash": self.book.source_hash,
                    })
                })
                .collect::<Vec<_>>();
                HttpResponse::json(
                    200,
                    json!({
                        "book_id": self.book.book_id,
                        "format_version": self.book.manifest.get("format_version"),
                        "entries": entries,
                    }),
                )
            }
            ["files", name] if method == "GET" => {
                let (content_type, bytes) = self.book.package_entry(name)?;
                Ok(HttpResponse::bytes(200, content_type, bytes))
            }
            ["map"] if method == "GET" => HttpResponse::json(200, &self.book.documents.book_map),
            ["concepts"] if method == "GET" => {
                HttpResponse::json(200, &self.book.documents.concepts)
            }
            ["claims"] if method == "GET" => HttpResponse::json(200, &self.book.documents.claims),
            ["chapters", chapter_id, "content"] if method == "GET" => {
                let chapter = self.book.chapter(chapter_id)?;
                HttpResponse::json(200, chapter)
            }
            ["chapters", chapter_id, "analysis"] if method == "GET" => {
                HttpResponse::json(200, self.chapter_analysis_card(chapter_id)?)
            }
            ["chapters", chapter_id, "checkpoint"] if method == "POST" => {
                let request: CheckpointActionRequest = json_body(body)?;
                HttpResponse::json(
                    200,
                    self.checkpoint_response(
                        chapter_id,
                        &request.reader_state,
                        request.spoiler_mode,
                    )?,
                )
            }
            ["chapters", chapter_id, "reflect"] if method == "POST" => {
                let request: ReflectActionRequest = json_body(body)?;
                let checkpoint = self
                    .book
                    .documents
                    .checkpoints
                    .checkpoints
                    .iter()
                    .find(|checkpoint| checkpoint.checkpoint_id == request.checkpoint_id)
                    .ok_or_else(|| RuntimeError::not_found("checkpoint is unavailable"))?;
                if checkpoint.chapter_id != *chapter_id {
                    return Err(RuntimeError::bad_request(
                        "checkpoint does not belong to route chapter",
                    ));
                }
                HttpResponse::json(200, self.reflect(request)?)
            }
            ["explain"] if method == "POST" => {
                HttpResponse::json(200, self.explain(json_body(body)?)?)
            }
            ["ask"] if method == "POST" => HttpResponse::json(200, self.ask(json_body(body)?)?),
            ["exports"] if method == "POST" => {
                let record = self.export(json_body(body)?)?;
                HttpResponse::json(
                    201,
                    json!({
                        "export_id": record.export_id,
                        "format": record.format,
                        "scope": record.scope,
                        "file_name": record.file_name,
                        "href": record.downloadable.then(|| format!("/v1/exports/{}", record.export_id)),
                        "local_path": record.path,
                    }),
                )
            }
            _ => Err(RuntimeError::not_found("book route not found")),
        }
    }

    fn require_book(&self, book_id: &str) -> Result<(), RuntimeError> {
        if book_id == self.book.book_id {
            Ok(())
        } else {
            Err(RuntimeError::not_found("unknown book"))
        }
    }
}

fn json_body<T: DeserializeOwned>(body: &[u8]) -> Result<T, RuntimeError> {
    serde_json::from_slice(body)
        .map_err(|error| RuntimeError::bad_request(format!("invalid JSON body: {error}")))
}

pub fn serve(
    package_dir: impl AsRef<Path>,
    state_dir: impl AsRef<Path>,
    bind: &str,
    agent_command: Option<PathBuf>,
) -> Result<(), String> {
    serve_surface(package_dir, state_dir, bind, agent_command, "/")
}

pub fn serve_studio(
    package_dir: impl AsRef<Path>,
    state_dir: impl AsRef<Path>,
    bind: &str,
    agent_command: Option<PathBuf>,
) -> Result<(), String> {
    serve_surface(package_dir, state_dir, bind, agent_command, "/studio")
}

fn serve_surface(
    package_dir: impl AsRef<Path>,
    state_dir: impl AsRef<Path>,
    bind: &str,
    agent_command: Option<PathBuf>,
    home: &str,
) -> Result<(), String> {
    let runtime = Arc::new(WebRuntime::load(package_dir, state_dir, agent_command)?);
    serve_http(
        bind,
        home,
        Arc::new(move |request| runtime.dispatch(&request.method, &request.target, &request.body)),
    )
}

type HttpHandler = dyn Fn(&HttpRequest) -> HttpResponse + Send + Sync;

pub(crate) fn serve_http(bind: &str, home: &str, handler: Arc<HttpHandler>) -> Result<(), String> {
    let listener =
        TcpListener::bind(bind).map_err(|error| format!("cannot bind {bind}: {error}"))?;
    eprintln!("Codexia: http://{bind}{home}");
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                if let Err(error) = handle_connection(handler.as_ref(), stream) {
                    eprintln!("request error: {error}");
                }
            }
            Err(error) => eprintln!("connection error: {error}"),
        }
    }
    Ok(())
}

fn handle_connection(handler: &HttpHandler, mut stream: TcpStream) -> Result<(), String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .map_err(|error| error.to_string())?;
    let request = read_http_request(&mut stream)?;
    let response = handler(&request);
    write_http_response(&mut stream, response)
}

pub(crate) struct HttpRequest {
    pub(crate) method: String,
    pub(crate) target: String,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) body: Vec<u8>,
}

fn read_http_request(stream: &mut TcpStream) -> Result<HttpRequest, String> {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 4096];
    let header_end = loop {
        let read = stream.read(&mut chunk).map_err(|error| error.to_string())?;
        if read == 0 {
            return Err("connection closed before request headers".to_owned());
        }
        bytes.extend_from_slice(&chunk[..read]);
        if bytes.len() > MAX_REQUEST_BYTES {
            return Err("request is too large".to_owned());
        }
        if let Some(position) = find_bytes(&bytes, b"\r\n\r\n") {
            break position + 4;
        }
    };
    let headers = std::str::from_utf8(&bytes[..header_end])
        .map_err(|_| "request headers are not UTF-8".to_owned())?;
    let mut lines = headers.split("\r\n");
    let request_line = lines
        .next()
        .ok_or_else(|| "missing request line".to_owned())?;
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts
        .next()
        .ok_or_else(|| "missing HTTP method".to_owned())?
        .to_owned();
    let target = request_parts
        .next()
        .ok_or_else(|| "missing request target".to_owned())?
        .to_owned();
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
        .collect::<BTreeMap<_, _>>();
    let content_length = headers
        .get("content-length")
        .map(|value| value.parse::<usize>())
        .transpose()
        .map_err(|_| "invalid Content-Length".to_owned())?
        .unwrap_or(0);
    if content_length > MAX_REQUEST_BYTES {
        return Err("request body is too large".to_owned());
    }
    while bytes.len() - header_end < content_length {
        let read = stream.read(&mut chunk).map_err(|error| error.to_string())?;
        if read == 0 {
            return Err("connection closed before request body".to_owned());
        }
        bytes.extend_from_slice(&chunk[..read]);
    }
    Ok(HttpRequest {
        method,
        target,
        headers,
        body: bytes[header_end..header_end + content_length].to_vec(),
    })
}

fn write_http_response(stream: &mut TcpStream, response: HttpResponse) -> Result<(), String> {
    let reason = match response.status {
        200 => "OK",
        201 => "Created",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        _ => "Response",
    };
    let headers = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\nX-Content-Type-Options: nosniff\r\nContent-Security-Policy: default-src 'self'; style-src 'unsafe-inline'; script-src 'unsafe-inline'\r\n\r\n",
        response.status,
        reason,
        response.content_type,
        response.body.len(),
    );
    stream
        .write_all(headers.as_bytes())
        .and_then(|()| stream.write_all(&response.body))
        .map_err(|error| error.to_string())
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn terms(value: &str) -> BTreeSet<String> {
    value
        .split(|character: char| !character.is_alphanumeric())
        .filter(|term| term.chars().count() > 1)
        .map(str::to_lowercase)
        .collect()
}

fn chapter_analysis_refs(analysis: &ChapterAnalysis) -> Vec<AnalysisSourceRef> {
    let mut refs = analysis
        .generated
        .concepts
        .iter()
        .flat_map(|concept| concept.source_refs.iter().cloned())
        .chain(analysis.generated.claims.iter().flat_map(|claim| {
            claim
                .evidence
                .iter()
                .map(|evidence| evidence.source_ref.clone())
        }))
        .chain(
            analysis
                .generated
                .difficult_passages
                .iter()
                .map(|passage| passage.source_ref.clone()),
        )
        .chain(
            analysis
                .generated
                .entities
                .iter()
                .flat_map(|entity| entity.source_refs.iter().cloned()),
        )
        .collect::<Vec<_>>();
    refs.sort_by(|left, right| left.block_id.cmp(&right.block_id));
    refs.dedup_by(|left, right| left.block_id == right.block_id);
    refs
}

fn spoiler_status(boundary: &SpoilerBoundary) -> SpoilerStatus {
    if boundary.mode == SpoilerMode::FullBook {
        SpoilerStatus::FullBookAllowed
    } else {
        SpoilerStatus::WithinBoundary
    }
}

fn last_location(book: &RuntimeBook) -> ReaderLocation {
    let chapter = book
        .chapters
        .iter()
        .rev()
        .find(|chapter| !chapter.is_noise)
        .expect("runtime requires at least one readable chapter");
    ReaderLocation {
        chapter_id: chapter.chapter_id.clone(),
        block_id: chapter.blocks.last().map(|block| block.block_id.clone()),
        char_offset: chapter
            .blocks
            .last()
            .map(|block| block.text.chars().count().min(u32::MAX as usize) as u32),
        epub_cfi: None,
    }
}

fn runtime_card_schema() -> Value {
    json!({
        "cards": [{
            "card_type": "explanation|answer|reflection|concept|claim|checkpoint|question|flashcard",
            "card_id": "string",
            "title": "string",
            "content": {},
            "source_refs": [{"block_id": "string", "start_char": 0, "end_char": 1, "text_fingerprint": "string"}],
            "confidence_basis_points": 0,
            "grounding": "grounded|inferred",
            "spoiler_status": "within_boundary|full_book_allowed",
            "follow_up_actions": [{"action": "open_source|ask|explain|quiz|note|export|reflect", "label": "string", "payload": {}}]
        }]
    })
}
