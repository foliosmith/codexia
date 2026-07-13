//! Reader Card protocol v0.1.
//!
//! Reader Cards are the presentation-neutral responses returned by the Book
//! Agent. A client decides how to render them; the protocol only describes the
//! content, source grounding, and commands that a user can invoke.

use crate::ir::SourceRef;

/// Version of the Reader Card wire contract.
pub const READER_CARD_VERSION: &str = "0.1";

/// Discriminator for every Reader Card variant.
#[derive(Debug, Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum CardKind {
    BookMap,
    ChapterSummary,
    Explanation,
    Concept,
    Claim,
    Checkpoint,
    Flashcard,
    Question,
    Export,
}

impl CardKind {
    /// Stable discriminator used by API clients.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::BookMap => "book_map",
            Self::ChapterSummary => "chapter_summary",
            Self::Explanation => "explanation",
            Self::Concept => "concept",
            Self::Claim => "claim",
            Self::Checkpoint => "checkpoint",
            Self::Flashcard => "flashcard",
            Self::Question => "question",
            Self::Export => "export",
        }
    }
}

/// Fields shared by every card.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CardMeta {
    pub card_id: String,
    pub book_id: String,
    pub title: String,
    pub source_refs: Vec<SourceRef>,
    pub actions: Vec<CardAction>,
}

/// A presentation-neutral card returned by the Book Agent.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum ReaderCard {
    BookMap(BookMapCard),
    ChapterSummary(ChapterSummaryCard),
    Explanation(ExplanationCard),
    Concept(ConceptCard),
    Claim(ClaimCard),
    Checkpoint(CheckpointCard),
    Flashcard(FlashcardCard),
    Question(QuestionCard),
    Export(ExportCard),
}

impl ReaderCard {
    /// Return the wire discriminator for this card.
    #[must_use]
    pub const fn kind(&self) -> CardKind {
        match self {
            Self::BookMap(_) => CardKind::BookMap,
            Self::ChapterSummary(_) => CardKind::ChapterSummary,
            Self::Explanation(_) => CardKind::Explanation,
            Self::Concept(_) => CardKind::Concept,
            Self::Claim(_) => CardKind::Claim,
            Self::Checkpoint(_) => CardKind::Checkpoint,
            Self::Flashcard(_) => CardKind::Flashcard,
            Self::Question(_) => CardKind::Question,
            Self::Export(_) => CardKind::Export,
        }
    }

    /// Access the common card envelope without matching the variant.
    #[must_use]
    pub const fn meta(&self) -> &CardMeta {
        match self {
            Self::BookMap(card) => &card.meta,
            Self::ChapterSummary(card) => &card.meta,
            Self::Explanation(card) => &card.meta,
            Self::Concept(card) => &card.meta,
            Self::Claim(card) => &card.meta,
            Self::Checkpoint(card) => &card.meta,
            Self::Flashcard(card) => &card.meta,
            Self::Question(card) => &card.meta,
            Self::Export(card) => &card.meta,
        }
    }

    /// Validate fields required by every client implementation.
    pub fn validate(&self) -> Result<(), ReaderCardError> {
        let meta = self.meta();
        require_text(&meta.card_id, "card_id")?;
        require_text(&meta.book_id, "book_id")?;
        require_text(&meta.title, "title")?;

        for action in &meta.actions {
            action.validate()?;
        }

        match self {
            Self::BookMap(card) => {
                require_text(&card.central_question, "central_question")?;
                require_text(&card.thesis, "thesis")
            }
            Self::ChapterSummary(card) => {
                require_text(&card.chapter_id, "chapter_id")?;
                require_text(&card.one_sentence, "one_sentence")?;
                require_text(&card.short_summary, "short_summary")
            }
            Self::Explanation(card) => {
                require_text(&card.passage, "passage")?;
                require_text(&card.explanation, "explanation")?;
                require_sources(meta, self.kind())
            }
            Self::Concept(card) => {
                require_text(&card.concept_id, "concept_id")?;
                require_text(&card.name, "name")?;
                require_text(&card.definition_in_this_book, "definition_in_this_book")
            }
            Self::Claim(card) => {
                require_text(&card.claim_id, "claim_id")?;
                require_text(&card.claim, "claim")?;
                require_sources(meta, self.kind())
            }
            Self::Checkpoint(card) => {
                require_text(&card.checkpoint_id, "checkpoint_id")?;
                require_text(&card.chapter_id, "chapter_id")?;
                if card.must_understand.is_empty() {
                    Err(ReaderCardError::EmptyCollection("must_understand"))
                } else {
                    Ok(())
                }
            }
            Self::Flashcard(card) => {
                require_text(&card.flashcard_id, "flashcard_id")?;
                require_text(&card.front, "front")?;
                require_text(&card.back, "back")
            }
            Self::Question(card) => {
                require_text(&card.question_id, "question_id")?;
                require_text(&card.prompt, "prompt")
            }
            Self::Export(card) => {
                require_text(&card.export_id, "export_id")?;
                require_text(&card.file_name, "file_name")
            }
        }
    }
}

/// A suggested route through the book.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ReadingPath {
    pub path_id: String,
    pub title: String,
    pub description: String,
    pub chapter_ids: Vec<String>,
}

/// Relative reading difficulty of a chapter.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum DifficultyLevel {
    Introductory,
    Intermediate,
    Advanced,
}

/// One entry in the book difficulty map.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ChapterDifficulty {
    pub chapter_id: String,
    pub level: DifficultyLevel,
    pub reason: String,
}

/// Whole-book navigation and synthesis card.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct BookMapCard {
    pub meta: CardMeta,
    pub central_question: String,
    pub thesis: String,
    pub reading_paths: Vec<ReadingPath>,
    pub difficulty_map: Vec<ChapterDifficulty>,
    pub key_chapter_ids: Vec<String>,
}

/// A chapter's role and layered summary.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ChapterSummaryCard {
    pub meta: CardMeta,
    pub chapter_id: String,
    pub one_sentence: String,
    pub short_summary: String,
    pub deep_summary: Option<String>,
    pub role_in_book: String,
    pub key_ideas: Vec<String>,
}

/// Grounded explanation of a selected passage.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ExplanationCard {
    pub meta: CardMeta,
    pub passage: String,
    pub explanation: String,
    pub simplified: Option<String>,
    pub why_it_matters: Option<String>,
    pub connections_to_prior_text: Vec<String>,
    pub related_concept_ids: Vec<String>,
}

/// Book-specific concept definition.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ConceptCard {
    pub meta: CardMeta,
    pub concept_id: String,
    pub name: String,
    pub aliases: Vec<String>,
    pub definition_in_this_book: String,
    pub related_concept_ids: Vec<String>,
}

/// Classification of a claim in the author's argument.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ClaimType {
    Thesis,
    Supporting,
    Counterclaim,
    Inference,
}

/// Evidence quoted or paraphrased from the source.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Evidence {
    pub text: String,
    pub source_ref: SourceRef,
}

/// Claim, evidence, and argument context.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ClaimCard {
    pub meta: CardMeta,
    pub claim_id: String,
    pub claim: String,
    pub claim_type: ClaimType,
    pub supporting_evidence: Vec<Evidence>,
    pub assumptions: Vec<String>,
    pub counterpoints: Vec<String>,
    pub depends_on_claim_ids: Vec<String>,
}

/// End-of-chapter understanding checkpoint.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CheckpointCard {
    pub meta: CardMeta,
    pub checkpoint_id: String,
    pub chapter_id: String,
    pub summary: String,
    pub must_understand: Vec<String>,
    pub recall_question_ids: Vec<String>,
    pub reflection_question_ids: Vec<String>,
    pub flashcard_ids: Vec<String>,
}

/// One spaced-repetition prompt and answer.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct FlashcardCard {
    pub meta: CardMeta,
    pub flashcard_id: String,
    pub front: String,
    pub back: String,
    pub concept_ids: Vec<String>,
}

/// Kind of reader question.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum QuestionKind {
    Recall,
    Reflection,
    Comprehension,
}

/// Recall, reflection, or comprehension prompt.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct QuestionCard {
    pub meta: CardMeta,
    pub question_id: String,
    pub kind: QuestionKind,
    pub prompt: String,
    pub expected_points: Vec<String>,
}

/// Supported export encoding.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ExportFormat {
    Json,
    Markdown,
    Obsidian,
}

/// Portion of a package included in an export.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum ExportScope {
    WholeBook,
    Chapters(Vec<String>),
    ReadRange,
}

/// Lifecycle of an asynchronous export.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ExportStatus {
    Pending,
    Ready,
    Failed,
}

/// Export artifact presented to the reader.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ExportCard {
    pub meta: CardMeta,
    pub export_id: String,
    pub format: ExportFormat,
    pub scope: ExportScope,
    pub status: ExportStatus,
    pub file_name: String,
    pub href: Option<String>,
}

/// Discriminator for a Card action.
#[derive(Debug, Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum CardActionKind {
    JumpToSource,
    Ask,
    ExplainMore,
    QuizMe,
    OpenChapter,
    Export,
}

impl CardActionKind {
    /// Stable action name used by clients.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::JumpToSource => "jump_to_source",
            Self::Ask => "ask",
            Self::ExplainMore => "explain_more",
            Self::QuizMe => "quiz_me",
            Self::OpenChapter => "open_chapter",
            Self::Export => "export",
        }
    }
}

/// Action command and the complete payload required to invoke it.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum CardAction {
    JumpToSource {
        source_ref: SourceRef,
    },
    Ask {
        suggested_question: Option<String>,
        source_refs: Vec<SourceRef>,
    },
    ExplainMore {
        source_refs: Vec<SourceRef>,
    },
    QuizMe {
        chapter_id: Option<String>,
        concept_ids: Vec<String>,
    },
    OpenChapter {
        chapter_id: String,
    },
    Export {
        format: ExportFormat,
        scope: ExportScope,
    },
}

impl CardAction {
    /// Return the stable command discriminator.
    #[must_use]
    pub const fn kind(&self) -> CardActionKind {
        match self {
            Self::JumpToSource { .. } => CardActionKind::JumpToSource,
            Self::Ask { .. } => CardActionKind::Ask,
            Self::ExplainMore { .. } => CardActionKind::ExplainMore,
            Self::QuizMe { .. } => CardActionKind::QuizMe,
            Self::OpenChapter { .. } => CardActionKind::OpenChapter,
            Self::Export { .. } => CardActionKind::Export,
        }
    }

    fn validate(&self) -> Result<(), ReaderCardError> {
        match self {
            Self::ExplainMore { source_refs } if source_refs.is_empty() => {
                Err(ReaderCardError::EmptyActionPayload(self.kind()))
            }
            Self::QuizMe {
                chapter_id,
                concept_ids,
            } if chapter_id
                .as_deref()
                .is_none_or(|chapter_id| chapter_id.trim().is_empty())
                && concept_ids.is_empty() =>
            {
                Err(ReaderCardError::EmptyActionPayload(self.kind()))
            }
            Self::OpenChapter { chapter_id } => require_text(chapter_id, "chapter_id"),
            _ => Ok(()),
        }
    }
}

/// A Reader Card violates the v0.1 wire contract.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ReaderCardError {
    EmptyField(&'static str),
    EmptyCollection(&'static str),
    MissingSourceRefs(CardKind),
    EmptyActionPayload(CardActionKind),
}

fn require_text(value: &str, field: &'static str) -> Result<(), ReaderCardError> {
    if value.trim().is_empty() {
        Err(ReaderCardError::EmptyField(field))
    } else {
        Ok(())
    }
}

fn require_sources(meta: &CardMeta, kind: CardKind) -> Result<(), ReaderCardError> {
    if meta.source_refs.is_empty() {
        Err(ReaderCardError::MissingSourceRefs(kind))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn source_ref() -> SourceRef {
        SourceRef {
            chapter_href: Arc::from("chapter-1.xhtml"),
            spine_index: 0,
            node_id: 4,
            cfi: Some("epubcfi(/6/2!/4/2)".to_owned()),
        }
    }

    fn meta() -> CardMeta {
        CardMeta {
            card_id: "card-1".to_owned(),
            book_id: "book-1".to_owned(),
            title: "Card".to_owned(),
            source_refs: vec![source_ref()],
            actions: vec![CardAction::JumpToSource {
                source_ref: source_ref(),
            }],
        }
    }

    #[test]
    fn union_exposes_all_nine_card_discriminators() {
        let cards = vec![
            ReaderCard::BookMap(BookMapCard {
                meta: meta(),
                central_question: "Why read?".to_owned(),
                thesis: "Reading changes attention.".to_owned(),
                reading_paths: vec![],
                difficulty_map: vec![],
                key_chapter_ids: vec![],
            }),
            ReaderCard::ChapterSummary(ChapterSummaryCard {
                meta: meta(),
                chapter_id: "chapter-1".to_owned(),
                one_sentence: "A premise is introduced.".to_owned(),
                short_summary: "The chapter establishes the premise.".to_owned(),
                deep_summary: None,
                role_in_book: "foundation".to_owned(),
                key_ideas: vec![],
            }),
            ReaderCard::Explanation(ExplanationCard {
                meta: meta(),
                passage: "Selected text".to_owned(),
                explanation: "Grounded explanation".to_owned(),
                simplified: None,
                why_it_matters: None,
                connections_to_prior_text: vec![],
                related_concept_ids: vec![],
            }),
            ReaderCard::Concept(ConceptCard {
                meta: meta(),
                concept_id: "concept-1".to_owned(),
                name: "Attention".to_owned(),
                aliases: vec![],
                definition_in_this_book: "Directed awareness.".to_owned(),
                related_concept_ids: vec![],
            }),
            ReaderCard::Claim(ClaimCard {
                meta: meta(),
                claim_id: "claim-1".to_owned(),
                claim: "Attention can be trained.".to_owned(),
                claim_type: ClaimType::Thesis,
                supporting_evidence: vec![],
                assumptions: vec![],
                counterpoints: vec![],
                depends_on_claim_ids: vec![],
            }),
            ReaderCard::Checkpoint(CheckpointCard {
                meta: meta(),
                checkpoint_id: "checkpoint-1".to_owned(),
                chapter_id: "chapter-1".to_owned(),
                summary: "Chapter checkpoint".to_owned(),
                must_understand: vec!["The premise".to_owned()],
                recall_question_ids: vec![],
                reflection_question_ids: vec![],
                flashcard_ids: vec![],
            }),
            ReaderCard::Flashcard(FlashcardCard {
                meta: meta(),
                flashcard_id: "flashcard-1".to_owned(),
                front: "What is attention?".to_owned(),
                back: "Directed awareness.".to_owned(),
                concept_ids: vec!["concept-1".to_owned()],
            }),
            ReaderCard::Question(QuestionCard {
                meta: meta(),
                question_id: "question-1".to_owned(),
                kind: QuestionKind::Recall,
                prompt: "State the premise.".to_owned(),
                expected_points: vec!["The premise".to_owned()],
            }),
            ReaderCard::Export(ExportCard {
                meta: meta(),
                export_id: "export-1".to_owned(),
                format: ExportFormat::Markdown,
                scope: ExportScope::WholeBook,
                status: ExportStatus::Ready,
                file_name: "notes.md".to_owned(),
                href: Some("/v1/exports/export-1".to_owned()),
            }),
        ];

        assert_eq!(
            cards.iter().map(ReaderCard::kind).collect::<Vec<_>>(),
            vec![
                CardKind::BookMap,
                CardKind::ChapterSummary,
                CardKind::Explanation,
                CardKind::Concept,
                CardKind::Claim,
                CardKind::Checkpoint,
                CardKind::Flashcard,
                CardKind::Question,
                CardKind::Export,
            ],
        );
        assert!(cards.iter().all(|card| card.validate().is_ok()));
    }

    #[test]
    fn action_discriminators_are_stable() {
        let actions = [
            CardAction::JumpToSource {
                source_ref: source_ref(),
            },
            CardAction::Ask {
                suggested_question: None,
                source_refs: vec![],
            },
            CardAction::ExplainMore {
                source_refs: vec![source_ref()],
            },
            CardAction::QuizMe {
                chapter_id: Some("chapter-1".to_owned()),
                concept_ids: vec![],
            },
            CardAction::OpenChapter {
                chapter_id: "chapter-1".to_owned(),
            },
            CardAction::Export {
                format: ExportFormat::Json,
                scope: ExportScope::ReadRange,
            },
        ];

        assert_eq!(
            actions
                .iter()
                .map(|action| action.kind().id())
                .collect::<Vec<_>>(),
            vec![
                "jump_to_source",
                "ask",
                "explain_more",
                "quiz_me",
                "open_chapter",
                "export",
            ],
        );
    }

    #[test]
    fn grounded_cards_require_source_references() {
        let mut card_meta = meta();
        card_meta.source_refs.clear();
        let card = ReaderCard::Explanation(ExplanationCard {
            meta: card_meta,
            passage: "Selected text".to_owned(),
            explanation: "Explanation".to_owned(),
            simplified: None,
            why_it_matters: None,
            connections_to_prior_text: vec![],
            related_concept_ids: vec![],
        });

        assert_eq!(
            card.validate(),
            Err(ReaderCardError::MissingSourceRefs(CardKind::Explanation)),
        );
    }

    #[test]
    fn payload_less_quiz_action_is_rejected() {
        let action = CardAction::QuizMe {
            chapter_id: None,
            concept_ids: vec![],
        };

        assert_eq!(
            action.validate(),
            Err(ReaderCardError::EmptyActionPayload(CardActionKind::QuizMe)),
        );
    }
}
