//! Authenticated multi-book Public API built on the local Web Runtime.

use std::{
    collections::{BTreeMap, VecDeque},
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex, MutexGuard},
    thread,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::web_runtime::{serve_http, HttpRequest, HttpResponse, WebRuntime};

const OPENAPI_JSON: &str = include_str!("../schemas/openapi.json");
const USAGE_VERSION: &str = "0.1";

#[derive(Debug, Clone)]
pub struct PublicApiConfig {
    pub api_key: String,
    pub rate_limit_per_minute: usize,
    pub agent_request_cost_micros: u64,
    pub compile_cost_micros: u64,
    pub analyzer_command: Option<PathBuf>,
    pub webhook_url: Option<String>,
    pub compiler_executable: PathBuf,
}

impl PublicApiConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.api_key.len() < 16 {
            return Err("API key must contain at least 16 characters".to_owned());
        }
        if self.rate_limit_per_minute == 0 {
            return Err("rate limit must be positive".to_owned());
        }
        if self
            .webhook_url
            .as_deref()
            .is_some_and(|url| !url.starts_with("http://") && !url.starts_with("https://"))
        {
            return Err("webhook URL must use http or https".to_owned());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum JobState {
    Queued,
    Processing,
    Ready,
    Failed,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
struct BookJob {
    book_id: String,
    state: JobState,
    progress_basis_points: u16,
    current_stage: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
struct UsageState {
    schema_version: String,
    request_count: u64,
    compile_count: u64,
    agent_request_count: u64,
    input_bytes: u64,
    output_bytes: u64,
    estimated_cost_micros: u64,
    by_operation: BTreeMap<String, u64>,
}

impl Default for UsageState {
    fn default() -> Self {
        Self {
            schema_version: USAGE_VERSION.to_owned(),
            request_count: 0,
            compile_count: 0,
            agent_request_count: 0,
            input_bytes: 0,
            output_bytes: 0,
            estimated_cost_micros: 0,
            by_operation: BTreeMap::new(),
        }
    }
}

pub struct PublicApi {
    library_dir: PathBuf,
    config: PublicApiConfig,
    books: Mutex<BTreeMap<String, Arc<WebRuntime>>>,
    jobs: Mutex<BTreeMap<String, BookJob>>,
    rate_window: Mutex<VecDeque<Instant>>,
    usage: Mutex<UsageState>,
}

impl PublicApi {
    pub fn load(library_dir: impl AsRef<Path>, config: PublicApiConfig) -> Result<Self, String> {
        config.validate()?;
        let library_dir = library_dir.as_ref().to_owned();
        fs::create_dir_all(library_dir.join("books"))
            .and_then(|()| fs::create_dir_all(library_dir.join("state")))
            .map_err(|error| format!("cannot create API library: {error}"))?;
        let usage_path = library_dir.join("usage.json");
        let usage = if usage_path.is_file() {
            let bytes = fs::read(&usage_path).map_err(|error| error.to_string())?;
            serde_json::from_slice(&bytes).map_err(|error| format!("usage.json: {error}"))?
        } else {
            UsageState::default()
        };
        let api = Self {
            library_dir,
            config,
            books: Mutex::new(BTreeMap::new()),
            jobs: Mutex::new(BTreeMap::new()),
            rate_window: Mutex::new(VecDeque::new()),
            usage: Mutex::new(usage),
        };
        api.load_existing_books()?;
        Ok(api)
    }

    fn load_existing_books(&self) -> Result<(), String> {
        let books_dir = self.library_dir.join("books");
        for entry in fs::read_dir(&books_dir).map_err(|error| error.to_string())? {
            let path = entry.map_err(|error| error.to_string())?.path();
            let package = path.join("package");
            if package.join("manifest.json").is_file() {
                self.register_package(&package)?;
            }
        }
        Ok(())
    }

    pub fn register_package(&self, package_dir: &Path) -> Result<String, String> {
        let manifest: Value = serde_json::from_slice(
            &fs::read(package_dir.join("manifest.json")).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        let source_hash = manifest
            .get("source_hash")
            .and_then(Value::as_str)
            .ok_or_else(|| "manifest source_hash is missing".to_owned())?;
        let book_id = source_hash.chars().take(16).collect::<String>();
        let state_dir = self.library_dir.join("state").join(&book_id);
        let runtime =
            WebRuntime::load(package_dir, state_dir, self.config.analyzer_command.clone())?;
        self.lock_books()
            .map_err(|error| error.message)?
            .insert(book_id.clone(), Arc::new(runtime));
        self.lock_jobs().map_err(|error| error.message)?.insert(
            book_id.clone(),
            BookJob {
                book_id: book_id.clone(),
                state: JobState::Ready,
                progress_basis_points: 10_000,
                current_stage: None,
                error: None,
            },
        );
        Ok(book_id)
    }

    pub fn dispatch(
        self: &Arc<Self>,
        method: &str,
        target: &str,
        headers: &BTreeMap<String, String>,
        body: &[u8],
    ) -> HttpResponse {
        let request = HttpRequest {
            method: method.to_owned(),
            target: target.to_owned(),
            headers: headers
                .iter()
                .map(|(name, value)| (name.to_ascii_lowercase(), value.clone()))
                .collect(),
            body: body.to_vec(),
        };
        self.dispatch_request(&request)
    }

    fn dispatch_request(self: &Arc<Self>, request: &HttpRequest) -> HttpResponse {
        let response = match self.authorize(request).and_then(|()| self.rate_limit()) {
            Ok(()) => self.route(request),
            Err(error) => error_response(error.status, error.code, error.message, error.retryable),
        };
        self.record_usage(request, &response);
        response
    }

    fn authorize(&self, request: &HttpRequest) -> Result<(), ApiError> {
        let path = request.target.split('?').next().unwrap_or(&request.target);
        if matches!(path, "/health" | "/openapi.json") {
            return Ok(());
        }
        let supplied = request
            .headers
            .get("authorization")
            .and_then(|value| value.strip_prefix("Bearer "))
            .or_else(|| request.headers.get("x-api-key").map(String::as_str))
            .unwrap_or("");
        if constant_time_eq(supplied.as_bytes(), self.config.api_key.as_bytes()) {
            Ok(())
        } else {
            Err(ApiError::new(401, "unauthorized", "invalid API key", false))
        }
    }

    fn rate_limit(&self) -> Result<(), ApiError> {
        let now = Instant::now();
        let mut window = self
            .rate_window
            .lock()
            .map_err(|_| ApiError::internal("rate limit lock is poisoned"))?;
        while window
            .front()
            .is_some_and(|instant| now.duration_since(*instant) >= Duration::from_secs(60))
        {
            window.pop_front();
        }
        if window.len() >= self.config.rate_limit_per_minute {
            return Err(ApiError::new(
                429,
                "rate_limited",
                "rate limit exceeded",
                true,
            ));
        }
        window.push_back(now);
        Ok(())
    }

    fn route(self: &Arc<Self>, request: &HttpRequest) -> HttpResponse {
        let path = request.target.split('?').next().unwrap_or(&request.target);
        if request.method == "GET" && path == "/health" {
            return json_response(200, json!({"status":"ok"}));
        }
        if request.method == "GET" && path == "/openapi.json" {
            return HttpResponse::bytes(200, "application/json", OPENAPI_JSON.as_bytes().to_vec());
        }
        if request.method == "GET" && path == "/v1/usage" {
            return match self.lock_usage() {
                Ok(usage) => json_response(200, &*usage),
                Err(error) => error_response(error.status, error.code, error.message, false),
            };
        }
        if request.method == "POST" && path == "/v1/books" {
            return self.upload(request);
        }
        let segments = path
            .split('/')
            .filter(|segment| !segment.is_empty())
            .collect::<Vec<_>>();
        if let ["v1", "books", book_id, "status"] = segments.as_slice() {
            if let Ok(jobs) = self.lock_jobs() {
                if let Some(job) = jobs.get(*book_id) {
                    return json_response(200, job);
                }
            }
        }
        if let ["v1", "books", book_id, ..] = segments.as_slice() {
            return self.delegate_to_book(book_id, request);
        }
        if segments.as_slice() == ["v1", "reader-sessions"] && request.method == "POST" {
            let book_id = serde_json::from_slice::<Value>(&request.body)
                .ok()
                .and_then(|value| value.get("book_id")?.as_str().map(str::to_owned));
            return book_id.map_or_else(
                || error_response(400, "bad_request", "book_id is required", false),
                |book_id| self.delegate_to_book(&book_id, request),
            );
        }
        if matches!(segments.first(), Some(&"v1"))
            && matches!(segments.get(1), Some(&"reader-sessions") | Some(&"exports"))
        {
            return self.delegate_until_handled(request);
        }
        error_response(404, "not_found", "route not found", false)
    }

    fn upload(self: &Arc<Self>, request: &HttpRequest) -> HttpResponse {
        if request.body.is_empty() {
            return error_response(400, "bad_request", "EPUB body is empty", false);
        }
        let profile = request
            .headers
            .get("x-codexia-profile")
            .map(String::as_str)
            .unwrap_or("standard");
        if !matches!(profile, "basic" | "standard" | "deep") {
            return error_response(400, "bad_request", "invalid processing profile", false);
        }
        if self.config.analyzer_command.is_none() {
            return error_response(
                409,
                "analyzer_required",
                "book compilation requires an analyzer command",
                false,
            );
        }
        let source_hash =
            hex_encode(pagelet::core::ContentHash::from_bytes(&request.body).as_bytes());
        let book_id = source_hash.chars().take(16).collect::<String>();
        if self
            .lock_jobs()
            .ok()
            .is_some_and(|jobs| jobs.contains_key(&book_id))
        {
            return json_response(
                202,
                json!({"book_id":book_id,"status_href":format!("/v1/books/{book_id}/status")}),
            );
        }
        let book_dir = self.library_dir.join("books").join(&book_id);
        if let Err(error) = fs::create_dir_all(&book_dir)
            .and_then(|()| fs::write(book_dir.join("source.epub"), &request.body))
        {
            return error_response(500, "storage_error", error.to_string(), true);
        }
        if let Ok(mut jobs) = self.lock_jobs() {
            jobs.insert(
                book_id.clone(),
                BookJob {
                    book_id: book_id.clone(),
                    state: JobState::Queued,
                    progress_basis_points: 0,
                    current_stage: Some("parse".to_owned()),
                    error: None,
                },
            );
        }
        let api = Arc::clone(self);
        let profile = profile.to_owned();
        let job_book_id = book_id.clone();
        thread::spawn(move || api.compile_job(job_book_id, book_dir, profile));
        json_response(
            202,
            json!({"book_id":book_id,"status_href":format!("/v1/books/{book_id}/status")}),
        )
    }

    fn compile_job(self: Arc<Self>, book_id: String, book_dir: PathBuf, profile: String) {
        self.update_job(&book_id, JobState::Processing, 1_000, Some("parse"), None);
        let package_dir = book_dir.join("package");
        let mut command = Command::new(&self.config.compiler_executable);
        command
            .arg("compile")
            .arg(book_dir.join("source.epub"))
            .arg("--profile")
            .arg(profile)
            .arg("--out")
            .arg(&package_dir);
        if let Some(analyzer) = &self.config.analyzer_command {
            command.arg("--analyzer-command").arg(analyzer);
        }
        let result = command.output();
        match result {
            Ok(output) if output.status.success() => match self.register_package(&package_dir) {
                Ok(_) => {
                    self.update_job(&book_id, JobState::Ready, 10_000, None, None);
                    self.record_compile_cost();
                    self.notify(
                        "book.processing.completed",
                        &book_id,
                        json!({"state":"ready"}),
                    );
                }
                Err(error) => self.fail_job(&book_id, error),
            },
            Ok(output) => self.fail_job(
                &book_id,
                String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            ),
            Err(error) => self.fail_job(&book_id, error.to_string()),
        }
    }

    fn fail_job(&self, book_id: &str, error: String) {
        self.update_job(book_id, JobState::Failed, 10_000, None, Some(error.clone()));
        self.notify(
            "book.processing.failed",
            book_id,
            json!({"state":"failed","error":error}),
        );
    }

    fn update_job(
        &self,
        book_id: &str,
        state: JobState,
        progress: u16,
        stage: Option<&str>,
        error: Option<String>,
    ) {
        if let Ok(mut jobs) = self.lock_jobs() {
            jobs.insert(
                book_id.to_owned(),
                BookJob {
                    book_id: book_id.to_owned(),
                    state,
                    progress_basis_points: progress,
                    current_stage: stage.map(str::to_owned),
                    error,
                },
            );
        }
    }

    fn delegate_to_book(&self, book_id: &str, request: &HttpRequest) -> HttpResponse {
        match self.lock_books() {
            Ok(books) => books.get(book_id).map_or_else(
                || error_response(404, "not_found", "book is not ready", true),
                |runtime| runtime.dispatch(&request.method, &request.target, &request.body),
            ),
            Err(error) => error_response(error.status, error.code, error.message, true),
        }
    }

    fn delegate_until_handled(&self, request: &HttpRequest) -> HttpResponse {
        let Ok(books) = self.lock_books() else {
            return error_response(500, "internal_error", "book lock is poisoned", true);
        };
        for runtime in books.values() {
            let response = runtime.dispatch(&request.method, &request.target, &request.body);
            if response.status() != 404 {
                return response;
            }
        }
        error_response(404, "not_found", "resource not found", false)
    }

    fn record_usage(&self, request: &HttpRequest, response: &HttpResponse) {
        let operation = operation_name(&request.method, &request.target);
        if let Ok(mut usage) = self.lock_usage() {
            usage.request_count = usage.request_count.saturating_add(1);
            usage.input_bytes = usage.input_bytes.saturating_add(request.body.len() as u64);
            usage.output_bytes = usage
                .output_bytes
                .saturating_add(response.body().len() as u64);
            *usage.by_operation.entry(operation.clone()).or_default() += 1;
            if matches!(operation.as_str(), "explain" | "ask" | "reflect") {
                usage.agent_request_count = usage.agent_request_count.saturating_add(1);
                usage.estimated_cost_micros = usage
                    .estimated_cost_micros
                    .saturating_add(self.config.agent_request_cost_micros);
            }
            let _ = self.save_usage(&usage);
        }
    }

    fn record_compile_cost(&self) {
        if let Ok(mut usage) = self.lock_usage() {
            usage.compile_count = usage.compile_count.saturating_add(1);
            usage.estimated_cost_micros = usage
                .estimated_cost_micros
                .saturating_add(self.config.compile_cost_micros);
            let _ = self.save_usage(&usage);
        }
    }

    fn save_usage(&self, usage: &UsageState) -> Result<(), String> {
        let path = self.library_dir.join("usage.json");
        let temporary = self.library_dir.join("usage.json.tmp");
        let mut bytes = serde_json::to_vec_pretty(usage).map_err(|error| error.to_string())?;
        bytes.push(b'\n');
        fs::write(&temporary, bytes)
            .and_then(|()| fs::rename(temporary, path))
            .map_err(|error| error.to_string())
    }

    fn notify(&self, event: &str, book_id: &str, data: Value) {
        let Some(url) = &self.config.webhook_url else {
            return;
        };
        let payload = json!({
            "event": event,
            "book_id": book_id,
            "data": data,
        });
        let url = url.clone();
        thread::spawn(move || {
            // ponytail: curl supplies HTTPS without adding a TLS stack; replace
            // it with an embedded client if Codexia must run without curl.
            let _ = Command::new("curl")
                .arg("--fail")
                .arg("--silent")
                .arg("--show-error")
                .arg("--header")
                .arg("Content-Type: application/json")
                .arg("--data-binary")
                .arg(payload.to_string())
                .arg(url)
                .status();
        });
    }

    fn lock_books(&self) -> Result<MutexGuard<'_, BTreeMap<String, Arc<WebRuntime>>>, ApiError> {
        self.books
            .lock()
            .map_err(|_| ApiError::internal("book lock is poisoned"))
    }

    fn lock_jobs(&self) -> Result<MutexGuard<'_, BTreeMap<String, BookJob>>, ApiError> {
        self.jobs
            .lock()
            .map_err(|_| ApiError::internal("job lock is poisoned"))
    }

    fn lock_usage(&self) -> Result<MutexGuard<'_, UsageState>, ApiError> {
        self.usage
            .lock()
            .map_err(|_| ApiError::internal("usage lock is poisoned"))
    }
}

pub fn serve(
    library_dir: impl AsRef<Path>,
    bind: &str,
    config: PublicApiConfig,
) -> Result<(), String> {
    let api = Arc::new(PublicApi::load(library_dir, config)?);
    serve_http(
        bind,
        "/openapi.json",
        Arc::new(move |request| api.dispatch_request(request)),
    )
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct ApiError {
    status: u16,
    code: &'static str,
    message: String,
    retryable: bool,
}

impl ApiError {
    fn new(status: u16, code: &'static str, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            retryable,
        }
    }

    fn internal(message: impl Into<String>) -> Self {
        Self::new(500, "internal_error", message, true)
    }
}

fn json_response(status: u16, value: impl Serialize) -> HttpResponse {
    HttpResponse::json(status, value)
        .unwrap_or_else(|_| error_response(500, "internal_error", "serialization failed", true))
}

fn error_response(
    status: u16,
    code: &'static str,
    message: impl Into<String>,
    retryable: bool,
) -> HttpResponse {
    json_response_raw(
        status,
        json!({"error":{"code":code,"message":message.into(),"retryable":retryable}}),
    )
}

fn json_response_raw(status: u16, value: Value) -> HttpResponse {
    let body = serde_json::to_vec(&value).unwrap_or_else(|_| b"{}".to_vec());
    HttpResponse::bytes(status, "application/json; charset=utf-8", body)
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

fn operation_name(method: &str, target: &str) -> String {
    let path = target.split('?').next().unwrap_or(target);
    if path.ends_with("/explain") {
        "explain".to_owned()
    } else if path.ends_with("/ask") {
        "ask".to_owned()
    } else if path.ends_with("/reflect") {
        "reflect".to_owned()
    } else if method == "POST" && path == "/v1/books" {
        "compile".to_owned()
    } else {
        format!("{} {}", method.to_ascii_lowercase(), path)
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().fold(
        String::with_capacity(bytes.len() * 2),
        |mut output, byte| {
            use std::fmt::Write;
            let _ = write!(output, "{byte:02x}");
            output
        },
    )
}
