export type Profile = "basic" | "standard" | "deep";
export type SpoilerMode = "read_range" | "current_chapter" | "full_book";
export type Grounding = "grounded" | "inferred";
export type SpoilerStatus = "within_boundary" | "full_book_allowed";

export interface SourceRef {
  block_id: string;
  start_char: number;
  end_char: number;
  text_fingerprint: string;
}

export interface ReaderLocation {
  chapter_id: string;
  block_id?: string | null;
  char_offset?: number | null;
  epub_cfi?: string | null;
}

export interface ReaderState {
  session_id?: string | null;
  current_location: ReaderLocation;
  read_until: ReaderLocation;
  completed_chapter_ids: string[];
  read_coverage?: ReaderLocation[];
  progress_basis_points: number;
}

export interface FollowUpAction {
  action: "open_source" | "ask" | "explain" | "quiz" | "note" | "export" | "reflect";
  label: string;
  payload: Record<string, unknown>;
}

export type ReaderCardType =
  | "book_map" | "chapter_summary" | "explanation" | "answer" | "concept"
  | "claim" | "checkpoint" | "flashcard" | "question" | "reflection" | "export";

export interface ReaderCard<TContent = Record<string, unknown>> {
  card_type: ReaderCardType;
  card_id: string;
  title: string;
  content: TContent;
  source_refs: SourceRef[];
  confidence_basis_points: number;
  grounding: Grounding;
  spoiler_status: SpoilerStatus;
  follow_up_actions: FollowUpAction[];
}

export interface ReaderCardResponse<TCard extends ReaderCard = ReaderCard> {
  cards: TCard[];
  spoiler_boundary: {
    mode: SpoilerMode;
    read_until?: ReaderLocation | null;
    excluded_chapter_ids: string[];
    read_coverage: ReaderLocation[];
  };
}

export interface BookPackage {
  book_id: string;
  format_version: string;
  entries: Array<{ name: string; media_type: string; href: string; source_hash: string }>;
}

export interface ExplanationContent { passage: string; explanation: string; simplified?: string; why_it_matters?: string; }
export interface AnswerContent { question: string; answer: string; }
export interface CheckpointContent { checkpoint_id: string; chapter_id: string; summary: string; must_understand: string[]; }
export interface FeedbackContent { score_basis_points: number; feedback: string; expected_points: string[]; }

export class CodexiaError extends Error {
  readonly status: number;
  readonly code: string;
  readonly retryable: boolean;
}

export class BookAgentClient {
  constructor(options: { baseUrl: string; apiKey: string; fetch?: typeof fetch });
  compile(epub: BodyInit, options?: { profile?: Profile; pollIntervalMs?: number }): Promise<BookPackage>;
  waitUntilReady(bookId: string, pollIntervalMs?: number): Promise<Record<string, unknown>>;
  getBookPackage(bookId: string): Promise<BookPackage>;
  explain(bookId: string, location: { selected_text: string; source_ref: SourceRef }, readerState: ReaderState, options?: { intent?: string; spoilerMode?: SpoilerMode }): Promise<ReaderCardResponse<ReaderCard<ExplanationContent>>>;
  ask(bookId: string, question: string, readerState: ReaderState, options?: { spoilerMode?: SpoilerMode }): Promise<ReaderCardResponse<ReaderCard<AnswerContent>>>;
  checkpoint(bookId: string, chapterId: string, readerState: ReaderState, options?: { spoilerMode?: SpoilerMode }): Promise<ReaderCardResponse<ReaderCard<CheckpointContent>>>;
  reflect(bookId: string, chapterId: string, userAnswer: { checkpoint_id: string; question_id: string; answer: string }, readerState: ReaderState): Promise<ReaderCardResponse<ReaderCard<FeedbackContent>>>;
}
