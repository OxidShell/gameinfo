/// Normalize a game title for fuzzy comparison.
///
/// Lowercases, removes leading articles, replaces punctuation with spaces,
/// collapses whitespace.
pub(crate) fn normalize(s: &str) -> String {
    let lower = s.to_lowercase();
    let no_article = strip_leading_article(&lower);
    let spaced: String = no_article
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' {
                c
            } else {
                ' '
            }
        })
        .collect();
    spaced.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn strip_leading_article(s: &str) -> &str {
    for prefix in ["the ", "a ", "an "] {
        if let Some(rest) = s.strip_prefix(prefix) {
            return rest;
        }
    }
    s
}

const STOP_WORDS: &[&str] = &[
    "the", "a", "an", "of", "in", "on", "at", "to", "for", "and", "or", "but", "is", "was",
];

/// Split a normalized title into significant tokens (stop words removed).
pub(crate) fn tokenize(s: &str) -> Vec<String> {
    normalize(s)
        .split_whitespace()
        .filter(|w| !STOP_WORDS.contains(w) && w.len() > 1)
        .map(String::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_article() {
        assert_eq!(normalize("The Witcher 3"), "witcher 3");
        assert_eq!(normalize("A Short Hike"), "short hike");
    }

    #[test]
    fn handles_punctuation() {
        assert_eq!(normalize("Nier: Automata"), "nier automata");
        assert_eq!(normalize("DOOM (2016)"), "doom 2016");
    }
}
