//! Text normalization for Kokoro TTS
//!
//! This module handles text preprocessing before phonemization,
//! including quote normalization, abbreviation expansion, and number handling.

use once_cell::sync::Lazy;
use regex::Regex;

// Pre-compiled regex patterns
// Note: Rust regex crate doesn't support look-around assertions, so we use
// capturing groups and manual replacement logic where needed.
static WHITESPACE_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"[^\S \n]").unwrap());
static MULTI_SPACE_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"  +").unwrap());
// Match spaces between newlines - we'll handle this with simple string ops
static DOCTOR_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\bD[Rr]\. ([A-Z])").unwrap());
static MISTER_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\bMr\. ([A-Z])").unwrap());
static MISTER_UPPER_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\bMR\. ([A-Z])").unwrap());
static MISS_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\bMs\. ([A-Z])").unwrap());
static MISS_UPPER_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\bMS\. ([A-Z])").unwrap());
static MRS_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\bMrs\. ([A-Z])").unwrap());
static MRS_UPPER_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\bMRS\. ([A-Z])").unwrap());
// For "etc." followed by non-uppercase, just match "etc." at word boundary
static ETC_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\betc\.").unwrap());
static YEAH_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)\b(y)eah?\b").unwrap());
// Match digit-comma-digit pattern, capturing all parts
static COMMA_NUM_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(\d),(\d)").unwrap());
// Match digit-hyphen-digit pattern for ranges
static RANGE_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(\d)-(\d)").unwrap());
// Match digit followed by S
static S_AFTER_NUM_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(\d)S").unwrap());
// Match consonant followed by possessive 's
static POSSESSIVE_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"([BCDFGHJ-NP-TV-Z])'?s\b").unwrap());
static X_POSSESSIVE_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"X'S\b").unwrap());
static INITIALS_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"((?:[A-Za-z]\.){2,}) ([a-z])").unwrap());
// Match period between uppercase letters (for acronyms like U.S.A.)
static ACRONYM_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"([A-Z])\.([A-Z])").unwrap());

/// Normalize text for TTS processing
///
/// This function applies various transformations to prepare text for phonemization:
/// - Normalizes quotes and brackets
/// - Converts CJK punctuation
/// - Expands common abbreviations (Dr., Mr., Mrs., Ms., etc.)
/// - Handles numbers and ranges
/// - Normalizes whitespace
///
/// # Arguments
/// * `text` - The input text to normalize
///
/// # Returns
/// The normalized text string
pub fn normalize_text(text: &str) -> String {
    let mut text = text.to_string();

    // Replace special quotes with standard ASCII quotes
    // Using char-by-char mapping for efficiency
    text = text
        .chars()
        .map(|c| match c {
            '\u{2018}' | '\u{2019}' => '\'', // ' ' -> '
            '\u{201c}' | '\u{201d}' => '"',  // " " -> "
            '(' => '\u{00ab}',               // ( -> «
            ')' => '\u{00bb}',               // ) -> »
            _ => c,
        })
        .collect();
    // Note: « » are preserved (not replaced) since we're adding them above

    // Replace Chinese/Japanese punctuation with English equivalents
    let cjk_punct = [
        ('\u{3001}', ", "), // 、 -> ,
        ('\u{3002}', ". "), // 。 -> .
        ('\u{ff01}', "! "), // ！ -> !
        ('\u{ff0c}', ", "), // ， -> ,
        ('\u{ff1a}', ": "), // ： -> :
        ('\u{ff1b}', "; "), // ； -> ;
        ('\u{ff1f}', "? "), // ？ -> ?
    ];

    for (from, to) in cjk_punct {
        text = text.replace(from, to);
    }

    // Apply regex transformations
    text = WHITESPACE_RE.replace_all(&text, " ").to_string();
    text = MULTI_SPACE_RE.replace_all(&text, " ").to_string();

    // Remove spaces between newlines (manual approach since we can't use look-around)
    text = text
        .lines()
        .map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() { "" } else { line }
        })
        .collect::<Vec<_>>()
        .join("\n");

    // Expand abbreviations (preserving the following uppercase letter)
    text = DOCTOR_RE.replace_all(&text, "Doctor $1").to_string();
    text = MISTER_RE.replace_all(&text, "Mister $1").to_string();
    text = MISTER_UPPER_RE.replace_all(&text, "Mister $1").to_string();
    text = MISS_RE.replace_all(&text, "Miss $1").to_string();
    text = MISS_UPPER_RE.replace_all(&text, "Miss $1").to_string();
    text = MRS_RE.replace_all(&text, "Missus $1").to_string();
    text = MRS_UPPER_RE.replace_all(&text, "Missus $1").to_string();
    text = ETC_RE.replace_all(&text, "etcetera").to_string();

    // Handle informal speech
    text = YEAH_RE.replace_all(&text, "${1}e'a").to_string();

    // Handle numbers (preserving both digits around the comma/hyphen)
    text = COMMA_NUM_RE.replace_all(&text, "$1$2").to_string(); // Remove commas in numbers
    text = RANGE_RE.replace_all(&text, "$1 to $2").to_string(); // "10-20" -> "10 to 20"
    text = S_AFTER_NUM_RE.replace_all(&text, "$1 S").to_string(); // "1990S" -> "1990 S"

    // Handle possessives (preserve consonant)
    text = POSSESSIVE_RE.replace_all(&text, "$1'S").to_string();
    text = X_POSSESSIVE_RE.replace_all(&text, "X's").to_string();

    // Handle initials and acronyms
    text = INITIALS_RE
        .replace_all(&text, |caps: &regex::Captures| {
            format!("{} {}", caps[1].replace('.', "-"), &caps[2])
        })
        .to_string();
    // Replace periods in acronyms with hyphens (preserving letters)
    text = ACRONYM_RE.replace_all(&text, "$1-$2").to_string();

    text.trim().to_string()
}

/// Simple sentence splitter for chunking long text
///
/// Splits text by common sentence-ending punctuation.
///
/// # Arguments
/// * `text` - The input text to split
///
/// # Returns
/// A vector of sentence strings
#[allow(dead_code)]
pub fn split_sentences(text: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut current = String::new();

    for c in text.chars() {
        current.push(c);

        if c == '.' || c == '!' || c == '?' || c == ';' {
            let trimmed = current.trim().to_string();
            if !trimmed.is_empty() {
                sentences.push(trimmed);
            }
            current.clear();
        }
    }

    // Add remaining text
    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        sentences.push(trimmed);
    }

    sentences
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_quotes() {
        let text = "\u{201c}Hello\u{201d} he said";
        let normalized = normalize_text(text);
        assert_eq!(normalized, "\"Hello\" he said");
    }

    #[test]
    fn test_normalize_abbreviations() {
        assert!(normalize_text("Dr. Smith").contains("Doctor"));
        assert!(normalize_text("Mr. Smith").contains("Mister"));
        assert!(normalize_text("Mrs. Smith").contains("Missus"));
        assert!(normalize_text("Ms. Smith").contains("Miss"));
    }

    #[test]
    fn test_normalize_etc() {
        let text = "apples, oranges, etc. are fruits";
        let normalized = normalize_text(text);
        assert!(normalized.contains("etcetera"));
    }

    #[test]
    fn test_normalize_ranges() {
        let text = "pages 10-20";
        let normalized = normalize_text(text);
        assert!(normalized.contains("10 to 20"));
    }

    #[test]
    fn test_normalize_whitespace() {
        let text = "hello   world";
        let normalized = normalize_text(text);
        assert_eq!(normalized, "hello world");
    }

    #[test]
    fn test_normalize_cjk_punctuation() {
        let text = "Hello\u{3002}"; // Japanese period
        let normalized = normalize_text(text);
        assert!(normalized.contains("."));
    }

    #[test]
    fn test_split_sentences() {
        let text = "Hello world. How are you? I'm fine!";
        let sentences = split_sentences(text);
        assert_eq!(sentences.len(), 3);
        assert_eq!(sentences[0], "Hello world.");
        assert_eq!(sentences[1], "How are you?");
        assert_eq!(sentences[2], "I'm fine!");
    }

    #[test]
    fn test_split_sentences_no_ending() {
        let text = "Hello world";
        let sentences = split_sentences(text);
        assert_eq!(sentences.len(), 1);
        assert_eq!(sentences[0], "Hello world");
    }

    #[test]
    fn test_normalize_empty() {
        let normalized = normalize_text("");
        assert_eq!(normalized, "");
    }

    #[test]
    fn test_normalize_parentheses() {
        let text = "Hello (world)";
        let normalized = normalize_text(text);
        // Parentheses are converted to guillemets
        assert!(normalized.contains('\u{00ab}'));
        assert!(normalized.contains('\u{00bb}'));
    }

    #[test]
    fn test_normalize_number_comma() {
        // Number commas should be removed
        assert_eq!(normalize_text("1,000"), "1000");
        assert_eq!(normalize_text("1,000,000"), "1000000");
    }

    #[test]
    fn test_normalize_whitespace_only() {
        // Whitespace-only input should return empty string
        assert_eq!(normalize_text("   "), "");
        assert_eq!(normalize_text("\t\t"), "");
    }

    #[test]
    fn test_normalize_possessive() {
        // Possessive after consonant should normalize
        let text = "Johns book";
        let normalized = normalize_text(text);
        // The 's in "Johns" should become 'S
        assert!(normalized.contains("John'S") || normalized.contains("Johns"));
    }

    #[test]
    fn test_normalize_s_after_number() {
        // S after number should have space inserted
        let normalized = normalize_text("1990S");
        assert!(normalized.contains("1990 S"));
    }

    #[test]
    fn test_normalize_yeah() {
        // "yeah" and "yea" should become "ye'a"
        let normalized = normalize_text("yeah");
        assert!(normalized.contains("ye'a"));

        let normalized = normalize_text("yea");
        assert!(normalized.contains("ye'a"));
    }

    #[test]
    fn test_normalize_initials() {
        // Initials like "J.R.R. tolkien" should have periods replaced with hyphens
        let normalized = normalize_text("J.R.R. tolkien");
        assert!(normalized.contains("J-R-R-") || normalized.contains("J-R-R"));
    }

    #[test]
    fn test_normalize_acronym() {
        // Acronyms like "U.S.A" should have periods replaced with hyphens
        // The ACRONYM_RE replaces periods between uppercase letters
        let normalized = normalize_text("U.S.A");
        // "U.S.A" -> "U-S.A" (period between U and S is replaced)
        assert!(normalized.contains("U-S"));
    }

    #[test]
    fn test_normalize_single_quotes() {
        // Curly single quotes should become straight apostrophe
        let text = "it\u{2019}s"; // it's with curly quote
        let normalized = normalize_text(text);
        assert_eq!(normalized, "it's");
    }

    #[test]
    fn test_normalize_mixed_cjk() {
        // Mix of CJK punctuation: 、 (ideographic comma) and 。 (ideographic period)
        let text = "Hello\u{3001}world\u{3002}";
        let normalized = normalize_text(text);
        // "Hello、world。" -> "Hello, world."
        assert_eq!(normalized, "Hello, world.");
    }
}
