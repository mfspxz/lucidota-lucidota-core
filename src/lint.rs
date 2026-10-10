//! Hedge-phrase lint — the 2026-03-07 standing list, as a scanner.
//!
//! Banned phrases (his words, verified from the "Anti-bullshit enforcement" thread):
//! "should now", "ought to", "likely", "appears to be", "probably worked",
//! "it seems", "this means it is running", "you should see".
//! Replacement vocabulary: what I verified / what I did not verify /
//! what evidence exists / what remains uncertain.

/// (banned phrase, what to say instead)
pub const BANNED: &[(&str, &str)] = &[
    ("should now", "state what was verified, not what should happen next"),
    ("ought to", "state what was verified, not what ought to follow"),
    ("likely", "say what evidence exists and what remains uncertain"),
    ("appears to be", "say what you verified or that it is unknown"),
    ("probably worked", "report observed state — did it work or is it unknown?"),
    ("it seems", "say what you verified or that it is unknown"),
    ("this means it is running", "report observed state — is it running or unknown?"),
    ("you should see", "report observed state, not what the reader should see"),
];

/// One banned-phrase hit in scanned text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LintHit {
    pub phrase: String,
    pub replacement: String,
    /// 1-based line number.
    pub line: usize,
    /// The offending line, trimmed.
    pub context: String,
}

/// Scan `text` for banned hedge phrases. Case-insensitive, substring match per line.
pub fn lint_text(text: &str) -> Vec<LintHit> {
    let mut hits = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let lower = line.to_lowercase();
        for (phrase, replacement) in BANNED {
            if lower.contains(phrase) {
                hits.push(LintHit {
                    phrase: phrase.to_string(),
                    replacement: replacement.to_string(),
                    line: i + 1,
                    context: line.trim().to_string(),
                });
            }
        }
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_banned_phrase_is_caught() {
        for (phrase, _) in BANNED {
            let text = format!("the deploy {} fine", phrase);
            let hits = lint_text(&text);
            assert!(!hits.is_empty(), "missed banned phrase: {}", phrase);
            assert_eq!(hits[0].line, 1);
        }
    }

    #[test]
    fn matching_is_case_insensitive() {
        let hits = lint_text("It SEEMS to be working. It LIKELY passed.");
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn clean_text_has_no_hits() {
        let text = "What I verified: cargo test ran 34 tests, all passed.\nWhat remains uncertain: GPU load under voice cloning.";
        assert!(lint_text(text).is_empty());
    }

    #[test]
    fn replacement_guidance_is_attached() {
        let hits = lint_text("it should now be running");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].phrase, "should now");
        assert!(!hits[0].replacement.is_empty());
    }

    #[test]
    fn line_numbers_are_one_based() {
        let hits = lint_text("line one is clean\nline two likely has an issue");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].line, 2);
        assert!(hits[0].context.contains("likely"));
    }

    #[test]
    fn multiple_hits_on_one_line_all_reported() {
        let hits = lint_text("it seems it likely worked");
        assert_eq!(hits.len(), 2);
    }
}
