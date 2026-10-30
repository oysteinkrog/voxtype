//! Spelling a word inside a spoken correction.
//!
//! A correction hold whose first word is "spell" spells one word: NATO
//! alphabet words ("golf lima india mike"), plain letters ("g l i m") or
//! digits, each letter lower case unless "capital" comes before it. The
//! letters make one word that replaces the wrong words as a correction does.

/// The NATO alphabet, with the spellings speech models write.
const NATO: &[(&str, char)] = &[
    ("alpha", 'a'),
    ("alfa", 'a'),
    ("bravo", 'b'),
    ("charlie", 'c'),
    ("delta", 'd'),
    ("echo", 'e'),
    ("foxtrot", 'f'),
    ("golf", 'g'),
    ("hotel", 'h'),
    ("india", 'i'),
    ("juliet", 'j'),
    ("juliett", 'j'),
    ("kilo", 'k'),
    ("lima", 'l'),
    ("mike", 'm'),
    ("november", 'n'),
    ("oscar", 'o'),
    ("papa", 'p'),
    ("quebec", 'q'),
    ("romeo", 'r'),
    ("sierra", 's'),
    ("tango", 't'),
    ("uniform", 'u'),
    ("victor", 'v'),
    ("whiskey", 'w'),
    ("whisky", 'w'),
    ("xray", 'x'),
    ("yankee", 'y'),
    ("zulu", 'z'),
];

const DIGITS: &[&str] = &[
    "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine",
];

fn key(word: &str) -> String {
    word.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// A few capitals a model wrote for letters said quickly ("GLI").
fn is_letter_run(word: &str) -> bool {
    let letters: Vec<char> = word.chars().filter(|c| c.is_alphanumeric()).collect();
    (2..=4).contains(&letters.len()) && letters.iter().all(|c| c.is_uppercase())
}

/// Whether the words of a hold spell a word: the first one is "spell".
pub fn is_spelling<S: AsRef<str>>(words: &[S]) -> bool {
    words.first().is_some_and(|w| key(w.as_ref()) == "spell")
}

/// The letters spelled so far in a hold that is spelling, and whether every
/// word after "spell" was a letter. None when the hold is not spelling.
pub fn letters<S: AsRef<str>>(words: &[S]) -> Option<(String, bool)> {
    if !is_spelling(words) {
        return None;
    }
    let mut out = String::new();
    let mut all = true;
    let mut capital = false;
    for word in &words[1..] {
        let k = key(word.as_ref());
        if k == "capital" || k == "cap" || k == "uppercase" {
            capital = true;
            continue;
        }
        // A model may join a few letters into one word ("GLI"): each is a letter.
        let chars: Vec<char> = if let Some(&(_, c)) = NATO.iter().find(|(n, _)| *n == k) {
            vec![c]
        } else if let Some(d) = DIGITS.iter().position(|d| *d == k) {
            vec![char::from(b'0' + d as u8)]
        } else if k.chars().count() == 1 || is_letter_run(word.as_ref()) {
            k.chars().collect()
        } else {
            all &= k.is_empty();
            continue;
        };
        for (i, c) in chars.into_iter().enumerate() {
            if capital && i == 0 {
                out.extend(c.to_uppercase());
            } else {
                out.push(c);
            }
        }
        capital = false;
    }
    Some((out, all))
}

/// The word a finished hold spells; None when the hold is not spelling, has
/// no letters, or has a word that is not a letter.
pub fn spelled<S: AsRef<str>>(words: &[S]) -> Option<String> {
    letters(words).and_then(|(word, all)| (all && !word.is_empty()).then_some(word))
}

/// Letters with spaces between them, for the popup's letter field.
pub fn spaced(letters: &str) -> String {
    letters
        .chars()
        .map(|c| c.to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(s: &str) -> Vec<&str> {
        s.split_whitespace().collect()
    }

    #[test]
    fn nato_words_spell_a_word() {
        assert_eq!(
            spelled(&words("Spell golf lima india mike bravo oscar tango.")).as_deref(),
            Some("glimbot")
        );
        assert_eq!(
            spelled(&words(
                "spell, Capital Golf, Lima, India. X-ray Alfa Juliett"
            ))
            .as_deref(),
            Some("Glixaj")
        );
    }

    #[test]
    fn plain_letters_and_digits_are_letters_too() {
        assert_eq!(
            spelled(&words("spell capital z e d nine")).as_deref(),
            Some("Zed9")
        );
        assert_eq!(spelled(&words("Spell GLI mike")).as_deref(), Some("glim"));
        assert_eq!(spelled(&words("spell b. o. t.")).as_deref(), Some("bot"));
    }

    #[test]
    fn a_hold_that_is_not_spelling_or_has_a_stray_word_spells_nothing() {
        assert_eq!(spelled(&words("glim bot")), None);
        assert_eq!(letters(&words("glim bot")), None);
        assert_eq!(spelled(&words("spell")), None);
        assert_eq!(spelled(&words("spell golf banana lima")), None);
        assert_eq!(spelled(&words("spell the bot")), None);
        assert_eq!(
            letters(&words("spell golf banana lima")),
            Some(("gl".to_string(), false))
        );
    }

    #[test]
    fn letters_so_far_show_while_the_key_is_held() {
        assert_eq!(letters(&words("Spell")), Some((String::new(), true)));
        assert_eq!(
            letters(&words("Spell capital golf lima")),
            Some(("Gl".to_string(), true))
        );
        assert_eq!(spaced("Glim"), "G l i m");
    }
}
