//! Finds the words a respoken correction replaces.
//!
//! While recording, the user holds the correction key and says the right
//! words again. They usually sound like the wrong words on screen but are
//! spelled differently ("franken term" said again as "FrankenTerm"), so the
//! match is by sound: each run of recent words is compared to the
//! correction by edit distance over its letters and over a consonant
//! skeleton, with word gaps ignored. A small cost for age makes the newest
//! of two equally good spans win. A correction that fits no span well, or
//! fits two different spans about equally, is refused rather than guessed.

/// How many of the newest words a correction may replace.
pub const MATCH_WINDOW_WORDS: usize = 40;

/// Best score a span may have and still be replaced.
const ACCEPT_BELOW: f32 = 0.45;

/// How much better than any other span the best one must score.
const MIN_MARGIN: f32 = 0.1;

/// Score added for a span ending at the start of the window, scaled down
/// to nothing for one ending at the newest word.
const AGE_COST: f32 = 0.05;

/// Spans this close in sound to the best one are repeats of it, not
/// competitors: the newest repeat wins.
const SAME_SOUND: f32 = 0.05;

/// A correction with fewer letters, or a shorter skeleton, is short: "on",
/// "the". It sounds like too many spans, so it needs a near-exact match
/// and a wide margin.
const SHORT_LETTERS: usize = 4;
const SHORT_SKELETON: usize = 3;
const SHORT_ACCEPT_BELOW: f32 = 0.15;
const SHORT_MIN_MARGIN: f32 = 0.3;

/// Where a correction goes: words `start..end` of the text are replaced.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    /// 0 for the same sound; [`ACCEPT_BELOW`] is the limit.
    pub score: f32,
}

/// Why a correction was not placed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Refused {
    /// The correction or the text has no letters.
    Empty,
    /// No span sounds close enough.
    NoMatch { best: f32 },
    /// Two different spans fit about equally well.
    Ambiguous { best: f32, runner_up: f32 },
    /// The correction is too short to place without a clear match.
    TooShort { best: f32 },
}

/// The span of `text` that `correction` most likely replaces, from the
/// last [`MATCH_WINDOW_WORDS`] words. Indices are into all of `text`.
pub fn place<S: AsRef<str>, C: AsRef<str>>(text: &[S], correction: &[C]) -> Result<Span, Refused> {
    place_before(text, text.len(), correction)
}

/// [`place`] as if `text` ended before word `end`: the search covers the
/// [`MATCH_WINDOW_WORDS`] words before it, and age counts back from it.
pub fn place_before<S: AsRef<str>, C: AsRef<str>>(
    text: &[S],
    end: usize,
    correction: &[C],
) -> Result<Span, Refused> {
    let text = &text[..end.min(text.len())];
    let wanted = Key::of(correction);
    if wanted.letters.is_empty() || text.is_empty() {
        return Err(Refused::Empty);
    }
    let n = correction.len();
    let (k_min, k_max) = (n.saturating_sub(2).max(1), n + 2);
    let first = text.len().saturating_sub(MATCH_WINDOW_WORDS);

    let mut spans = Vec::new();
    for start in first..text.len() {
        for end in (start + k_min)..=(start + k_max).min(text.len()) {
            let key = Key::of(&text[start..end]);
            let age = (text.len() - end) as f32 / MATCH_WINDOW_WORDS as f32;
            let score = key.distance(&wanted) + AGE_COST * age;
            spans.push((Span { start, end, score }, key));
        }
    }
    let (best, best_key) = spans
        .iter()
        .min_by(|a, b| a.0.score.total_cmp(&b.0.score))
        .ok_or(Refused::Empty)?;
    let runner_up = spans
        .iter()
        .filter(|(s, _)| s.end <= best.start || s.start >= best.end)
        .filter(|(_, key)| key.distance(best_key) > SAME_SOUND)
        .map(|(s, _)| s.score)
        .min_by(f32::total_cmp);
    let margin = runner_up.map_or(f32::INFINITY, |r| r - best.score);

    let short = wanted.letters.len() < SHORT_LETTERS || wanted.skeleton.len() < SHORT_SKELETON;
    if short && (best.score >= SHORT_ACCEPT_BELOW || margin < SHORT_MIN_MARGIN) {
        return Err(Refused::TooShort { best: best.score });
    }
    if best.score >= ACCEPT_BELOW {
        return Err(Refused::NoMatch { best: best.score });
    }
    if let Some(runner_up) = runner_up.filter(|_| margin < MIN_MARGIN) {
        return Err(Refused::Ambiguous {
            best: best.score,
            runner_up,
        });
    }
    Ok(*best)
}

/// How a run of words sounds: its letters and digits, lowercase and
/// joined, and a consonant skeleton of them.
struct Key {
    letters: Vec<char>,
    skeleton: Vec<char>,
}

impl Key {
    fn of<S: AsRef<str>>(words: &[S]) -> Key {
        let letters: Vec<char> = words
            .iter()
            .flat_map(|w| w.as_ref().chars())
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect();
        let skeleton = skeleton(&letters);
        Key { letters, skeleton }
    }

    /// 0 for the same sound, 1 for nothing in common.
    fn distance(&self, other: &Key) -> f32 {
        0.5 * normalized_edits(&self.letters, &other.letters)
            + 0.5 * normalized_edits(&self.skeleton, &other.skeleton)
    }
}

fn is_vowel(c: char) -> bool {
    matches!(c, 'a' | 'e' | 'i' | 'o' | 'u' | 'y' | 'æ' | 'ø' | 'å')
}

/// The consonants as they sound, doubles collapsed: "FrankenTerm" and
/// "franken turm" are both "frnkntrm". A leading vowel stays as 'a', so
/// "on" and "in" keep a sound.
fn skeleton(letters: &[char]) -> Vec<char> {
    let mut out: Vec<char> = Vec::with_capacity(letters.len());
    let mut push = |c: char| {
        if out.last() != Some(&c) {
            out.push(c);
        }
    };
    for (i, &c) in letters.iter().enumerate() {
        let next = letters.get(i + 1).copied();
        match c {
            c if is_vowel(c) => {
                if i == 0 {
                    push('a');
                }
            }
            'h' | 'w' if i > 0 => {}
            'c' if matches!(next, Some('e' | 'i' | 'y')) => push('s'),
            'c' | 'q' => push('k'),
            'p' if next == Some('h') => push('f'),
            'z' => push('s'),
            'x' => {
                push('k');
                push('s');
            }
            c => push(c),
        }
    }
    out
}

/// Levenshtein distance divided by the longer length.
fn normalized_edits(a: &[char], b: &[char]) -> f32 {
    let longest = a.len().max(b.len());
    if longest == 0 {
        return 0.0;
    }
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let substitute = diagonal + usize::from(ca != cb);
            diagonal = row[j + 1];
            row[j + 1] = substitute.min(row[j] + 1).min(row[j + 1] + 1);
        }
    }
    row[b.len()] as f32 / longest as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(s: &str) -> Vec<&str> {
        s.split_whitespace().collect()
    }

    #[test]
    fn one_word_replaces_the_two_it_sounds_like() {
        let span = place(&words("use franken term here"), &["FrankenTerm"]).unwrap();
        assert_eq!((span.start, span.end), (1, 3));
    }

    #[test]
    fn a_close_misspelling_still_places() {
        let span = place(&words("open the franken turn window"), &["FrankenTerm"]).unwrap();
        assert_eq!((span.start, span.end), (2, 4));
        assert!(span.score < ACCEPT_BELOW, "{span:?}");
    }

    #[test]
    fn the_newer_of_two_similar_spans_wins() {
        let text = words("send the franken term log to the franken term team");
        let span = place(&text, &["FrankenTerm"]).unwrap();
        assert_eq!((span.start, span.end), (7, 9));
    }

    #[test]
    fn nothing_close_enough_changes_nothing() {
        let got = place(&words("the cat sat on the mat"), &["Kubernetes"]);
        assert!(matches!(got, Err(Refused::NoMatch { .. })), "{got:?}");
    }

    #[test]
    fn two_different_spans_that_fit_equally_change_nothing() {
        let got = place(&words("ask marlene or marlina about it"), &["Marlena"]);
        assert!(matches!(got, Err(Refused::Ambiguous { .. })), "{got:?}");
    }

    #[test]
    fn a_short_unclear_correction_is_refused() {
        let got = place(&words("put it in the box"), &["on"]);
        assert!(matches!(got, Err(Refused::TooShort { .. })), "{got:?}");
    }

    #[test]
    fn only_the_newest_window_is_searched() {
        let mut text = vec!["franken", "term"];
        text.extend(std::iter::repeat_n("cat", MATCH_WINDOW_WORDS));
        let got = place(&text, &["FrankenTerm"]);
        assert!(matches!(got, Err(Refused::NoMatch { .. })), "{got:?}");
    }

    #[test]
    fn place_before_ignores_the_words_after_its_end() {
        let text = words("use franken term here then say FrankenTerm again");
        let span = place_before(&text, 4, &["FrankenTerm"]).unwrap();
        assert_eq!((span.start, span.end), (1, 3));
        let span = place_before(&text, 99, &["FrankenTerm"]).unwrap();
        assert_eq!((span.start, span.end), (6, 7));
    }

    #[test]
    fn empty_input_is_refused() {
        assert_eq!(place(&words("hello there"), &["..."]), Err(Refused::Empty));
        assert_eq!(place::<&str, _>(&[], &["hello"]), Err(Refused::Empty));
    }

    #[test]
    fn skeletons_ignore_vowels_and_spelling() {
        let key = |s: &str| {
            skeleton(&s.chars().collect::<Vec<_>>())
                .into_iter()
                .collect::<String>()
        };
        assert_eq!(key("frankenterm"), "frnkntrm");
        assert_eq!(key("phoenix"), "fnks");
        assert_eq!(key("cello"), "sl");
        assert_eq!(key("on"), "an");
    }

    #[test]
    fn edits_are_normalized_by_the_longer_side() {
        let c = |s: &str| s.chars().collect::<Vec<_>>();
        assert_eq!(normalized_edits(&c("kitten"), &c("sitting")), 3.0 / 7.0);
        assert_eq!(normalized_edits(&c(""), &c("")), 0.0);
        assert_eq!(normalized_edits(&c("abc"), &c("")), 1.0);
    }
}
