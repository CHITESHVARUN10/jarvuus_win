pub fn final_chunk_start(
    length: usize,
    failed: bool,
    chunks: u64,
    last_end: usize,
    overlap: usize,
) -> usize {
    if failed || chunks == 0 {
        0
    } else {
        last_end.saturating_sub(overlap).min(length)
    }
}

pub fn stitch(session: &mut String, addition: &str) {
    if addition.is_empty() {
        return;
    }
    let addition = addition.trim();
    if addition.is_empty() {
        return;
    }
    if !session.is_empty() && !session.ends_with(' ') && !session.ends_with('\n') {
        session.push(' ');
    }
    session.push_str(addition);
}

fn norm_word(word: &str) -> String {
    let lower = word.to_ascii_lowercase();
    lower
        .trim_matches(|character: char| !character.is_alphanumeric())
        .to_string()
}

fn is_stopword(norm: &str) -> bool {
    matches!(
        norm,
        "the"
            | "a"
            | "an"
            | "and"
            | "or"
            | "so"
            | "but"
            | "of"
            | "to"
            | "in"
            | "on"
            | "is"
            | "it"
            | "we"
            | "you"
            | "for"
            | "as"
            | "at"
            | "be"
            | "are"
            | "was"
            | "by"
            | "with"
            | "that"
            | "this"
            | "from"
            | "have"
            | "has"
            | "had"
            | "will"
            | "would"
            | "can"
            | "do"
            | "does"
            | "did"
            | "not"
            | "no"
            | "if"
            | "then"
            | "okay"
            | "ok"
    )
}

/// Strip any duplicated tail of `prev` from the head of `next` (overlap dedupe).
pub fn dedupe_merge(prev_tail: Option<&str>, next: &str) -> String {
    let Some(tail) = prev_tail else {
        return next.trim().to_string();
    };
    let tail = tail.trim();
    let next = next.trim();
    if tail.is_empty() || next.is_empty() {
        return next.to_string();
    }

    let tail_words: Vec<&str> = tail.split_whitespace().collect();
    let next_words: Vec<&str> = next.split_whitespace().collect();
    if tail_words.is_empty() || next_words.is_empty() {
        return next.to_string();
    }

    let tail_norm: Vec<String> = tail_words.iter().map(|word| norm_word(word)).collect();
    let next_norm: Vec<String> = next_words.iter().map(|word| norm_word(word)).collect();

    let tail_norm_filtered: Vec<&str> = tail_norm
        .iter()
        .map(String::as_str)
        .filter(|word| !word.is_empty())
        .collect();
    let next_norm_filtered: Vec<&str> = next_norm
        .iter()
        .map(String::as_str)
        .filter(|word| !word.is_empty())
        .collect();
    if !tail_norm_filtered.is_empty()
        && tail_norm_filtered.len() <= next_norm_filtered.len()
        && next_norm_filtered[..tail_norm_filtered.len()] == tail_norm_filtered[..]
    {
        let skip = tail_words.len();
        if skip >= next_words.len() {
            return String::new();
        }
        return next_words[skip..].join(" ");
    }

    let head_limit = next_words.len().min(12);
    for take in (1..=tail_words.len().min(12)).rev() {
        if take >= 3 {
            // meaningful overlap
        } else if take == 2 {
            let first = tail_norm[tail_norm.len() - 2].as_str();
            let second = tail_norm[tail_norm.len() - 1].as_str();
            if first.is_empty()
                || second.is_empty()
                || is_stopword(first)
                || is_stopword(second)
                || first.len() < 3
                || second.len() < 3
            {
                continue;
            }
        } else {
            let word = tail_norm[tail_norm.len() - 1].as_str();
            if word.is_empty() || word.len() < 6 || is_stopword(word) {
                continue;
            }
        }

        let suffix_norm = &tail_norm[tail_norm.len() - take..];
        for start in 0..head_limit {
            if start + take > next_words.len() {
                break;
            }
            if take == 1 && start > 1 {
                continue;
            }
            if start > 3 {
                continue;
            }
            let window_norm = &next_norm[start..start + take];
            let matches = window_norm
                .iter()
                .zip(suffix_norm.iter())
                .all(|(a, b)| !a.is_empty() && !b.is_empty() && a == b);
            if matches {
                return next_words[start + take..].join(" ");
            }
        }
    }

    next.to_string()
}

pub fn last_words(text: &str, n: usize) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    let start = words.len().saturating_sub(n);
    words[start..].join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delayed_chunk_failure_retries_the_complete_audio_including_beginning() {
        let length = 16_000 * 900;
        assert_eq!(
            final_chunk_start(length, false, 149, 16_000 * 894, 16_000),
            16_000 * 893
        );
        assert_eq!(
            final_chunk_start(length, true, 149, 16_000 * 894, 16_000),
            0
        );
        assert_eq!(final_chunk_start(length, false, 0, 0, 16_000), 0);
    }

    #[test]
    fn dedupe_exact_prefix() {
        let tail = "timeline and key";
        let next = "timeline and key milestones we need to hit";
        assert_eq!(dedupe_merge(Some(tail), next), "milestones we need to hit");
    }

    #[test]
    fn dedupe_long_overlap_more_than_3_words() {
        let tail = "the new project timeline and key milestones";
        let next = "timeline and key milestones we need to hit by the end of the week";
        assert_eq!(
            dedupe_merge(Some(tail), next),
            "we need to hit by the end of the week"
        );
    }

    #[test]
    fn dedupe_with_leading_filler() {
        let tail = "we have six weeks to deliver";
        let next = "so we have six weeks to deliver something tangible";
        assert_eq!(dedupe_merge(Some(tail), next), "something tangible");
    }

    #[test]
    fn dedupe_no_overlap_keeps_both() {
        let tail = "completely different sentence";
        let next = "something entirely new";
        assert_eq!(dedupe_merge(Some(tail), next), "something entirely new");
    }

    #[test]
    fn dedupe_filler_then_overlap_at_head() {
        let tail = "now we will discuss the action items";
        let next = "okay now we will discuss the action items each member needs";
        assert_eq!(dedupe_merge(Some(tail), next), "each member needs");
    }

    #[test]
    fn dedupe_common_word_inside_head_not_false_positive() {
        let tail = "we need to deliver the prototype";
        let next = "the design and engineering teams are aligned";
        assert_eq!(
            dedupe_merge(Some(tail), next),
            "the design and engineering teams are aligned"
        );
    }

    #[test]
    fn last_words_keeps_n() {
        assert_eq!(last_words("a b c d e", 3), "c d e");
        assert_eq!(last_words("a b", 3), "a b");
    }

    #[test]
    fn stitch_inserts_space() {
        let mut session = String::from("currently");
        stitch(&mut session, "we have being like the UI should look");
        assert_eq!(session, "currently we have being like the UI should look");
        assert!(!session.contains("currentlywe"));
    }

    #[test]
    fn stitch_trim_and_empty() {
        let mut session = String::from("hello");
        stitch(&mut session, "   ");
        assert_eq!(session, "hello");
        stitch(&mut session, " world ");
        assert_eq!(session, "hello world");
    }
}
