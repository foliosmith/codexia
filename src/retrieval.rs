//! Small lexical evidence ranking; authorization precedes this module.
use std::collections::{BTreeMap, BTreeSet};
pub(crate) fn terms(text: &str) -> BTreeSet<String> {
    const STOP: &[&str] = &[
        "a", "an", "the", "and", "or", "of", "to", "in", "on", "at", "is", "are", "was", "were",
        "be", "been", "what", "who", "how", "why", "where", "when", "which", "does", "do", "did",
        "have", "has", "had", "it", "this", "that", "with", "for", "from", "as", "i", "you", "we",
        "they", "he", "she", "its", "my", "me", "by", "about", "please",
    ];
    let mut terms = BTreeSet::new();
    for word in text
        .split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
    {
        if word.chars().any(|c| matches!(c as u32, 0x3400..=0x9fff)) {
            let chars = word.chars().collect::<Vec<_>>();
            for pair in chars.windows(2) {
                terms.insert(pair.iter().collect());
            }
        } else if word.len() > 1 && !STOP.contains(&word.as_str()) {
            terms.insert(word);
        }
    }
    terms
}
// ponytail: linear lexical ranking misses synonyms; add semantic retrieval only when trial evidence warrants it.
pub(crate) fn rank<'a>(query: &str, texts: impl Iterator<Item = &'a str>) -> Vec<usize> {
    let query = terms(query);
    let documents = texts.map(terms).collect::<Vec<_>>();
    let mut counts = BTreeMap::new();
    for document in &documents {
        for term in document.intersection(&query) {
            *counts.entry(term).or_insert(0usize) += 1;
        }
    }
    let mut scored = documents
        .iter()
        .enumerate()
        .filter_map(|(index, document)| {
            let score = document
                .intersection(&query)
                .map(|term| 100000 / (counts[term] + 1))
                .sum::<usize>();
            (score > 0).then_some((index, score))
        })
        .collect::<Vec<_>>();
    scored.sort_by_key(|(index, score)| (std::cmp::Reverse(*score), *index));
    scored.into_iter().map(|(index, _)| index).collect()
}
