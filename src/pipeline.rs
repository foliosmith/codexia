//! Analyzer pipeline contract.
//!
//! This module defines the boundary between the deterministic EPUB compiler
//! and later LLM-backed analysis. It deliberately describes *what* each stage
//! consumes and publishes without choosing an LLM provider or an index engine.

use std::collections::BTreeSet;

/// Version of the analyzer pipeline contract.
pub const ANALYZER_PIPELINE_VERSION: &str = "0.2";

/// The six ordered stages of book analysis.
#[derive(Debug, Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum AnalyzerStage {
    Parse,
    Normalize,
    ChapterAnalysis,
    BookSynthesis,
    Indexing,
    GroundingValidation,
}

impl AnalyzerStage {
    /// Stable identifier used by manifests, logs, and persisted checkpoints.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Parse => "parse",
            Self::Normalize => "normalize",
            Self::ChapterAnalysis => "chapter_analysis",
            Self::BookSynthesis => "book_synthesis",
            Self::Indexing => "indexing",
            Self::GroundingValidation => "grounding_validation",
        }
    }
}

/// A durable artifact crossing a stage boundary.
#[derive(Debug, Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Artifact {
    EpubSource,
    ParsedBook,
    BookIr,
    ChapterAnalyses,
    BookMap,
    Concepts,
    Claims,
    Entities,
    Checkpoints,
    LexicalIndex,
    VectorIndex,
    GraphIndex,
    EvalReport,
}

impl Artifact {
    /// Stable artifact name used for persisted package entries.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::EpubSource => "epub_source",
            Self::ParsedBook => "parsed_book",
            Self::BookIr => "book_ir",
            Self::ChapterAnalyses => "chapter_analyses",
            Self::BookMap => "book_map",
            Self::Concepts => "concepts",
            Self::Claims => "claims",
            Self::Entities => "entities",
            Self::Checkpoints => "checkpoints",
            Self::LexicalIndex => "lexical_index",
            Self::VectorIndex => "vector_index",
            Self::GraphIndex => "graph_index",
            Self::EvalReport => "eval_report",
        }
    }

    const fn is_external_input(self) -> bool {
        matches!(self, Self::EpubSource)
    }
}

/// Unit of work used to schedule a stage.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ExecutionScope {
    /// One deterministic operation for the entire book.
    Book,
    /// Fan out by chapter, then collect results in spine order.
    Chapter,
    /// Build the lexical, vector, and graph indexes independently.
    Index,
    /// Validate references independently, then reduce into one report.
    SourceRef,
}

/// Static contract for one analyzer stage.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct StageDefinition {
    pub stage: AnalyzerStage,
    pub description: &'static str,
    pub scope: ExecutionScope,
    pub depends_on: &'static [AnalyzerStage],
    pub inputs: &'static [Artifact],
    pub outputs: &'static [Artifact],
}

/// A complete, versioned analyzer pipeline definition.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PipelineDefinition {
    pub version: &'static str,
    pub stages: &'static [StageDefinition],
}

impl PipelineDefinition {
    /// Find a stage contract by its stable stage value.
    #[must_use]
    pub fn stage(&self, stage: AnalyzerStage) -> Option<&StageDefinition> {
        self.stages
            .iter()
            .find(|definition| definition.stage == stage)
    }

    /// Check ordering and artifact-flow invariants before an executor uses the
    /// definition. A valid pipeline has every canonical stage exactly once,
    /// declares dependencies before consumers, and produces every non-external
    /// input before it is consumed.
    pub fn validate(&self) -> Result<(), PipelineDefinitionError> {
        if self.version.is_empty() {
            return Err(PipelineDefinitionError::EmptyVersion);
        }

        let mut seen_stages = BTreeSet::new();
        let mut available_artifacts = BTreeSet::new();
        let mut produced_artifacts = BTreeSet::new();

        for definition in self.stages {
            if !seen_stages.insert(definition.stage) {
                return Err(PipelineDefinitionError::DuplicateStage(definition.stage));
            }

            for dependency in definition.depends_on {
                if !seen_stages.contains(dependency) {
                    return Err(PipelineDefinitionError::DependencyNotReady {
                        stage: definition.stage,
                        dependency: *dependency,
                    });
                }
            }

            for input in definition.inputs {
                if !input.is_external_input() && !available_artifacts.contains(input) {
                    return Err(PipelineDefinitionError::InputNotReady {
                        stage: definition.stage,
                        artifact: *input,
                    });
                }
            }

            for output in definition.outputs {
                if !produced_artifacts.insert(*output) {
                    return Err(PipelineDefinitionError::DuplicateProducer(*output));
                }
                available_artifacts.insert(*output);
            }
        }

        for stage in CANONICAL_STAGE_ORDER {
            if !seen_stages.contains(&stage) {
                return Err(PipelineDefinitionError::MissingStage(stage));
            }
        }

        if seen_stages.len() != CANONICAL_STAGE_ORDER.len() {
            return Err(PipelineDefinitionError::UnexpectedStageCount {
                expected: CANONICAL_STAGE_ORDER.len(),
                actual: seen_stages.len(),
            });
        }

        if self.stages.last().map(|definition| definition.stage)
            != Some(AnalyzerStage::GroundingValidation)
        {
            return Err(PipelineDefinitionError::ValidationIsNotTerminal);
        }

        Ok(())
    }
}

/// Invalid analyzer pipeline definition.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PipelineDefinitionError {
    EmptyVersion,
    DuplicateStage(AnalyzerStage),
    MissingStage(AnalyzerStage),
    UnexpectedStageCount {
        expected: usize,
        actual: usize,
    },
    DependencyNotReady {
        stage: AnalyzerStage,
        dependency: AnalyzerStage,
    },
    InputNotReady {
        stage: AnalyzerStage,
        artifact: Artifact,
    },
    DuplicateProducer(Artifact),
    ValidationIsNotTerminal,
}

const CANONICAL_STAGE_ORDER: [AnalyzerStage; 6] = [
    AnalyzerStage::Parse,
    AnalyzerStage::Normalize,
    AnalyzerStage::ChapterAnalysis,
    AnalyzerStage::BookSynthesis,
    AnalyzerStage::Indexing,
    AnalyzerStage::GroundingValidation,
];

const PARSE_OUTPUTS: &[Artifact] = &[Artifact::ParsedBook];
const NORMALIZE_OUTPUTS: &[Artifact] = &[Artifact::BookIr];
const CHAPTER_ANALYSIS_OUTPUTS: &[Artifact] = &[Artifact::ChapterAnalyses];
const BOOK_SYNTHESIS_OUTPUTS: &[Artifact] = &[
    Artifact::BookMap,
    Artifact::Concepts,
    Artifact::Claims,
    Artifact::Entities,
    Artifact::Checkpoints,
];
const INDEX_OUTPUTS: &[Artifact] = &[
    Artifact::LexicalIndex,
    Artifact::VectorIndex,
    Artifact::GraphIndex,
];
const VALIDATION_OUTPUTS: &[Artifact] = &[Artifact::EvalReport];

const PIPELINE_STAGES: &[StageDefinition] = &[
    StageDefinition {
        stage: AnalyzerStage::Parse,
        description: "Parse EPUB metadata, navigation, spine, and semantic content blocks",
        scope: ExecutionScope::Book,
        depends_on: &[],
        inputs: &[Artifact::EpubSource],
        outputs: PARSE_OUTPUTS,
    },
    StageDefinition {
        stage: AnalyzerStage::Normalize,
        description: "Normalize parsed content into stable Book IR and source references",
        scope: ExecutionScope::Book,
        depends_on: &[AnalyzerStage::Parse],
        inputs: PARSE_OUTPUTS,
        outputs: NORMALIZE_OUTPUTS,
    },
    StageDefinition {
        stage: AnalyzerStage::ChapterAnalysis,
        description: "Analyze chapters independently using normalized book context",
        scope: ExecutionScope::Chapter,
        depends_on: &[AnalyzerStage::Normalize],
        inputs: NORMALIZE_OUTPUTS,
        outputs: CHAPTER_ANALYSIS_OUTPUTS,
    },
    StageDefinition {
        stage: AnalyzerStage::BookSynthesis,
        description: "Synthesize the book map, concepts, claims, entities, and checkpoints",
        scope: ExecutionScope::Book,
        depends_on: &[AnalyzerStage::ChapterAnalysis],
        inputs: &[Artifact::BookIr, Artifact::ChapterAnalyses],
        outputs: BOOK_SYNTHESIS_OUTPUTS,
    },
    StageDefinition {
        stage: AnalyzerStage::Indexing,
        description:
            "Build lexical, vector, and graph indexes from normalized and synthesized artifacts",
        scope: ExecutionScope::Index,
        depends_on: &[AnalyzerStage::BookSynthesis],
        inputs: &[
            Artifact::BookIr,
            Artifact::ChapterAnalyses,
            Artifact::BookMap,
            Artifact::Concepts,
            Artifact::Claims,
            Artifact::Entities,
        ],
        outputs: INDEX_OUTPUTS,
    },
    StageDefinition {
        stage: AnalyzerStage::GroundingValidation,
        description: "Validate source references and grounding before the package becomes ready",
        scope: ExecutionScope::SourceRef,
        depends_on: &[AnalyzerStage::Indexing],
        inputs: &[
            Artifact::BookIr,
            Artifact::ChapterAnalyses,
            Artifact::BookMap,
            Artifact::Concepts,
            Artifact::Claims,
            Artifact::Entities,
            Artifact::Checkpoints,
            Artifact::LexicalIndex,
            Artifact::VectorIndex,
            Artifact::GraphIndex,
        ],
        outputs: VALIDATION_OUTPUTS,
    },
];

/// The canonical v0.1 analyzer pipeline.
#[must_use]
pub const fn analyzer_pipeline() -> PipelineDefinition {
    PipelineDefinition {
        version: ANALYZER_PIPELINE_VERSION,
        stages: PIPELINE_STAGES,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_pipeline_is_valid_and_ordered() {
        let pipeline = analyzer_pipeline();

        assert_eq!(pipeline.stages.len(), 6);
        assert_eq!(
            pipeline
                .stages
                .iter()
                .map(|definition| definition.stage)
                .collect::<Vec<_>>(),
            CANONICAL_STAGE_ORDER,
        );
        assert_eq!(pipeline.validate(), Ok(()));
    }

    #[test]
    fn chapter_analysis_is_the_only_chapter_fan_out() {
        let pipeline = analyzer_pipeline();
        let chapter_stages = pipeline
            .stages
            .iter()
            .filter(|definition| definition.scope == ExecutionScope::Chapter)
            .map(|definition| definition.stage)
            .collect::<Vec<_>>();

        assert_eq!(chapter_stages, vec![AnalyzerStage::ChapterAnalysis]);
    }

    #[test]
    fn validation_is_the_terminal_publication_gate() {
        let pipeline = analyzer_pipeline();
        let validation = pipeline
            .stage(AnalyzerStage::GroundingValidation)
            .expect("grounding validation stage");

        assert_eq!(validation.outputs, &[Artifact::EvalReport]);
        assert_eq!(pipeline.stages.last(), Some(validation));
    }

    #[test]
    fn invalid_artifact_flow_is_rejected() {
        const INVALID_STAGES: &[StageDefinition] = &[
            StageDefinition {
                stage: AnalyzerStage::Parse,
                description: "invalid parse",
                scope: ExecutionScope::Book,
                depends_on: &[],
                inputs: &[Artifact::EpubSource],
                outputs: &[Artifact::ParsedBook],
            },
            StageDefinition {
                stage: AnalyzerStage::Normalize,
                description: "invalid normalize",
                scope: ExecutionScope::Book,
                depends_on: &[AnalyzerStage::Parse],
                inputs: &[Artifact::Claims],
                outputs: &[Artifact::BookIr],
            },
        ];
        let pipeline = PipelineDefinition {
            version: "test",
            stages: INVALID_STAGES,
        };

        assert_eq!(
            pipeline.validate(),
            Err(PipelineDefinitionError::InputNotReady {
                stage: AnalyzerStage::Normalize,
                artifact: Artifact::Claims,
            }),
        );
    }
}
