//! The dictation's text while keyboard editing is on.
//!
//! An edit tap freezes the words spoken so far: they are processed and
//! normalised once, as delivery would, and from then on only the popup's
//! edits change them. Live text spoken after the last resume stays raw and
//! is processed at delivery, as today. The live preview text is the whole
//! recording's text, so the freeze boundary is kept in its terms: the raw
//! text through the boundary, and its word count.

/// One piece of the frozen region.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Piece {
    /// Spoken words as the model gave them (`raw`) and as frozen (`text`).
    Spoken { raw: String, text: String },
    /// The popup's text at the end of an edit session. It stands for every
    /// piece before it.
    Edited { text: String },
}

/// The document of one dictation with keyboard editing on.
#[derive(Clone, Debug, Default)]
pub struct EditDoc {
    session: u64,
    pieces: Vec<Piece>,
    /// The raw live text through the freeze boundary.
    frozen_raw: String,
    /// Words in `frozen_raw`.
    frozen_words: usize,
    /// Raw live text after the boundary.
    live: String,
}

impl EditDoc {
    pub fn new(session: u64) -> Self {
        Self {
            session,
            ..Self::default()
        }
    }

    pub fn session(&self) -> u64 {
        self.session
    }

    /// Every piece frozen or edited so far, oldest first.
    pub fn pieces(&self) -> &[Piece] {
        &self.pieces
    }

    /// The frozen region, as the popup shows it and as it is pasted.
    pub fn frozen(&self) -> String {
        let start = self
            .pieces
            .iter()
            .rposition(|p| matches!(p, Piece::Edited { .. }))
            .unwrap_or(0);
        self.pieces[start..]
            .iter()
            .fold(String::new(), |text, piece| match piece {
                Piece::Spoken { text: t, .. } | Piece::Edited { text: t } => join(&text, t),
            })
    }

    /// Raw live text after the freeze boundary.
    pub fn live_text(&self) -> &str {
        &self.live
    }

    /// A new live preview text (the whole recording's). The part after the
    /// boundary becomes the live text; any change before it is dropped.
    pub fn live(&mut self, raw: &str) {
        let raw = raw.trim();
        let after = match raw.strip_prefix(self.frozen_raw.as_str()) {
            Some(rest)
                if self.frozen_raw.is_empty() || rest.is_empty() || starts_word_gap(rest) =>
            {
                rest
            }
            // An earlier word changed: the boundary stays after as many
            // words as were frozen.
            _ => after_words(raw, self.frozen_words),
        };
        self.live = after.trim().to_string();
    }

    /// Freeze the live text: everything in `raw` past the boundary is run
    /// through `process` (text processing and delivery normalisation) and
    /// added to the frozen region. Returns the text added.
    pub fn freeze(&mut self, raw: &str, process: impl Fn(&str) -> String) -> String {
        self.live(raw);
        let new = std::mem::take(&mut self.live);
        // The boundary moves to the end of the text.
        self.frozen_raw = raw.trim().to_string();
        self.frozen_words = self.frozen_raw.split_whitespace().count();
        if new.is_empty() {
            return String::new();
        }
        let text = process(&new);
        self.pieces.push(Piece::Spoken {
            raw: new,
            text: text.clone(),
        });
        text
    }

    /// The popup's text at the end of an edit session replaces the whole
    /// frozen region. The live text is left alone.
    pub fn replace_frozen(&mut self, text: &str) {
        self.pieces.push(Piece::Edited {
            text: text.to_string(),
        });
    }

    /// The text to paste: the frozen region exactly as frozen or edited,
    /// then the live text after the last resume, run through `process`.
    pub fn final_text(&self, process: impl Fn(&str) -> String) -> String {
        let live = if self.live.is_empty() {
            String::new()
        } else {
            process(&self.live)
        };
        join(&self.frozen(), &live)
    }
}

/// `a` then `b`, with a space between unless `a` ends in whitespace.
fn join(a: &str, b: &str) -> String {
    if a.is_empty() {
        return b.to_string();
    }
    if b.is_empty() || a.ends_with(char::is_whitespace) {
        return format!("{a}{b}");
    }
    format!("{a} {b}")
}

fn starts_word_gap(s: &str) -> bool {
    s.starts_with(char::is_whitespace)
}

/// `s` after its first `n` whitespace-separated words.
fn after_words(s: &str, n: usize) -> &str {
    let mut words = 0;
    let mut in_word = false;
    for (i, c) in s.char_indices() {
        if c.is_whitespace() {
            if in_word && words == n {
                return &s[i..];
            }
            in_word = false;
        } else if !in_word {
            in_word = true;
            words += 1;
            if words > n {
                return &s[i..];
            }
        }
    }
    if words <= n {
        ""
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn upper(s: &str) -> String {
        s.to_uppercase()
    }

    #[test]
    fn freeze_processes_once_and_delivery_leaves_it_alone() {
        let mut doc = EditDoc::new(7);
        doc.live("um hello there comma");
        let tp = crate::text::TextProcessor::new(&crate::config::TextConfig {
            spoken_punctuation: true,
            filter_filler_words: true,
            ..Default::default()
        });
        let process = |t: &str| tp.process(t);
        assert_eq!(doc.freeze("um hello there comma", process), "hello there,");
        assert_eq!(doc.frozen(), "hello there,");
        // The popup's typed text is pasted byte for byte, even words
        // processing would change.
        doc.replace_frozen("hello there, um I typed comma");
        doc.live("um hello there comma and then period");
        assert_eq!(doc.live_text(), "and then period");
        assert_eq!(
            doc.final_text(process),
            "hello there, um I typed comma and then."
        );
        assert_eq!(doc.session(), 7);
    }

    #[test]
    fn tail_freeze_appends_after_the_first_freeze() {
        let mut doc = EditDoc::new(1);
        assert_eq!(doc.freeze("one two", upper), "ONE TWO");
        // Settling: the lookahead tail lands, and is frozen too.
        assert_eq!(doc.freeze("one two three four", upper), "THREE FOUR");
        assert_eq!(doc.frozen(), "ONE TWO THREE FOUR");
        assert_eq!(doc.freeze("one two three four", upper), "");
        assert_eq!(doc.pieces().len(), 2);
    }

    #[test]
    fn changes_before_the_boundary_are_dropped() {
        let mut doc = EditDoc::new(1);
        doc.freeze("the quick brown", upper);
        // A revision of a frozen word, then words after resume.
        doc.live("the quack brown fox jumps");
        assert_eq!(doc.live_text(), "fox jumps");
        assert_eq!(doc.final_text(upper), "THE QUICK BROWN FOX JUMPS");
        // A revision that splits a frozen word still keeps the boundary
        // after the frozen word count.
        doc.live("the quick brow n fox");
        assert_eq!(doc.live_text(), "n fox");
        assert_eq!(doc.frozen(), "THE QUICK BROWN");
    }

    #[test]
    fn a_word_that_continues_past_the_boundary_is_not_split() {
        let mut doc = EditDoc::new(1);
        doc.freeze("act", upper);
        doc.live("activity detection");
        // "activity" is not "act" + "ivity": the frozen word count decides.
        assert_eq!(doc.live_text(), "detection");
    }

    #[test]
    fn edits_replace_the_frozen_region_and_keep_the_history() {
        let mut doc = EditDoc::new(1);
        doc.freeze("fix this word", upper);
        doc.replace_frozen("Fix this world\n");
        doc.live("fix this word next line");
        assert_eq!(doc.final_text(upper), "Fix this world\nNEXT LINE");
        // A second edit session freezes the new speech after the edit.
        doc.freeze("fix this word next line", upper);
        assert_eq!(doc.frozen(), "Fix this world\nNEXT LINE");
        doc.replace_frozen("Done.");
        assert_eq!(doc.final_text(upper), "Done.");
        assert_eq!(
            doc.pieces()[0],
            Piece::Spoken {
                raw: "fix this word".into(),
                text: "FIX THIS WORD".into()
            }
        );
    }

    #[test]
    fn no_decoration_or_cut_enters_the_buffer() {
        // The freeze takes the whole live text, never the popup's cut and
        // ellipsis form of it.
        let words: Vec<String> = (0..300).map(|i| format!("w{i}")).collect();
        let raw = words.join(" ");
        let mut doc = EditDoc::new(1);
        doc.freeze(&raw, |t| t.to_string());
        assert_eq!(doc.frozen(), raw);
        assert!(!doc.frozen().contains('…'));
    }

    #[test]
    fn after_words_counts_whitespace_separated_words() {
        assert_eq!(after_words("a b  c", 0), "a b  c");
        assert_eq!(after_words("a b  c", 1), " b  c");
        assert_eq!(after_words("a b  c", 2), "  c");
        assert_eq!(after_words("a b  c", 3), "");
        assert_eq!(after_words("æø å", 1), " å");
        assert_eq!(after_words("", 2), "");
    }

    /// R21: random live logs with appends and changes to earlier words,
    /// random freezes and random edits. The frozen region changes only by
    /// a freeze (which only appends) or an edit (which sets it), and the
    /// final text always starts with it.
    #[test]
    fn frozen_and_typed_text_never_change_by_live_text() {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut rand = move |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        let vocab = ["um", "comma", "the", "æble", "fox", "I'm", "period", "x"];
        for _ in 0..300 {
            let mut doc = EditDoc::new(1);
            let mut words: Vec<String> = Vec::new();
            let mut frozen = String::new();
            for _ in 0..60 {
                match rand(10) {
                    // The model adds words.
                    0..=4 => {
                        for _ in 0..=rand(3) {
                            words.push(vocab[rand(vocab.len() as u64) as usize].into());
                        }
                        doc.live(&words.join(" "));
                    }
                    // A revision of any earlier word, frozen or not.
                    5 | 6 if !words.is_empty() => {
                        let i = rand(words.len() as u64) as usize;
                        words[i] = vocab[rand(vocab.len() as u64) as usize].into();
                        doc.live(&words.join(" "));
                    }
                    7 => {
                        let added = doc.freeze(&words.join(" "), upper);
                        let want = join(&frozen, &added);
                        frozen = want;
                    }
                    8 => {
                        frozen = format!("typed {} ", rand(100));
                        doc.replace_frozen(&frozen);
                    }
                    _ => {}
                }
                assert_eq!(doc.frozen(), frozen);
                assert!(doc.final_text(upper).starts_with(frozen.trim_end()));
            }
        }
    }
}
