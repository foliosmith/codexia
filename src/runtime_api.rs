//! Runtime API contract v1.
//!
//! The module is HTTP-framework neutral. It defines the public routes, their
//! request/response schemas, and the reader-state contracts used by the local
//! Web Reader runtime.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{
    ir::{Profile, SourceRef},
    pipeline::AnalyzerStage,
    reader_cards::ReaderCard,
};

/// Public API version prefix.
pub const API_VERSION: &str = "v1";

/// HTTP methods used by the runtime contract.
#[derive(Debug, Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum HttpMethod {
    Get,
    Post,
    Patch,
}

impl HttpMethod {
    /// Stable uppercase method name.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Patch => "PATCH",
        }
    }
}

/// Stable operation identifier independent of the route path.
#[derive(Debug, Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum RuntimeOperation {
    UploadBook,
    GetBookStatus,
    GetBookPackage,
    GetBookMap,
    GetChapterAnalysis,
    GetConcepts,
    GetClaims,
    ExplainPassage,
    AskBook,
    GenerateCheckpoint,
    ReflectOnAnswer,
    CreateReaderSession,
    UpdateReaderSession,
}

impl RuntimeOperation {
    /// Stable operation name for logs and generated clients.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::UploadBook => "upload_book",
            Self::GetBookStatus => "get_book_status",
            Self::GetBookPackage => "get_book_package",
            Self::GetBookMap => "get_book_map",
            Self::GetChapterAnalysis => "get_chapter_analysis",
            Self::GetConcepts => "get_concepts",
            Self::GetClaims => "get_claims",
            Self::ExplainPassage => "explain_passage",
            Self::AskBook => "ask_book",
            Self::GenerateCheckpoint => "generate_checkpoint",
            Self::ReflectOnAnswer => "reflect_on_answer",
            Self::CreateReaderSession => "create_reader_session",
            Self::UpdateReaderSession => "update_reader_session",
        }
    }
}

/// Named path parameters exposed by the API.
#[derive(Debug, Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum PathParameter {
    BookId,
    ChapterId,
    SessionId,
}

impl PathParameter {
    const fn placeholder(self) -> &'static str {
        match self {
            Self::BookId => "{book_id}",
            Self::ChapterId => "{chapter_id}",
            Self::SessionId => "{session_id}",
        }
    }
}

/// Request and response schema identifiers used by endpoint definitions.
#[derive(Debug, Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ApiSchema {
    BookUploadRequest,
    BookAccepted,
    BookStatus,
    BookPackage,
    BookMapCard,
    ChapterSummaryCard,
    ConceptCardList,
    ClaimCardList,
    ExplainRequest,
    AskRequest,
    CheckpointRequest,
    ReflectRequest,
    CreateReaderSessionRequest,
    UpdateReaderSessionRequest,
    ReaderSession,
    ReaderCardResponse,
}

impl ApiSchema {
    /// Stable schema name for API documentation and generated clients.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::BookUploadRequest => "BookUploadRequest",
            Self::BookAccepted => "BookAccepted",
            Self::BookStatus => "BookStatus",
            Self::BookPackage => "BookPackage",
            Self::BookMapCard => "BookMapCard",
            Self::ChapterSummaryCard => "ChapterSummaryCard",
            Self::ConceptCardList => "ConceptCardList",
            Self::ClaimCardList => "ClaimCardList",
            Self::ExplainRequest => "ExplainRequest",
            Self::AskRequest => "AskRequest",
            Self::CheckpointRequest => "CheckpointRequest",
            Self::ReflectRequest => "ReflectRequest",
            Self::CreateReaderSessionRequest => "CreateReaderSessionRequest",
            Self::UpdateReaderSessionRequest => "UpdateReaderSessionRequest",
            Self::ReaderSession => "ReaderSession",
            Self::ReaderCardResponse => "ReaderCardResponse",
        }
    }
}

/// Contract for one public endpoint.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct EndpointDefinition {
    pub operation: RuntimeOperation,
    pub method: HttpMethod,
    pub path: &'static str,
    pub path_parameters: &'static [PathParameter],
    pub request: Option<ApiSchema>,
    pub response: ApiSchema,
}

/// Versioned collection of runtime endpoints.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct RuntimeApiDefinition {
    pub version: &'static str,
    pub endpoints: &'static [EndpointDefinition],
}

impl RuntimeApiDefinition {
    /// Look up an endpoint by stable operation ID.
    #[must_use]
    pub fn endpoint(&self, operation: RuntimeOperation) -> Option<&EndpointDefinition> {
        self.endpoints
            .iter()
            .find(|endpoint| endpoint.operation == operation)
    }

    /// Validate route uniqueness and canonical operation coverage.
    pub fn validate(&self) -> Result<(), RuntimeApiError> {
        if self.version.is_empty() {
            return Err(RuntimeApiError::EmptyVersion);
        }

        let expected_prefix = format!("/{}/", self.version);
        let mut operations = BTreeSet::new();
        let mut routes = BTreeSet::new();

        for endpoint in self.endpoints {
            if !operations.insert(endpoint.operation) {
                return Err(RuntimeApiError::DuplicateOperation(endpoint.operation));
            }
            if !routes.insert((endpoint.method, endpoint.path)) {
                return Err(RuntimeApiError::DuplicateRoute {
                    method: endpoint.method,
                    path: endpoint.path,
                });
            }
            if !endpoint.path.starts_with(&expected_prefix) {
                return Err(RuntimeApiError::InvalidVersionedPath(endpoint.operation));
            }
            let placeholder_count = endpoint.path.bytes().filter(|byte| *byte == b'{').count();
            if placeholder_count != endpoint.path_parameters.len()
                || endpoint
                    .path_parameters
                    .iter()
                    .any(|parameter| !endpoint.path.contains(parameter.placeholder()))
            {
                return Err(RuntimeApiError::InvalidPathParameters(endpoint.operation));
            }
        }

        for operation in CANONICAL_OPERATIONS {
            if !operations.contains(&operation) {
                return Err(RuntimeApiError::MissingOperation(operation));
            }
        }

        if operations.len() != CANONICAL_OPERATIONS.len() {
            return Err(RuntimeApiError::UnexpectedOperationCount {
                expected: CANONICAL_OPERATIONS.len(),
                actual: operations.len(),
            });
        }

        Ok(())
    }
}

/// Invalid runtime API definition.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum RuntimeApiError {
    EmptyVersion,
    DuplicateOperation(RuntimeOperation),
    DuplicateRoute {
        method: HttpMethod,
        path: &'static str,
    },
    InvalidVersionedPath(RuntimeOperation),
    InvalidPathParameters(RuntimeOperation),
    MissingOperation(RuntimeOperation),
    UnexpectedOperationCount {
        expected: usize,
        actual: usize,
    },
}

const BOOK_ID: &[PathParameter] = &[PathParameter::BookId];
const BOOK_AND_CHAPTER_IDS: &[PathParameter] = &[PathParameter::BookId, PathParameter::ChapterId];
const SESSION_ID: &[PathParameter] = &[PathParameter::SessionId];

const CANONICAL_OPERATIONS: [RuntimeOperation; 13] = [
    RuntimeOperation::UploadBook,
    RuntimeOperation::GetBookStatus,
    RuntimeOperation::GetBookPackage,
    RuntimeOperation::GetBookMap,
    RuntimeOperation::GetChapterAnalysis,
    RuntimeOperation::GetConcepts,
    RuntimeOperation::GetClaims,
    RuntimeOperation::ExplainPassage,
    RuntimeOperation::AskBook,
    RuntimeOperation::GenerateCheckpoint,
    RuntimeOperation::ReflectOnAnswer,
    RuntimeOperation::CreateReaderSession,
    RuntimeOperation::UpdateReaderSession,
];

const ENDPOINTS: &[EndpointDefinition] = &[
    EndpointDefinition {
        operation: RuntimeOperation::UploadBook,
        method: HttpMethod::Post,
        path: "/v1/books",
        path_parameters: &[],
        request: Some(ApiSchema::BookUploadRequest),
        response: ApiSchema::BookAccepted,
    },
    EndpointDefinition {
        operation: RuntimeOperation::GetBookStatus,
        method: HttpMethod::Get,
        path: "/v1/books/{book_id}/status",
        path_parameters: BOOK_ID,
        request: None,
        response: ApiSchema::BookStatus,
    },
    EndpointDefinition {
        operation: RuntimeOperation::GetBookPackage,
        method: HttpMethod::Get,
        path: "/v1/books/{book_id}/package",
        path_parameters: BOOK_ID,
        request: None,
        response: ApiSchema::BookPackage,
    },
    EndpointDefinition {
        operation: RuntimeOperation::GetBookMap,
        method: HttpMethod::Get,
        path: "/v1/books/{book_id}/map",
        path_parameters: BOOK_ID,
        request: None,
        response: ApiSchema::BookMapCard,
    },
    EndpointDefinition {
        operation: RuntimeOperation::GetChapterAnalysis,
        method: HttpMethod::Get,
        path: "/v1/books/{book_id}/chapters/{chapter_id}/analysis",
        path_parameters: BOOK_AND_CHAPTER_IDS,
        request: None,
        response: ApiSchema::ChapterSummaryCard,
    },
    EndpointDefinition {
        operation: RuntimeOperation::GetConcepts,
        method: HttpMethod::Get,
        path: "/v1/books/{book_id}/concepts",
        path_parameters: BOOK_ID,
        request: None,
        response: ApiSchema::ConceptCardList,
    },
    EndpointDefinition {
        operation: RuntimeOperation::GetClaims,
        method: HttpMethod::Get,
        path: "/v1/books/{book_id}/claims",
        path_parameters: BOOK_ID,
        request: None,
        response: ApiSchema::ClaimCardList,
    },
    EndpointDefinition {
        operation: RuntimeOperation::ExplainPassage,
        method: HttpMethod::Post,
        path: "/v1/books/{book_id}/explain",
        path_parameters: BOOK_ID,
        request: Some(ApiSchema::ExplainRequest),
        response: ApiSchema::ReaderCardResponse,
    },
    EndpointDefinition {
        operation: RuntimeOperation::AskBook,
        method: HttpMethod::Post,
        path: "/v1/books/{book_id}/ask",
        path_parameters: BOOK_ID,
        request: Some(ApiSchema::AskRequest),
        response: ApiSchema::ReaderCardResponse,
    },
    EndpointDefinition {
        operation: RuntimeOperation::GenerateCheckpoint,
        method: HttpMethod::Post,
        path: "/v1/books/{book_id}/chapters/{chapter_id}/checkpoint",
        path_parameters: BOOK_AND_CHAPTER_IDS,
        request: Some(ApiSchema::CheckpointRequest),
        response: ApiSchema::ReaderCardResponse,
    },
    EndpointDefinition {
        operation: RuntimeOperation::ReflectOnAnswer,
        method: HttpMethod::Post,
        path: "/v1/books/{book_id}/chapters/{chapter_id}/reflect",
        path_parameters: BOOK_AND_CHAPTER_IDS,
        request: Some(ApiSchema::ReflectRequest),
        response: ApiSchema::ReaderCardResponse,
    },
    EndpointDefinition {
        operation: RuntimeOperation::CreateReaderSession,
        method: HttpMethod::Post,
        path: "/v1/reader-sessions",
        path_parameters: &[],
        request: Some(ApiSchema::CreateReaderSessionRequest),
        response: ApiSchema::ReaderSession,
    },
    EndpointDefinition {
        operation: RuntimeOperation::UpdateReaderSession,
        method: HttpMethod::Patch,
        path: "/v1/reader-sessions/{session_id}",
        path_parameters: SESSION_ID,
        request: Some(ApiSchema::UpdateReaderSessionRequest),
        response: ApiSchema::ReaderSession,
    },
];

/// Canonical v1 Runtime API.
#[must_use]
pub const fn runtime_api() -> RuntimeApiDefinition {
    RuntimeApiDefinition {
        version: API_VERSION,
        endpoints: ENDPOINTS,
    }
}

/// Multipart upload content accepted by `POST /v1/books`.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct BookUploadRequest {
    pub file_name: String,
    pub media_type: String,
    pub content: Vec<u8>,
    pub profile: Profile,
}

impl BookUploadRequest {
    pub fn validate(&self) -> Result<(), ApiRequestError> {
        require_text(&self.file_name, "file_name")?;
        require_text(&self.media_type, "media_type")?;
        if self.content.is_empty() {
            Err(ApiRequestError::EmptyContent)
        } else {
            Ok(())
        }
    }
}

/// Initial response after accepting a book for processing.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct BookAccepted {
    pub book_id: String,
    pub status_href: String,
}

/// Runtime processing lifecycle.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum BookProcessingState {
    Queued,
    Processing,
    Ready,
    Failed,
}

/// Processing status returned for a book.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct BookStatus {
    pub book_id: String,
    pub state: BookProcessingState,
    pub current_stage: Option<AnalyzerStage>,
    pub completed_stages: Vec<AnalyzerStage>,
    pub progress_basis_points: u16,
    pub error: Option<ApiError>,
}

/// One downloadable entry in a compiled package.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PackageEntry {
    pub name: String,
    pub media_type: String,
    pub href: String,
    pub source_hash: String,
}

/// Book Package response without embedding large artifacts inline.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct BookPackage {
    pub book_id: String,
    pub format_version: String,
    pub entries: Vec<PackageEntry>,
}

/// Location tracked by reader state and spoiler boundaries.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReaderLocation {
    pub chapter_id: String,
    pub block_id: Option<String>,
    pub char_offset: Option<u32>,
    pub epub_cfi: Option<String>,
}

/// Client reading state sent with contextual agent actions.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReaderState {
    pub session_id: Option<String>,
    pub current_location: ReaderLocation,
    pub read_until: ReaderLocation,
    #[serde(default)]
    pub read_coverage: Vec<ReaderLocation>,
    pub completed_chapter_ids: Vec<String>,
    pub progress_basis_points: u16,
}

impl ReaderState {
    /// Validate bounded progress and required chapter locations.
    pub fn validate(&self) -> Result<(), ApiRequestError> {
        require_text(
            &self.current_location.chapter_id,
            "current_location.chapter_id",
        )?;
        require_text(&self.read_until.chapter_id, "read_until.chapter_id")?;
        validate_progress(self.progress_basis_points)
    }
}

/// Policy for using content beyond the reader's current position.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpoilerMode {
    /// Use only explicitly recorded chapter prefixes.
    ReadRange,
    /// Explicitly allow the complete current chapter only.
    CurrentChapter,
    /// Allow the whole book when explicitly requested by the reader.
    FullBook,
}

/// Effective boundary applied to an agent response.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct SpoilerBoundary {
    pub mode: SpoilerMode,
    pub read_until: Option<ReaderLocation>,
    pub excluded_chapter_ids: Vec<String>,
    #[serde(default)]
    pub read_coverage: Vec<ReaderLocation>,
}

/// Request for a grounded passage explanation.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ExplainRequest {
    pub selected_text: String,
    pub source_ref: SourceRef,
    pub reader_state: ReaderState,
    pub spoiler_mode: SpoilerMode,
}

impl ExplainRequest {
    pub fn validate(&self) -> Result<(), ApiRequestError> {
        require_text(&self.selected_text, "selected_text")?;
        self.reader_state.validate()
    }
}

/// Request to ask a question constrained by reader state.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct AskRequest {
    pub question: String,
    pub context_refs: Vec<SourceRef>,
    pub reader_state: ReaderState,
    pub spoiler_mode: SpoilerMode,
}

impl AskRequest {
    pub fn validate(&self) -> Result<(), ApiRequestError> {
        require_text(&self.question, "question")?;
        self.reader_state.validate()
    }
}

/// Request to generate or refresh a chapter checkpoint.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CheckpointRequest {
    pub reader_state: ReaderState,
    pub spoiler_mode: SpoilerMode,
    pub force_regenerate: bool,
}

impl CheckpointRequest {
    pub fn validate(&self) -> Result<(), ApiRequestError> {
        self.reader_state.validate()
    }
}

/// Reader's answer submitted for reflection feedback.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ReflectRequest {
    pub checkpoint_id: String,
    pub question_id: String,
    pub answer: String,
    pub reader_state: ReaderState,
}

impl ReflectRequest {
    pub fn validate(&self) -> Result<(), ApiRequestError> {
        require_text(&self.checkpoint_id, "checkpoint_id")?;
        require_text(&self.question_id, "question_id")?;
        require_text(&self.answer, "answer")?;
        self.reader_state.validate()
    }
}

/// Create a persistent reader session.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct CreateReaderSessionRequest {
    pub book_id: String,
    pub current_location: ReaderLocation,
    pub spoiler_mode: SpoilerMode,
}

impl CreateReaderSessionRequest {
    pub fn validate(&self) -> Result<(), ApiRequestError> {
        require_text(&self.book_id, "book_id")?;
        require_text(
            &self.current_location.chapter_id,
            "current_location.chapter_id",
        )
    }
}

/// Patch only the reader-session fields supplied by the client.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct UpdateReaderSessionRequest {
    pub current_location: Option<ReaderLocation>,
    pub read_until: Option<ReaderLocation>,
    pub progress_basis_points: Option<u16>,
    pub spoiler_mode: Option<SpoilerMode>,
}

impl UpdateReaderSessionRequest {
    pub fn validate(&self) -> Result<(), ApiRequestError> {
        if self.current_location.is_none()
            && self.read_until.is_none()
            && self.progress_basis_points.is_none()
            && self.spoiler_mode.is_none()
        {
            return Err(ApiRequestError::EmptySessionPatch);
        }
        if let Some(progress) = self.progress_basis_points {
            validate_progress(progress)?;
        }
        if let Some(location) = &self.current_location {
            require_text(&location.chapter_id, "current_location.chapter_id")?;
        }
        if let Some(location) = &self.read_until {
            require_text(&location.chapter_id, "read_until.chapter_id")?;
        }
        Ok(())
    }
}

/// Persisted reader session returned after create or update.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReaderSession {
    pub session_id: String,
    pub book_id: String,
    pub current_location: ReaderLocation,
    pub read_until: ReaderLocation,
    pub progress_basis_points: u16,
    pub spoiler_mode: SpoilerMode,
    pub revision: u64,
    #[serde(default)]
    pub read_coverage: Vec<ReaderLocation>,
    #[serde(default)]
    pub completed_chapter_ids: Vec<String>,
}

/// Common response for explain, ask, checkpoint, and reflect actions.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ReaderCardResponse {
    pub cards: Vec<ReaderCard>,
    pub spoiler_boundary: SpoilerBoundary,
}

/// Stable API error envelope.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ApiError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

/// Invalid request payload.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ApiRequestError {
    EmptyField(&'static str),
    EmptyContent,
    ProgressOutOfRange(u16),
    EmptySessionPatch,
}

fn require_text(value: &str, field: &'static str) -> Result<(), ApiRequestError> {
    if value.trim().is_empty() {
        Err(ApiRequestError::EmptyField(field))
    } else {
        Ok(())
    }
}

fn validate_progress(progress_basis_points: u16) -> Result<(), ApiRequestError> {
    if progress_basis_points > 10_000 {
        Err(ApiRequestError::ProgressOutOfRange(progress_basis_points))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn location(chapter_id: &str) -> ReaderLocation {
        ReaderLocation {
            chapter_id: chapter_id.to_owned(),
            block_id: Some("block-1".to_owned()),
            char_offset: Some(12),
            epub_cfi: None,
        }
    }

    fn reader_state() -> ReaderState {
        ReaderState {
            session_id: Some("session-1".to_owned()),
            current_location: location("chapter-2"),
            read_until: location("chapter-2"),
            read_coverage: Vec::new(),
            completed_chapter_ids: vec!["chapter-1".to_owned()],
            progress_basis_points: 2_500,
        }
    }

    #[test]
    fn canonical_runtime_api_has_all_thirteen_routes() {
        let api = runtime_api();

        assert_eq!(api.endpoints.len(), 13);
        assert_eq!(api.validate(), Ok(()));
        for operation in CANONICAL_OPERATIONS {
            assert!(api.endpoint(operation).is_some(), "{}", operation.id());
        }
    }

    #[test]
    fn contextual_actions_and_session_patch_use_expected_routes() {
        let api = runtime_api();

        let explain = api
            .endpoint(RuntimeOperation::ExplainPassage)
            .expect("explain endpoint");
        assert_eq!(explain.method, HttpMethod::Post);
        assert_eq!(explain.path, "/v1/books/{book_id}/explain");
        assert_eq!(explain.request, Some(ApiSchema::ExplainRequest));

        let update_session = api
            .endpoint(RuntimeOperation::UpdateReaderSession)
            .expect("update session endpoint");
        assert_eq!(update_session.method, HttpMethod::Patch);
        assert_eq!(update_session.path, "/v1/reader-sessions/{session_id}");
    }

    #[test]
    fn get_endpoints_do_not_declare_request_bodies() {
        let api = runtime_api();

        assert!(api
            .endpoints
            .iter()
            .filter(|endpoint| endpoint.method == HttpMethod::Get)
            .all(|endpoint| endpoint.request.is_none()));
    }

    #[test]
    fn endpoint_path_parameters_must_match_placeholders() {
        const INVALID_ENDPOINTS: &[EndpointDefinition] = &[EndpointDefinition {
            operation: RuntimeOperation::UploadBook,
            method: HttpMethod::Post,
            path: "/v1/books",
            path_parameters: BOOK_ID,
            request: Some(ApiSchema::BookUploadRequest),
            response: ApiSchema::BookAccepted,
        }];
        let api = RuntimeApiDefinition {
            version: API_VERSION,
            endpoints: INVALID_ENDPOINTS,
        };

        assert_eq!(
            api.validate(),
            Err(RuntimeApiError::InvalidPathParameters(
                RuntimeOperation::UploadBook,
            )),
        );
    }

    #[test]
    fn ask_request_carries_reader_state_and_spoiler_mode() {
        let request = AskRequest {
            question: "How does this connect to chapter one?".to_owned(),
            context_refs: vec![],
            reader_state: reader_state(),
            spoiler_mode: SpoilerMode::ReadRange,
        };

        assert_eq!(request.spoiler_mode, SpoilerMode::ReadRange);
        assert_eq!(request.reader_state.read_until.chapter_id, "chapter-2");
        assert_eq!(request.validate(), Ok(()));
    }

    #[test]
    fn empty_or_out_of_range_session_patches_are_rejected() {
        let empty = UpdateReaderSessionRequest {
            current_location: None,
            read_until: None,
            progress_basis_points: None,
            spoiler_mode: None,
        };
        let invalid_progress = UpdateReaderSessionRequest {
            current_location: None,
            read_until: None,
            progress_basis_points: Some(10_001),
            spoiler_mode: None,
        };

        assert_eq!(empty.validate(), Err(ApiRequestError::EmptySessionPatch));
        assert_eq!(
            invalid_progress.validate(),
            Err(ApiRequestError::ProgressOutOfRange(10_001)),
        );
    }

    #[test]
    fn upload_and_session_creation_validate_required_content() {
        let upload = BookUploadRequest {
            file_name: "book.epub".to_owned(),
            media_type: "application/epub+zip".to_owned(),
            content: vec![],
            profile: Profile::Standard,
        };
        let session = CreateReaderSessionRequest {
            book_id: "".to_owned(),
            current_location: location("chapter-1"),
            spoiler_mode: SpoilerMode::ReadRange,
        };

        assert_eq!(upload.validate(), Err(ApiRequestError::EmptyContent));
        assert_eq!(
            session.validate(),
            Err(ApiRequestError::EmptyField("book_id")),
        );
    }
}
