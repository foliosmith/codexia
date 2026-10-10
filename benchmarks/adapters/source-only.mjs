#!/usr/bin/env node
import assert from 'node:assert/strict';

const chunks = [];
for await (const chunk of process.stdin) chunks.push(chunk);
const request = JSON.parse(Buffer.concat(chunks));
const unavailable = 'Source-only baseline: no precomputed interpretation.';
let output;
if (['chapter_analysis', 'chapter_reanalysis'].includes(request.task)) {
  output = {
    summary: Object.fromEntries(['one_sentence', 'short', 'deep', 'role_in_book'].map(key => [key, unavailable])),
    key_ideas: [], concepts: [], claims: [], argument_flow: [], difficult_passages: [], entities: [],
  };
} else {
  assert.equal(request.task, 'book_synthesis');
  const ids = request.context.chapter_analyses.map(chapter => chapter.chapter_id);
  output = {
    book_map: {
      central_question: unavailable, thesis: unavailable,
      chapter_roles: ids.map(chapter_id => ({ chapter_id, role: unavailable, depends_on_chapter_ids: [] })),
      reading_paths: ['deep', 'fast', 'selective'].map(kind => ({ path_id: kind, kind, title: kind, description: unavailable, chapter_ids: ids })),
      difficulty_map: ids.map(chapter_id => ({ chapter_id, level: 'intermediate', reason: unavailable })),
      key_chapter_ids: ids.slice(0, 1),
    },
    concepts: [], claims: [], entities: [],
    checkpoints: ids.map(chapter_id => ({
      checkpoint_id: `source-${chapter_id}`, chapter_id, summary: unavailable,
      must_understand: ['Consult the source.', 'Check conditions.', 'Do not infer unread content.'],
      recall_questions: [{ question_id: `recall-${chapter_id}`, prompt: 'Restate this chapter using its source.', expected_points: [] }],
      reflection_questions: [{ question_id: `reflect-${chapter_id}`, prompt: 'Which source supports your retelling?', expected_points: [] }],
      flashcards: [{ flashcard_id: `source-${chapter_id}`, front: 'Consult the source.', back: unavailable, concept_ids: [] }],
      source_refs: [], grounding: 'inferred',
    })),
    book_reflection_questions: [],
  };
}
process.stdout.write(JSON.stringify(output));
