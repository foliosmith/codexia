#!/usr/bin/env node

import { writeFileSync } from "node:fs";

const chunks = [];
for await (const chunk of process.stdin) chunks.push(chunk);
const request = JSON.parse(Buffer.concat(chunks).toString("utf8"));

if (["chapter_analysis", "chapter_reanalysis"].includes(request.task)) {
  for (const name of ["concepts", "claims", "argument_flow", "difficult_passages", "entities"]) {
    if (!request.output_schema[name]?.[0] || typeof request.output_schema[name][0] !== "object") {
      throw new Error(`strict provider requires an item schema for ${name}`);
    }
  }
  const chapter = request.context?.chapter || { chapter_id: request.chapter_id, title: request.chapter_title };
  const label = chapter.title || chapter.chapter_id;
  const block = request.context?.chapter.blocks[0];
  process.stdout.write(JSON.stringify({
    summary: {
      one_sentence: `Offline registration baseline for ${label}.`,
      short: `Deterministic placeholder summary for ${label}.`,
      deep: `This placeholder proves the package and Studio registration path; it is not a quality judgment of ${label}.`,
      role_in_book: "Preserves chapter order for the offline registration baseline.",
    },
    key_ideas: [],
    concepts: process.env.CODEXIA_TEST_GROUNDING_MODE ? [{
      concept_id: `concept-${chapter.chapter_id}`, name: "Test inference", aliases: [],
      definition_in_this_book: "An interpretation with contextual support.",
      source_refs: [{ block_id: block.block_id, start_char: 0, end_char: Array.from(block.text).length, text_fingerprint: block.text_fingerprint }],
    }] : [],
    claims: [],
    argument_flow: [],
    difficult_passages: [],
    entities: [],
  }));
} else if (request.task === "book_synthesis") {
  const title = request.context.book_title || "Untitled Book";
  const chapters = request.context.chapter_analyses;
  const ids = chapters.map((chapter) => chapter.chapter_id);
  const shortPath = ids.slice(0, Math.max(1, Math.ceil(ids.length / 3)));
  process.stdout.write(JSON.stringify({
    book_map: {
      central_question: `What should a real analyzer determine about ${title}?`,
      thesis: "Offline registration output is structural evidence only, not an analytical baseline.",
      chapter_roles: chapters.map((chapter) => ({
        chapter_id: chapter.chapter_id,
        role: "Preserves this chapter in the registration package.",
        depends_on_chapter_ids: [],
      })),
      reading_paths: [
        { path_id: "deep", kind: "deep", title: "Deep", description: "All analyzed chapters.", chapter_ids: ids },
        { path_id: "fast", kind: "fast", title: "Fast", description: "First structural sample.", chapter_ids: shortPath },
        { path_id: "selective", kind: "selective", title: "Selective", description: "Last structural sample.", chapter_ids: ids.slice(-1) },
      ],
      difficulty_map: ids.map((chapter_id) => ({ chapter_id, level: "intermediate", reason: "Not scored by the offline registration analyzer." })),
      key_chapter_ids: ids.slice(0, 1),
    },
    concepts: process.env.CODEXIA_TEST_GROUNDING_MODE ? [{
      concept_id: "test-inference", name: "Test inference", aliases: [],
      definition_in_this_book: "An interpretation with contextual support.",
      appearances: process.env.CODEXIA_TEST_GROUNDING_MODE === "inferred-refs" ? chapters[0].concepts[0].source_refs : [],
      related_concepts: [], importance: 50,
      grounding: process.env.CODEXIA_TEST_GROUNDING_MODE === "inferred-refs" ? "inferred" : "grounded",
    }] : [],
    claims: [],
    entities: [],
    checkpoints: ids.map((chapter_id) => ({
      checkpoint_id: `checkpoint-${chapter_id}`,
      chapter_id,
      summary: "Offline registration checkpoint.",
      must_understand: ["Package structure", "Chapter identity", "Registration boundary"],
      recall_questions: [{ question_id: `recall-${chapter_id}`, prompt: "Was this chapter registered?", expected_points: ["Yes"] }],
      reflection_questions: [{ question_id: `reflection-${chapter_id}`, prompt: "What remains for online analysis?", expected_points: ["Content quality"] }],
      flashcards: [{ flashcard_id: `flashcard-${chapter_id}`, front: "Registration baseline", back: "Structural evidence only.", concept_ids: [] }],
      source_refs: [],
      grounding: "inferred",
    })),
    book_reflection_questions: [{
      question_id: "book-reflection-registration",
      prompt: "What evidence is still required before Beta?",
      expected_points: ["Real online analysis"],
    }],
  }));
} else if (["explain_passage", "ask_book", "reflect_on_answer"].includes(request.task)) {
  if (process.env.CODEXIA_TEST_CONTEXT) writeFileSync(process.env.CODEXIA_TEST_CONTEXT, JSON.stringify(request));
  const block = request.context.selected_block || request.context.nearby_blocks[0];
  const source = request.input.source_ref || (block && {
    block_id: block.block_id,
    start_char: 0,
    end_char: Array.from(block.text).length,
    text_fingerprint: block.text_fingerprint,
  });
  const values = { passage: request.input.selected_text || "", explanation: "Only the supplied reading context is available.", simplified: "Supplied context only.", why_it_matters: "It anchors the current passage.", question: request.input.question || "", answer: "The unread ending is unavailable in the supplied context.", feedback: "Recorded answer.", score_basis_points: 8000, expected_points: request.input.expected_points || [] };
  const content = Object.fromEntries(Object.keys(request.output_schema.cards[0].content).map((key) => [key, values[key]]));
  process.stdout.write(JSON.stringify({ cards: [{
    card_type: request.task === "reflect_on_answer" ? "reflection" : request.task === "ask_book" ? "answer" : "explanation",
    card_id: `fixture-${request.task}`,
    title: "Deterministic HTTP fixture",
    content,
    source_refs: source ? [source] : [],
    confidence_basis_points: 8000,
    grounding: source ? "grounded" : "inferred",
    spoiler_status: request.spoiler_boundary?.mode === "full_book" ? "full_book_allowed" : "within_boundary",
    follow_up_actions: [],
  }] }));
} else {
  throw new Error(`unsupported registration task: ${request.task}`);
}
