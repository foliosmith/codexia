//! Whole-book synthesis, package output, progressive status, and grounding.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{
    chapter_analysis::{
        profile_name, AnalysisError, AnalysisSourceRef, ChapterAnalysis, ClaimEvidence, ClaimType,
        CommandAnalyzer, EntityType, TocContext,
    },
    ir::{BookIr, Profile},
};

pub const BOOK_ANALYSIS_VERSION: &str = "0.1";

const SYSTEM_PROMPT: &str = r#"Synthesize a book only from the supplied chapter analyses and their source references. Merge duplicate concepts, claims, and entities across chapters. Preserve evidence and mark an assertion as inferred when it has no direct source. Claims must have supporting evidence. Checkpoints may cite only their chapter or earlier chapters. Produce deep, fast, and selective reading paths. Do not invent block IDs or wrap the JSON response in Markdown. Return one JSON object matching output_schema."#;

const PROMPT_TASKS: &[SynthesisPromptTask] = &[
    SynthesisPromptTask { id: "central_question", instruction: "Extract the central question the book addresses." },
    SynthesisPromptTask { id: "thesis", instruction: "Extract the book's thesis and core claims." },
    SynthesisPromptTask { id: "chapter_structure", instruction: "Describe every analyzed chapter's role and dependencies." },
    SynthesisPromptTask { id: "reading_paths", instruction: "Generate deep, fast, and selective reading paths." },
    SynthesisPromptTask { id: "difficulty_map", instruction: "Rate analyzed chapters and explain their difficulty." },
    SynthesisPromptTask { id: "key_chapters", instruction: "Mark the key chapters." },
    SynthesisPromptTask { id: "concepts", instruction: "Merge cross-chapter concepts, relations, and importance scores." },
    SynthesisPromptTask { id: "claims", instruction: "Merge, rank, and connect cross-chapter claims with evidence." },
    SynthesisPromptTask { id: "entities", instruction: "Merge entities, relations, appearances, and importance scores." },
    SynthesisPromptTask { id: "checkpoints", instruction: "Create 3-5 must-understand points, recall and reflection questions, and flashcards per analyzed chapter." },
    SynthesisPromptTask { id: "book_reflection", instruction: "Create whole-book reflection questions." },
];

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize)]
pub struct SynthesisPromptTask {
    pub id: &'static str,
    pub instruction: &'static str,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
pub struct BookSynthesisRequest {
    pub protocol_version: &'static str,
    pub task: &'static str,
    pub analysis_profile: &'static str,
    pub profile_instruction: &'static str,
    pub system_prompt: &'static str,
    pub prompt_tasks: &'static [SynthesisPromptTask],
    pub context: BookSynthesisContext,
    pub output_schema: serde_json::Value,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
pub struct BookSynthesisContext {
    pub book_title: Option<String>,
    pub table_of_contents: Vec<TocContext>,
    pub analyzed_through: Option<u32>,
    pub total_chapter_count: usize,
    pub chapter_analyses: Vec<ChapterAnalysis>,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct GeneratedBookSynthesis {
    pub book_map: BookMap,
    pub concepts: Vec<Concept>,
    pub claims: Vec<Claim>,
    pub entities: Vec<Entity>,
    pub checkpoints: Vec<Checkpoint>,
    pub book_reflection_questions: Vec<Question>,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct BookMap {
    pub central_question: String,
    pub thesis: String,
    pub chapter_roles: Vec<ChapterRole>,
    pub reading_paths: Vec<ReadingPath>,
    pub difficulty_map: Vec<ChapterDifficulty>,
    pub key_chapter_ids: Vec<String>,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct ChapterRole {
    pub chapter_id: String,
    pub role: String,
    pub depends_on_chapter_ids: Vec<String>,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReadingPath {
    pub path_id: String,
    pub kind: ReadingPathKind,
    pub title: String,
    pub description: String,
    pub chapter_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadingPathKind {
    Deep,
    Fast,
    Selective,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct ChapterDifficulty {
    pub chapter_id: String,
    pub level: DifficultyLevel,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DifficultyLevel {
    Introductory,
    Intermediate,
    Advanced,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct Concept {
    pub concept_id: String,
    pub name: String,
    pub aliases: Vec<String>,
    pub definition_in_this_book: String,
    pub appearances: Vec<AnalysisSourceRef>,
    pub related_concepts: Vec<ConceptRelation>,
    pub importance: u8,
    pub grounding: Grounding,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct ConceptRelation {
    pub concept_id: String,
    pub relation: ConceptRelationKind,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConceptRelationKind {
    RelatedTo,
    ShapedBy,
    ExtensionOf,
    Contradicts,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct Claim {
    pub claim_id: String,
    pub claim: String,
    #[serde(rename = "type")]
    pub claim_type: ClaimType,
    pub supporting_evidence: Vec<ClaimEvidence>,
    pub assumptions: Vec<String>,
    pub counterpoints: Vec<String>,
    pub depends_on_claim_ids: Vec<String>,
    pub importance: u8,
    pub grounding: Grounding,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct Entity {
    pub entity_id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub entity_type: EntityType,
    pub role_in_book: String,
    pub appearances: Vec<AnalysisSourceRef>,
    pub relations: Vec<EntityRelation>,
    pub importance: u8,
    pub grounding: Grounding,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct EntityRelation {
    pub entity_id: String,
    pub relation: EntityRelationKind,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityRelationKind {
    WorkedAt,
    Opposes,
    Introduced,
    RelatedTo,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Grounding {
    Grounded,
    Inferred,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct Checkpoint {
    pub checkpoint_id: String,
    pub chapter_id: String,
    pub summary: String,
    pub must_understand: Vec<String>,
    pub recall_questions: Vec<Question>,
    pub reflection_questions: Vec<Question>,
    pub flashcards: Vec<Flashcard>,
    pub source_refs: Vec<AnalysisSourceRef>,
    pub grounding: Grounding,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct Question {
    pub question_id: String,
    pub prompt: String,
    pub expected_points: Vec<String>,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct Flashcard {
    pub flashcard_id: String,
    pub front: String,
    pub back: String,
    pub concept_ids: Vec<String>,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct BookMapDocument {
    pub schema_version: String,
    pub source_hash: String,
    #[serde(flatten)]
    pub book_map: BookMap,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct ConceptsDocument {
    pub schema_version: String,
    pub source_hash: String,
    pub concepts: Vec<Concept>,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct ClaimsDocument {
    pub schema_version: String,
    pub source_hash: String,
    pub claims: Vec<Claim>,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct EntitiesDocument {
    pub schema_version: String,
    pub source_hash: String,
    pub entities: Vec<Entity>,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct CheckpointsDocument {
    pub schema_version: String,
    pub source_hash: String,
    pub checkpoints: Vec<Checkpoint>,
    pub book_reflection_questions: Vec<Question>,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct RecallCardsDocument {
    pub schema_version: String,
    pub source_hash: String,
    pub recall_questions: Vec<Question>,
    pub flashcards: Vec<Flashcard>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SynthesisDocuments {
    pub book_map: BookMapDocument,
    pub concepts: ConceptsDocument,
    pub claims: ClaimsDocument,
    pub entities: EntitiesDocument,
    pub checkpoints: CheckpointsDocument,
    pub recall_cards: RecallCardsDocument,
}

impl SynthesisDocuments {
    #[must_use]
    pub fn new(source_hash: &str, generated: GeneratedBookSynthesis) -> Self {
        let recall_questions = generated
            .checkpoints
            .iter()
            .flat_map(|checkpoint| checkpoint.recall_questions.iter().cloned())
            .collect();
        let flashcards = generated
            .checkpoints
            .iter()
            .flat_map(|checkpoint| checkpoint.flashcards.iter().cloned())
            .collect();
        let version = BOOK_ANALYSIS_VERSION.to_owned();
        let source_hash = source_hash.to_owned();
        Self {
            book_map: BookMapDocument {
                schema_version: version.clone(),
                source_hash: source_hash.clone(),
                book_map: generated.book_map,
            },
            concepts: ConceptsDocument {
                schema_version: version.clone(),
                source_hash: source_hash.clone(),
                concepts: generated.concepts,
            },
            claims: ClaimsDocument {
                schema_version: version.clone(),
                source_hash: source_hash.clone(),
                claims: generated.claims,
            },
            entities: EntitiesDocument {
                schema_version: version.clone(),
                source_hash: source_hash.clone(),
                entities: generated.entities,
            },
            checkpoints: CheckpointsDocument {
                schema_version: version.clone(),
                source_hash: source_hash.clone(),
                checkpoints: generated.checkpoints,
                book_reflection_questions: generated.book_reflection_questions,
            },
            recall_cards: RecallCardsDocument {
                schema_version: version,
                source_hash,
                recall_questions,
                flashcards,
            },
        }
    }

    #[must_use]
    pub fn generated(&self) -> GeneratedBookSynthesis {
        GeneratedBookSynthesis {
            book_map: self.book_map.book_map.clone(),
            concepts: self.concepts.concepts.clone(),
            claims: self.claims.claims.clone(),
            entities: self.entities.entities.clone(),
            checkpoints: self.checkpoints.checkpoints.clone(),
            book_reflection_questions: self.checkpoints.book_reflection_questions.clone(),
        }
    }
}

pub fn synthesize_book(
    book: &BookIr,
    analyses: &[ChapterAnalysis],
    analyzer: &CommandAnalyzer,
    profile: Profile,
    analyzed_through: Option<u32>,
) -> Result<GeneratedBookSynthesis, AnalysisError> {
    if analyses.is_empty() {
        return Err(AnalysisError::new(
            "book synthesis requires chapter analyses",
        ));
    }
    let request = BookSynthesisRequest {
        protocol_version: BOOK_ANALYSIS_VERSION,
        task: "book_synthesis",
        analysis_profile: profile_name(profile),
        profile_instruction: crate::chapter_analysis::profile_instruction(profile),
        system_prompt: SYSTEM_PROMPT,
        prompt_tasks: PROMPT_TASKS,
        context: BookSynthesisContext {
            book_title: book.metadata.title.as_deref().map(str::to_owned),
            table_of_contents: book.toc.iter().map(toc_context).collect(),
            analyzed_through,
            total_chapter_count: book.chapters.len(),
            chapter_analyses: analyses.to_vec(),
        },
        output_schema: output_schema(),
    };
    let generated = analyzer.execute(&request)?;
    validate_generated(&generated, analyses)?;
    Ok(generated)
}

pub fn write_synthesis_documents(
    package_dir: &Path,
    documents: &SynthesisDocuments,
) -> Result<Vec<PathBuf>, AnalysisError> {
    let files = [
        (
            "book_map.json",
            serde_json::to_vec_pretty(&documents.book_map),
        ),
        (
            "concepts.json",
            serde_json::to_vec_pretty(&documents.concepts),
        ),
        ("claims.json", serde_json::to_vec_pretty(&documents.claims)),
        (
            "entities.json",
            serde_json::to_vec_pretty(&documents.entities),
        ),
        (
            "checkpoints.json",
            serde_json::to_vec_pretty(&documents.checkpoints),
        ),
        (
            "recall_cards.json",
            serde_json::to_vec_pretty(&documents.recall_cards),
        ),
    ];
    let mut paths = Vec::with_capacity(files.len());
    for (name, json) in files {
        let path = package_dir.join(name);
        let mut json =
            json.map_err(|error| AnalysisError::new(format!("cannot serialize {name}: {error}")))?;
        json.push(b'\n');
        fs::write(&path, json)
            .map_err(|error| AnalysisError::new(format!("cannot write {name}: {error}")))?;
        paths.push(path);
    }
    Ok(paths)
}

pub fn read_synthesis_documents(package_dir: &Path) -> Result<SynthesisDocuments, String> {
    Ok(SynthesisDocuments {
        book_map: read_document(package_dir, "book_map.json")?,
        concepts: read_document(package_dir, "concepts.json")?,
        claims: read_document(package_dir, "claims.json")?,
        entities: read_document(package_dir, "entities.json")?,
        checkpoints: read_document(package_dir, "checkpoints.json")?,
        recall_cards: read_document(package_dir, "recall_cards.json")?,
    })
}

fn read_document<T: for<'de> Deserialize<'de>>(dir: &Path, name: &str) -> Result<T, String> {
    let bytes = fs::read(dir.join(name)).map_err(|error| format!("{name}: {error}"))?;
    serde_json::from_slice(&bytes).map_err(|error| format!("{name}: invalid schema: {error}"))
}

fn validate_generated(
    generated: &GeneratedBookSynthesis,
    analyses: &[ChapterAnalysis],
) -> Result<(), AnalysisError> {
    require_text(
        "book_map.central_question",
        &generated.book_map.central_question,
    )?;
    require_text("book_map.thesis", &generated.book_map.thesis)?;
    let analyzed_ids = analyses
        .iter()
        .map(|analysis| analysis.chapter_id.as_str())
        .collect::<BTreeSet<_>>();
    let role_ids = generated
        .book_map
        .chapter_roles
        .iter()
        .map(|role| role.chapter_id.as_str())
        .collect::<BTreeSet<_>>();
    if role_ids != analyzed_ids || generated.book_map.chapter_roles.len() != analyzed_ids.len() {
        return Err(AnalysisError::new(
            "book_map.chapter_roles must cover every analyzed chapter exactly once",
        ));
    }
    for role in &generated.book_map.chapter_roles {
        require_text("chapter role", &role.role)?;
        require_known_ids(
            "chapter dependency",
            role.depends_on_chapter_ids.iter().map(String::as_str),
            &analyzed_ids,
        )?;
    }
    let path_kinds = generated
        .book_map
        .reading_paths
        .iter()
        .map(|path| path.kind)
        .collect::<BTreeSet<_>>();
    if generated.book_map.reading_paths.len() != 3
        || path_kinds
            != BTreeSet::from([
                ReadingPathKind::Deep,
                ReadingPathKind::Fast,
                ReadingPathKind::Selective,
            ])
    {
        return Err(AnalysisError::new(
            "book_map.reading_paths must contain deep, fast, and selective paths",
        ));
    }
    unique_ids(
        "reading path id",
        generated
            .book_map
            .reading_paths
            .iter()
            .map(|path| path.path_id.as_str()),
    )?;
    for path in &generated.book_map.reading_paths {
        if path.chapter_ids.is_empty() {
            return Err(AnalysisError::new("reading paths must not be empty"));
        }
        require_known_ids(
            "reading path chapter",
            path.chapter_ids.iter().map(String::as_str),
            &analyzed_ids,
        )?;
    }
    let difficulty_ids = generated
        .book_map
        .difficulty_map
        .iter()
        .map(|item| item.chapter_id.as_str())
        .collect::<BTreeSet<_>>();
    if difficulty_ids != analyzed_ids
        || generated.book_map.difficulty_map.len() != analyzed_ids.len()
    {
        return Err(AnalysisError::new(
            "book_map.difficulty_map must cover every analyzed chapter exactly once",
        ));
    }
    if generated.book_map.key_chapter_ids.is_empty() {
        return Err(AnalysisError::new(
            "book_map.key_chapter_ids must not be empty",
        ));
    }
    require_known_ids(
        "key chapter",
        generated
            .book_map
            .key_chapter_ids
            .iter()
            .map(String::as_str),
        &analyzed_ids,
    )?;
    unique_ids(
        "concept_id",
        generated
            .concepts
            .iter()
            .map(|value| value.concept_id.as_str()),
    )?;
    unique_ids(
        "claim_id",
        generated.claims.iter().map(|value| value.claim_id.as_str()),
    )?;
    unique_ids(
        "entity_id",
        generated
            .entities
            .iter()
            .map(|value| value.entity_id.as_str()),
    )?;
    let concept_ids = generated
        .concepts
        .iter()
        .map(|value| value.concept_id.as_str())
        .collect::<BTreeSet<_>>();
    let claim_ids = generated
        .claims
        .iter()
        .map(|value| value.claim_id.as_str())
        .collect::<BTreeSet<_>>();
    let entity_ids = generated
        .entities
        .iter()
        .map(|value| value.entity_id.as_str())
        .collect::<BTreeSet<_>>();
    for concept in &generated.concepts {
        require_score("concept importance", concept.importance)?;
        require_grounding("concept", concept.grounding, &concept.appearances)?;
        require_known_ids(
            "related concept",
            concept
                .related_concepts
                .iter()
                .map(|relation| relation.concept_id.as_str()),
            &concept_ids,
        )?;
    }
    for claim in &generated.claims {
        require_score("claim importance", claim.importance)?;
        if claim.supporting_evidence.is_empty() || claim.grounding != Grounding::Grounded {
            return Err(AnalysisError::new(
                "every synthesized claim must be grounded by supporting_evidence",
            ));
        }
        require_known_ids(
            "claim dependency",
            claim.depends_on_claim_ids.iter().map(String::as_str),
            &claim_ids,
        )?;
    }
    for entity in &generated.entities {
        require_score("entity importance", entity.importance)?;
        require_grounding("entity", entity.grounding, &entity.appearances)?;
        require_known_ids(
            "entity relation",
            entity
                .relations
                .iter()
                .map(|relation| relation.entity_id.as_str()),
            &entity_ids,
        )?;
    }
    let checkpoint_ids = generated
        .checkpoints
        .iter()
        .map(|checkpoint| checkpoint.chapter_id.as_str())
        .collect::<BTreeSet<_>>();
    if checkpoint_ids != analyzed_ids || generated.checkpoints.len() != analyzed_ids.len() {
        return Err(AnalysisError::new(
            "checkpoints must cover every analyzed chapter exactly once",
        ));
    }
    unique_ids(
        "checkpoint_id",
        generated
            .checkpoints
            .iter()
            .map(|checkpoint| checkpoint.checkpoint_id.as_str()),
    )?;
    let mut question_ids = BTreeSet::new();
    let mut flashcard_ids = BTreeSet::new();
    for checkpoint in &generated.checkpoints {
        if !(3..=5).contains(&checkpoint.must_understand.len()) {
            return Err(AnalysisError::new(
                "each checkpoint must contain 3-5 must_understand points",
            ));
        }
        if checkpoint.recall_questions.is_empty()
            || checkpoint.reflection_questions.is_empty()
            || checkpoint.flashcards.is_empty()
        {
            return Err(AnalysisError::new(
                "each checkpoint requires recall questions, reflection questions, and flashcards",
            ));
        }
        for question in checkpoint
            .recall_questions
            .iter()
            .chain(&checkpoint.reflection_questions)
        {
            require_text("question prompt", &question.prompt)?;
            if !question_ids.insert(question.question_id.as_str()) {
                return Err(AnalysisError::new(format!(
                    "duplicate question_id: {}",
                    question.question_id
                )));
            }
        }
        for flashcard in &checkpoint.flashcards {
            require_text("flashcard front", &flashcard.front)?;
            require_text("flashcard back", &flashcard.back)?;
            if !flashcard_ids.insert(flashcard.flashcard_id.as_str()) {
                return Err(AnalysisError::new(format!(
                    "duplicate flashcard_id: {}",
                    flashcard.flashcard_id
                )));
            }
            require_known_ids(
                "flashcard concept",
                flashcard.concept_ids.iter().map(String::as_str),
                &concept_ids,
            )?;
        }
        require_grounding("checkpoint", checkpoint.grounding, &checkpoint.source_refs)?;
    }
    for question in &generated.book_reflection_questions {
        require_text("book reflection prompt", &question.prompt)?;
        if !question_ids.insert(question.question_id.as_str()) {
            return Err(AnalysisError::new(format!(
                "duplicate question_id: {}",
                question.question_id
            )));
        }
    }
    Ok(())
}

pub fn validate_synthesis_documents(
    documents: &SynthesisDocuments,
    analyses: &[ChapterAnalysis],
) -> Result<(), AnalysisError> {
    validate_generated(&documents.generated(), analyses)
}

fn unique_ids<'a>(name: &str, values: impl Iterator<Item = &'a str>) -> Result<(), AnalysisError> {
    let mut ids = BTreeSet::new();
    for value in values {
        require_text(name, value)?;
        if !ids.insert(value) {
            return Err(AnalysisError::new(format!("duplicate {name}: {value}")));
        }
    }
    Ok(())
}

fn require_known_ids<'a>(
    name: &str,
    values: impl Iterator<Item = &'a str>,
    known: &BTreeSet<&str>,
) -> Result<(), AnalysisError> {
    for value in values {
        if !known.contains(value) {
            return Err(AnalysisError::new(format!(
                "{name} references unknown id: {value}"
            )));
        }
    }
    Ok(())
}

fn require_text(name: &str, value: &str) -> Result<(), AnalysisError> {
    if value.trim().is_empty() {
        Err(AnalysisError::new(format!("{name} must not be empty")))
    } else {
        Ok(())
    }
}

fn require_score(name: &str, value: u8) -> Result<(), AnalysisError> {
    if value > 100 {
        Err(AnalysisError::new(format!(
            "{name} must be between 0 and 100"
        )))
    } else {
        Ok(())
    }
}

fn require_grounding(
    name: &str,
    grounding: Grounding,
    source_refs: &[AnalysisSourceRef],
) -> Result<(), AnalysisError> {
    let expected = if source_refs.is_empty() {
        Grounding::Inferred
    } else {
        Grounding::Grounded
    };
    if grounding == expected {
        Ok(())
    } else {
        Err(AnalysisError::new(format!(
            "{name} grounding must match source availability"
        )))
    }
}

fn toc_context(entry: &crate::ir::TocEntry) -> TocContext {
    TocContext {
        label: entry.label.to_string(),
        href: entry.href.to_string(),
        children: entry.children.iter().map(toc_context).collect(),
    }
}

fn output_schema() -> serde_json::Value {
    serde_json::json!({
        "book_map": {
            "central_question": "string", "thesis": "string",
            "chapter_roles": [{"chapter_id": "string", "role": "string", "depends_on_chapter_ids": ["string"]}],
            "reading_paths": [{"path_id": "string", "kind": "deep|fast|selective", "title": "string", "description": "string", "chapter_ids": ["string"]}],
            "difficulty_map": [{"chapter_id": "string", "level": "introductory|intermediate|advanced", "reason": "string"}],
            "key_chapter_ids": ["string"]
        },
        "concepts": [{"concept_id": "string", "name": "string", "aliases": ["string"], "definition_in_this_book": "string", "appearances": [source_ref_schema()], "related_concepts": [{"concept_id": "string", "relation": "related_to|shaped_by|extension_of|contradicts"}], "importance": 0, "grounding": "grounded|inferred"}],
        "claims": [{"claim_id": "string", "claim": "string", "type": "thesis|supporting|counterclaim|inference", "supporting_evidence": [{"text": "string", "source_ref": source_ref_schema()}], "assumptions": ["string"], "counterpoints": ["string"], "depends_on_claim_ids": ["string"], "importance": 0, "grounding": "grounded"}],
        "entities": [{"entity_id": "string", "name": "string", "type": "person|organization|place|event", "role_in_book": "string", "appearances": [source_ref_schema()], "relations": [{"entity_id": "string", "relation": "worked_at|opposes|introduced|related_to"}], "importance": 0, "grounding": "grounded|inferred"}],
        "checkpoints": [{"checkpoint_id": "string", "chapter_id": "string", "summary": "string", "must_understand": ["string"], "recall_questions": [{"question_id": "string", "prompt": "string", "expected_points": ["string"]}], "reflection_questions": [{"question_id": "string", "prompt": "string", "expected_points": ["string"]}], "flashcards": [{"flashcard_id": "string", "front": "string", "back": "string", "concept_ids": ["string"]}], "source_refs": [source_ref_schema()], "grounding": "grounded|inferred"}],
        "book_reflection_questions": [{"question_id": "string", "prompt": "string", "expected_points": ["string"]}]
    })
}

fn source_ref_schema() -> serde_json::Value {
    serde_json::json!({"block_id": "string", "start_char": 0, "end_char": 0, "text_fingerprint": "string"})
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct GroundingBlock {
    pub chapter_index: u32,
    pub text: String,
    pub text_fingerprint: String,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueSeverity {
    Error,
    Warning,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct EvalIssue {
    pub severity: IssueSeverity,
    pub code: String,
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct EvalStats {
    pub analyzed_chapter_count: usize,
    pub source_ref_count: usize,
    pub cited_block_count: usize,
    pub analyzed_block_count: usize,
    pub summary_coverage_basis_points: u16,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct EvalReport {
    pub schema_version: String,
    pub source_hash: String,
    pub valid: bool,
    pub stats: EvalStats,
    pub issues: Vec<EvalIssue>,
}

#[must_use]
pub fn grounding_blocks(book: &BookIr) -> BTreeMap<String, GroundingBlock> {
    book.blocks
        .iter()
        .map(|block| {
            (
                block.block_id.clone(),
                GroundingBlock {
                    chapter_index: block.chapter_index,
                    text: block.text.to_string(),
                    text_fingerprint: block.text_fingerprint.clone(),
                },
            )
        })
        .collect()
}

#[must_use]
pub fn validate_grounding(
    source_hash: &str,
    blocks: &BTreeMap<String, GroundingBlock>,
    analyses: &[ChapterAnalysis],
    synthesis: &GeneratedBookSynthesis,
) -> EvalReport {
    let mut validator = GroundingValidator::new(blocks);
    for analysis in analyses {
        let prefix = format!("chapters/{}.analysis.json", analysis.chapter_id);
        for (index, concept) in analysis.generated.concepts.iter().enumerate() {
            validator.refs(
                &format!("{prefix}.concepts[{index}]"),
                &concept.source_refs,
                Some(analysis.spine_index),
            );
        }
        for (index, claim) in analysis.generated.claims.iter().enumerate() {
            if claim.evidence.is_empty() {
                validator.error(
                    "claim_without_evidence",
                    format!("{prefix}.claims[{index}]"),
                    "claim has no supporting evidence",
                );
            }
            for (evidence_index, evidence) in claim.evidence.iter().enumerate() {
                validator.reference(
                    &format!("{prefix}.claims[{index}].evidence[{evidence_index}]"),
                    &evidence.source_ref,
                    Some(analysis.spine_index),
                );
            }
        }
        for (index, passage) in analysis.generated.difficult_passages.iter().enumerate() {
            validator.reference(
                &format!("{prefix}.difficult_passages[{index}]"),
                &passage.source_ref,
                Some(analysis.spine_index),
            );
        }
        for (index, entity) in analysis.generated.entities.iter().enumerate() {
            validator.refs(
                &format!("{prefix}.entities[{index}]"),
                &entity.source_refs,
                Some(analysis.spine_index),
            );
        }
    }
    for (index, concept) in synthesis.concepts.iter().enumerate() {
        validator.refs(
            &format!("concepts.json.concepts[{index}]"),
            &concept.appearances,
            None,
        );
    }
    for (index, claim) in synthesis.claims.iter().enumerate() {
        if claim.supporting_evidence.is_empty() {
            validator.error(
                "claim_without_evidence",
                format!("claims.json.claims[{index}]"),
                "claim has no supporting evidence",
            );
        }
        for (evidence_index, evidence) in claim.supporting_evidence.iter().enumerate() {
            validator.reference(
                &format!("claims.json.claims[{index}].supporting_evidence[{evidence_index}]"),
                &evidence.source_ref,
                None,
            );
        }
    }
    for (index, entity) in synthesis.entities.iter().enumerate() {
        validator.refs(
            &format!("entities.json.entities[{index}]"),
            &entity.appearances,
            None,
        );
    }
    for (index, checkpoint) in synthesis.checkpoints.iter().enumerate() {
        validator.refs(
            &format!("checkpoints.json.checkpoints[{index}]"),
            &checkpoint.source_refs,
            chapter_number(&checkpoint.chapter_id).map(|value| value.saturating_sub(1)),
        );
    }

    let analyzed_chapters = analyses
        .iter()
        .map(|analysis| analysis.spine_index)
        .collect::<BTreeSet<_>>();
    let analyzed_block_count = blocks
        .values()
        .filter(|block| analyzed_chapters.contains(&block.chapter_index))
        .count();
    // ponytail: cited-block coverage is the deterministic proxy; add semantic
    // key-passage scoring when Golden Book evaluations justify that cost.
    let coverage = validator
        .cited_blocks
        .len()
        .saturating_mul(10_000)
        .checked_div(analyzed_block_count)
        .unwrap_or(0)
        .min(10_000) as u16;
    if coverage < 5_000 {
        validator.warning(
            "low_summary_coverage",
            "chapter_analyses".to_owned(),
            format!("only {coverage} basis points of analyzed blocks are cited"),
        );
    }
    let valid = !validator
        .issues
        .iter()
        .any(|issue| issue.severity == IssueSeverity::Error);
    EvalReport {
        schema_version: BOOK_ANALYSIS_VERSION.to_owned(),
        source_hash: source_hash.to_owned(),
        valid,
        stats: EvalStats {
            analyzed_chapter_count: analyses.len(),
            source_ref_count: validator.source_ref_count,
            cited_block_count: validator.cited_blocks.len(),
            analyzed_block_count,
            summary_coverage_basis_points: coverage,
        },
        issues: validator.issues,
    }
}

pub fn write_eval_report(
    package_dir: &Path,
    report: &EvalReport,
) -> Result<PathBuf, AnalysisError> {
    let path = package_dir.join("eval_report.json");
    let mut json = serde_json::to_vec_pretty(report).map_err(|error| {
        AnalysisError::new(format!("cannot serialize eval_report.json: {error}"))
    })?;
    json.push(b'\n');
    fs::write(&path, json)
        .map_err(|error| AnalysisError::new(format!("cannot write eval_report.json: {error}")))?;
    Ok(path)
}

struct GroundingValidator<'a> {
    blocks: &'a BTreeMap<String, GroundingBlock>,
    cited_blocks: BTreeSet<String>,
    source_ref_count: usize,
    issues: Vec<EvalIssue>,
}

impl<'a> GroundingValidator<'a> {
    fn new(blocks: &'a BTreeMap<String, GroundingBlock>) -> Self {
        Self {
            blocks,
            cited_blocks: BTreeSet::new(),
            source_ref_count: 0,
            issues: Vec::new(),
        }
    }

    fn refs(&mut self, path: &str, refs: &[AnalysisSourceRef], max_chapter: Option<u32>) {
        for (index, reference) in refs.iter().enumerate() {
            self.reference(
                &format!("{path}.source_refs[{index}]"),
                reference,
                max_chapter,
            );
        }
    }

    fn reference(&mut self, path: &str, reference: &AnalysisSourceRef, max_chapter: Option<u32>) {
        self.source_ref_count += 1;
        let Some(block) = self.blocks.get(&reference.block_id) else {
            self.error(
                "missing_source_ref",
                path.to_owned(),
                format!("missing block {}", reference.block_id),
            );
            return;
        };
        self.cited_blocks.insert(reference.block_id.clone());
        let char_count = block.text.chars().count();
        if reference.start_char >= reference.end_char || reference.end_char > char_count {
            self.error(
                "invalid_char_range",
                path.to_owned(),
                format!(
                    "range {}..{} is outside block length {char_count}",
                    reference.start_char, reference.end_char
                ),
            );
        }
        if reference.text_fingerprint != block.text_fingerprint {
            self.error(
                "fingerprint_mismatch",
                path.to_owned(),
                "text_fingerprint does not match source block".to_owned(),
            );
        }
        if max_chapter.is_some_and(|max| block.chapter_index > max) {
            self.error(
                "spoiler_boundary",
                path.to_owned(),
                format!(
                    "block belongs to later chapter {}",
                    block.chapter_index.saturating_add(1)
                ),
            );
        }
    }

    fn error(&mut self, code: &str, path: String, message: impl Into<String>) {
        self.issues.push(EvalIssue {
            severity: IssueSeverity::Error,
            code: code.to_owned(),
            path,
            message: message.into(),
        });
    }

    fn warning(&mut self, code: &str, path: String, message: impl Into<String>) {
        self.issues.push(EvalIssue {
            severity: IssueSeverity::Warning,
            code: code.to_owned(),
            path,
            message: message.into(),
        });
    }
}

fn chapter_number(chapter_id: &str) -> Option<u32> {
    chapter_id.strip_prefix("chapter_")?.parse().ok()
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct CompileStatus {
    pub schema_version: String,
    pub source_hash: String,
    pub profile: String,
    pub ready_stages: Vec<String>,
    pub analyzed_through: Option<u32>,
    pub analyzed_chapter_count: usize,
    pub total_analyzable_chapter_count: usize,
    pub complete: bool,
}

pub fn write_compile_status(package_dir: &Path, status: &CompileStatus) -> Result<(), String> {
    let mut json = serde_json::to_vec_pretty(status)
        .map_err(|error| format!("cannot serialize compile_status.json: {error}"))?;
    json.push(b'\n');
    fs::write(package_dir.join("compile_status.json"), json)
        .map_err(|error| format!("cannot write compile_status.json: {error}"))
}

pub fn read_compile_status(package_dir: &Path) -> Result<CompileStatus, String> {
    read_document(package_dir, "compile_status.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chapter_analysis::{ChapterSummary, DifficultPassage, GeneratedChapterAnalysis};

    #[test]
    fn grounding_rejects_bad_ranges_fingerprints_spoilers_and_ungrounded_claims() {
        let blocks = BTreeMap::from([
            (
                "block-1".to_owned(),
                GroundingBlock {
                    chapter_index: 0,
                    text: "First".to_owned(),
                    text_fingerprint: "fingerprint-1".to_owned(),
                },
            ),
            (
                "block-2".to_owned(),
                GroundingBlock {
                    chapter_index: 1,
                    text: "Second".to_owned(),
                    text_fingerprint: "fingerprint-2".to_owned(),
                },
            ),
        ]);
        let analyses = vec![ChapterAnalysis {
            schema_version: "0.1".to_owned(),
            analysis_profile: "standard".to_owned(),
            chapter_id: "chapter_001".to_owned(),
            spine_index: 0,
            chapter_title: "First".to_owned(),
            source_content_hash: "chapter-hash".to_owned(),
            generated: GeneratedChapterAnalysis {
                summary: ChapterSummary {
                    one_sentence: "Summary".to_owned(),
                    short: "Summary".to_owned(),
                    deep: "Summary".to_owned(),
                    role_in_book: "Opening".to_owned(),
                },
                key_ideas: vec!["Idea".to_owned()],
                concepts: Vec::new(),
                claims: Vec::new(),
                argument_flow: Vec::new(),
                difficult_passages: vec![DifficultPassage {
                    source_ref: AnalysisSourceRef {
                        block_id: "block-2".to_owned(),
                        start_char: 0,
                        end_char: 99,
                        text_fingerprint: "wrong".to_owned(),
                    },
                    reason: "Hard".to_owned(),
                    explanation: "Explanation".to_owned(),
                }],
                entities: Vec::new(),
            },
        }];
        let synthesis = GeneratedBookSynthesis {
            book_map: BookMap {
                central_question: "Question".to_owned(),
                thesis: "Thesis".to_owned(),
                chapter_roles: Vec::new(),
                reading_paths: Vec::new(),
                difficulty_map: Vec::new(),
                key_chapter_ids: Vec::new(),
            },
            concepts: Vec::new(),
            claims: vec![Claim {
                claim_id: "claim-1".to_owned(),
                claim: "Claim".to_owned(),
                claim_type: ClaimType::Thesis,
                supporting_evidence: Vec::new(),
                assumptions: Vec::new(),
                counterpoints: Vec::new(),
                depends_on_claim_ids: Vec::new(),
                importance: 100,
                grounding: Grounding::Grounded,
            }],
            entities: Vec::new(),
            checkpoints: Vec::new(),
            book_reflection_questions: Vec::new(),
        };

        let report = validate_grounding("source-hash", &blocks, &analyses, &synthesis);
        assert!(!report.valid);
        let codes = report
            .issues
            .iter()
            .map(|issue| issue.code.as_str())
            .collect::<BTreeSet<_>>();
        assert!(codes.contains("invalid_char_range"));
        assert!(codes.contains("fingerprint_mismatch"));
        assert!(codes.contains("spoiler_boundary"));
        assert!(codes.contains("claim_without_evidence"));
    }
}
