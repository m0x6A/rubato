//! The spoken cut of an agent's reply (plan 146 design §4): what is read aloud. The full reply
//! stays in the room thread.

use rubato_protocol::voice::limits::SPOKEN_CUT_MAX_CHARS;

/// What is said when nothing speakable is left.
pub const FALLBACK: &str = "I've posted the details in the room.";

/// Drop fenced code blocks and table rows, strip Markdown marks, and keep whole sentences up to
/// `SPOKEN_CUT_MAX_CHARS`. A first sentence longer than that is cut at a word with `…`.
pub fn spoken_cut(text: &str) -> String {
    let mut kept: Vec<&str> = Vec::new();
    let mut in_fence = false;
    for line in text.lines() {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence || t.starts_with('|') {
            continue;
        }
        kept.push(line);
    }
    let plain: String = kept
        .join(" ")
        .chars()
        .filter(|c| !matches!(c, '`' | '*' | '#' | '_' | '>'))
        .collect();
    let words: Vec<&str> = plain.split_whitespace().collect();
    let flat = words.join(" ");
    if flat.is_empty() {
        return FALLBACK.into();
    }
    let mut out = String::new();
    for sentence in sentences(&flat) {
        let next = if out.is_empty() {
            sentence.to_owned()
        } else {
            format!("{out} {sentence}")
        };
        if next.chars().count() > SPOKEN_CUT_MAX_CHARS {
            break;
        }
        out = next;
    }
    if out.is_empty() {
        // One sentence over the limit: cut it at a word.
        let mut cut = String::new();
        for w in flat.split(' ') {
            if cut.chars().count() + w.chars().count() + 2 > SPOKEN_CUT_MAX_CHARS {
                break;
            }
            if !cut.is_empty() {
                cut.push(' ');
            }
            cut.push_str(w);
        }
        cut.push('…');
        return cut;
    }
    out
}

/// Sentences of flat text: each ends at `.`, `!` or `?` followed by a space, or at the end.
fn sentences(flat: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let b = flat.as_bytes();
    for (i, c) in flat.char_indices() {
        if matches!(c, '.' | '!' | '?') && (i + 1 == b.len() || b[i + 1] == b' ') {
            out.push(flat[start..=i].trim());
            start = i + 1;
        }
    }
    let rest = flat[start..].trim();
    if !rest.is_empty() {
        out.push(rest);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cut_drops_code_and_tables_and_keeps_whole_sentences() {
        let cases: &[(&str, &str)] = &[
            (
                "Because the seam is owned there.",
                "Because the seam is owned there.",
            ),
            (
                "Short answer: yes.\n\n```rust\nfn main() {}\n```\nDetails follow.",
                "Short answer: yes. Details follow.",
            ),
            (
                "It is **fine**.\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n",
                "It is fine.",
            ),
            ("```\nonly code\n```", FALLBACK),
            ("| only | a table |", FALLBACK),
            ("", FALLBACK),
            (
                "Use `plan.ts check`! Then push?",
                "Use plan.ts check! Then push?",
            ),
        ];
        for (input, want) in cases {
            assert_eq!(spoken_cut(input), *want, "for {input:?}");
        }
        println!("checked {} spoken cuts", cases.len());
    }

    #[test]
    fn the_cut_stops_at_a_sentence_end_within_the_limit() {
        let s = "A sentence of about forty characters here. ";
        let text = s.repeat(20);
        let cut = spoken_cut(&text);
        assert!(cut.chars().count() <= SPOKEN_CUT_MAX_CHARS, "{}", cut.len());
        assert!(cut.ends_with('.'), "{cut}");
        assert!(
            cut.chars().count() > SPOKEN_CUT_MAX_CHARS - s.len(),
            "kept as many as fit"
        );
        let long = "word ".repeat(200);
        let cut = spoken_cut(&long);
        assert!(cut.ends_with('…') && cut.chars().count() <= SPOKEN_CUT_MAX_CHARS);
    }
}
