#!/usr/bin/env node
const chunks = [];
for await (const chunk of process.stdin) chunks.push(chunk);
const request = JSON.parse(Buffer.concat(chunks));
const blocks = [request.context.selected_block, ...request.context.nearby_blocks].filter(Boolean);
const text = [...new Set(blocks.map(block => block.text))].join('\n');
const task = request.task;
const content = task === 'ask_book' ? { question: request.input.question, answer: text }
  : task === 'explain_passage' ? { passage: request.input.selected_text, explanation: text, simplified: text, why_it_matters: 'Offline source extraction only.' }
    : { feedback: 'Offline extraction does not evaluate the supplied reflection.', score_basis_points: 0, expected_points: request.input.expected_points || [] };
process.stdout.write(JSON.stringify({ cards: [{
  card_type: task === 'ask_book' ? 'answer' : task === 'explain_passage' ? 'explanation' : 'reflection',
  card_id: 'offline-extract', title: 'Offline source extraction', content,
  source_refs: request.evidence_refs,
  confidence_basis_points: 0, grounding: request.evidence_refs.length ? 'grounded' : 'inferred',
  spoiler_status: 'within_boundary', follow_up_actions: [],
}] }));
