//! The dictation's text while keyboard editing is on.
//!
//! An edit tap freezes the words spoken so far: they are processed and
//! normalised once, as delivery would, and from then on only the popup's
//! edits change them. Live text spoken after the last resume stays raw and
//! is processed at delivery, as today. The live preview text is the whole
//! recording's text, so the freeze boundary is kept in its terms: the raw
//! text through the boundary, and its word count.
//!
//! Each edit session's changes of spoken words ([`KeyboardEdit`]) are kept
//! for learning at delivery (table 2.7); a later edit that puts the words
//! back drops the change. Words typed in the popup are never the source of
//! one.
//!
//! With `[osd] edit_resume_at = "caret"`, speech after a resume goes where
//! the caret was when the edit ended, not at the end.

/// One piece of the frozen region.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Piece {
    /// Spoken words as the model gave them (`raw`) and as frozen (`text`),
    /// put at byte `at` of the region before them, or at its end.
    Spoken {
        raw: String,
        text: String,
        at: Option<usize>,
    },
    /// The popup's text at the end of an edit session. It stands for every
    /// piece before it.
    Edited { text: String },
}

/// A keyboard change of spoken words: `wrong` empty for a pure insertion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyboardEdit {
    pub wrong: String,
    pub right: String,
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
    /// Where speech after the last resume goes: a byte offset into the
    /// frozen region, or None for its end.
    insert_at: Option<usize>,
    /// Seconds of audio sent to the live worker before the last resume.
    resumed_at: Option<f32>,
    /// For each word of the frozen region, whether it was spoken.
    spoken: Vec<bool>,
    /// The keyboard changes of spoken words so far, oldest first.
    edits: Vec<KeyboardEdit>,
    /// The changes the popup has said will be learned.
    noted: Vec<KeyboardEdit>,
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
                Piece::Spoken { text: t, at, .. } => match at {
                    Some(at) => insert(&text, *at, t).0,
                    None => join(&text, t),
                },
                Piece::Edited { text: t } => join(&text, t),
            })
    }

    /// Where speech after the last resume goes in the frozen region: a
    /// byte offset, or None for its end.
    pub fn insert_at(&self) -> Option<usize> {
        self.insert_at
    }

    /// Speech after the resume goes at byte `at` of the frozen region (the
    /// popup's caret), or at its end for None. An offset at or past the
    /// end, or inside a character, is the end.
    pub fn resume_at(&mut self, at: Option<usize>) {
        let region = self.frozen();
        self.insert_at = at.filter(|&a| a < region.len() && region.is_char_boundary(a));
    }

    /// Words of the live preview text through the freeze boundary.
    pub fn frozen_words(&self) -> usize {
        self.frozen_words
    }

    /// Raw live text after the freeze boundary.
    pub fn live_text(&self) -> &str {
        &self.live
    }

    /// The last resume came after `at` seconds of audio.
    pub fn resumed(&mut self, at: f32) {
        self.resumed_at = Some(at);
    }

    pub fn resumed_at(&self) -> Option<f32> {
        self.resumed_at
    }

    /// The final engine's text of the audio after the last resume stands
    /// in for the live text after the boundary (engine mode, M9.2). Only
    /// when there is live text: speech after the resume that a pause froze
    /// is in the frozen region already.
    pub fn engine_live(&mut self, text: &str) {
        if !self.live.is_empty() && !text.trim().is_empty() {
            self.live = text.trim().to_string();
        }
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
        let old = self.frozen();
        let at = self.insert_at;
        if let Some(a) = at {
            // Later speech goes on after this piece.
            self.insert_at = Some(insert(&self.frozen(), a, &text).1);
        }
        self.pieces.push(Piece::Spoken {
            raw: new,
            text: text.clone(),
            at,
        });
        self.spoken = carry(&old, &self.frozen(), &self.spoken, true);
        text
    }

    /// The popup's text at the end of an edit session replaces the whole
    /// frozen region. The live text is left alone, and goes at the end
    /// until [`EditDoc::resume_at`] says otherwise.
    pub fn replace_frozen(&mut self, text: &str) {
        self.insert_at = None;
        let old = self.frozen();
        self.note_edits(&old, text);
        self.spoken = carry(&old, text, &self.spoken, false);
        self.pieces.push(Piece::Edited {
            text: text.to_string(),
        });
    }

    /// The keyboard changes of spoken words to learn from at delivery.
    pub fn edits(&self) -> &[KeyboardEdit] {
        &self.edits
    }

    /// The kept changes for which `learns` holds that the popup has not
    /// yet said will be learned; they count as said from now on.
    pub fn take_unnoted(&mut self, learns: impl Fn(&KeyboardEdit) -> bool) -> Vec<KeyboardEdit> {
        let new: Vec<KeyboardEdit> = self
            .edits
            .iter()
            .filter(|e| !self.noted.contains(e) && learns(e))
            .cloned()
            .collect();
        self.noted.extend(new.iter().cloned());
        new
    }

    /// The rules this dictation's edits make for the words frozen after
    /// them (table 2.7): `(key, right)` pairs as [`learn::keyboard_rule`]
    /// makes them.
    ///
    /// [`learn::keyboard_rule`]: crate::text::learn::keyboard_rule
    pub fn session_rules(&self) -> Vec<(String, String)> {
        self.edits
            .iter()
            .filter_map(|e| crate::text::learn::keyboard_rule(&e.wrong, &e.right))
            .collect()
    }

    /// Keep the changes from `old` to `new` that only replace spoken words
    /// or only insert; drop a change that puts back an earlier one's words.
    fn note_edits(&mut self, old: &str, new: &str) {
        let a: Vec<&str> = old.split_whitespace().collect();
        let b: Vec<&str> = new.split_whitespace().collect();
        for (del, ins) in hunks(&word_diff(&a, &b)) {
            let wrong = del.iter().map(|&i| a[i]).collect::<Vec<_>>().join(" ");
            let right = ins.iter().map(|&j| b[j]).collect::<Vec<_>>().join(" ");
            if let Some(i) = self
                .edits
                .iter()
                .position(|e| e.right == wrong && e.wrong == right)
            {
                self.edits.remove(i);
                continue;
            }
            let spoken = del.iter().all(|&i| self.spoken.get(i) == Some(&true));
            // A pure deletion teaches nothing; typed words are no source.
            if right.is_empty() || !spoken {
                continue;
            }
            self.edits.push(KeyboardEdit { wrong, right });
        }
    }

    /// The text to paste: the frozen region exactly as frozen or edited,
    /// with the live text after the last resume, run through `process`, at
    /// its end or at the resume point.
    pub fn final_text(&self, process: impl Fn(&str) -> String) -> String {
        let live = if self.live.is_empty() {
            String::new()
        } else {
            process(&self.live)
        };
        match self.insert_at {
            Some(at) => insert(&self.frozen(), at, &live).0,
            None => join(&self.frozen(), &live),
        }
    }
}

/// One step from the words of an old text to a new one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    /// Old word `i` is new word `j`.
    Same(usize, usize),
    Del(usize),
    Ins(usize),
}

/// The fewest word insertions and deletions from `a` to `b` (a longest
/// common subsequence), in order.
fn word_diff(a: &[&str], b: &[&str]) -> Vec<Op> {
    let pre = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let post = a[pre..]
        .iter()
        .rev()
        .zip(b[pre..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let (am, bm) = (&a[pre..a.len() - post], &b[pre..b.len() - post]);
    // lcs[i][j]: the longest common run of am[i..] and bm[j..].
    let mut lcs = vec![vec![0u32; bm.len() + 1]; am.len() + 1];
    for i in (0..am.len()).rev() {
        for j in (0..bm.len()).rev() {
            lcs[i][j] = if am[i] == bm[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let mut ops: Vec<Op> = (0..pre).map(|i| Op::Same(i, i)).collect();
    let (mut i, mut j) = (0, 0);
    while i < am.len() || j < bm.len() {
        if i < am.len() && j < bm.len() && am[i] == bm[j] {
            ops.push(Op::Same(pre + i, pre + j));
            i += 1;
            j += 1;
        } else if j < bm.len() && (i == am.len() || lcs[i][j + 1] >= lcs[i + 1][j]) {
            ops.push(Op::Ins(pre + j));
            j += 1;
        } else {
            ops.push(Op::Del(pre + i));
            i += 1;
        }
    }
    let (ae, be) = (a.len() - post, b.len() - post);
    ops.extend((0..post).map(|k| Op::Same(ae + k, be + k)));
    ops
}

/// The changes in `ops`: each run of deletions and insertions between two
/// kept words, as the old and the new word indexes.
fn hunks(ops: &[Op]) -> Vec<(Vec<usize>, Vec<usize>)> {
    let mut out = Vec::new();
    let mut cur: (Vec<usize>, Vec<usize>) = Default::default();
    for op in ops {
        match *op {
            Op::Same(..) => {
                if !cur.0.is_empty() || !cur.1.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            Op::Del(i) => cur.0.push(i),
            Op::Ins(j) => cur.1.push(j),
        }
    }
    if !cur.0.is_empty() || !cur.1.is_empty() {
        out.push(cur);
    }
    out
}

/// Which words of `new` were spoken: a word kept from `old` keeps its
/// mark in `spoken`, a new one gets `added`.
fn carry(old: &str, new: &str, spoken: &[bool], added: bool) -> Vec<bool> {
    let a: Vec<&str> = old.split_whitespace().collect();
    let b: Vec<&str> = new.split_whitespace().collect();
    let mut out = vec![added; b.len()];
    for op in word_diff(&a, &b) {
        if let Op::Same(i, j) = op {
            out[j] = spoken.get(i).copied().unwrap_or(added);
        }
    }
    out
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

/// `add` put into `s` at byte `at`, with a space wherever it would touch a
/// word. Also the byte just after `add` in the result, where more speech
/// goes. At the end of `s` this is [`join`].
pub fn insert(s: &str, at: usize, add: &str) -> (String, usize) {
    if add.is_empty() {
        return (s.to_string(), at);
    }
    let (left, right) = s.split_at(at);
    let mut out = join(left, add);
    let end = out.len();
    if !right.is_empty() && !starts_word_gap(right) && !add.ends_with(char::is_whitespace) {
        out.push(' ');
    }
    out.push_str(right);
    (out, end)
}

/// Where `frozen` added text to `base`, when it is `base` with one
/// insertion: the byte offset in `base` and the text added. The end of
/// `base` when `frozen` starts with it.
pub fn insertion<'a>(base: &str, frozen: &'a str) -> Option<(usize, &'a str)> {
    if let Some(added) = frozen.strip_prefix(base) {
        return Some((base.len(), added));
    }
    if frozen.len() < base.len() {
        return None;
    }
    let common = base
        .char_indices()
        .zip(frozen.chars())
        .find(|((_, a), b)| a != b)
        .map_or(base.len(), |((i, _), _)| i);
    (0..=common)
        .rev()
        .filter(|&p| base.is_char_boundary(p))
        .find(|&p| {
            frozen.ends_with(&base[p..]) && frozen.is_char_boundary(frozen.len() - (base.len() - p))
        })
        .map(|p| (p, &frozen[p..frozen.len() - (base.len() - p)]))
}

/// The popup's `text`, made from `base`, with what `frozen` added to
/// `base` since: at the end, or, with `inside`, where it was added when
/// `text` still ends as `base` does after that point. Also where the added
/// text went and its length, None when nothing was added.
pub fn add_unseen(
    text: &str,
    base: &str,
    frozen: &str,
    inside: bool,
) -> (String, Option<(usize, usize)>) {
    let found = if inside {
        insertion(base, frozen)
    } else {
        frozen.strip_prefix(base).map(|added| (base.len(), added))
    };
    let Some((at, added)) = found.filter(|(_, a)| !a.is_empty()) else {
        return (text.to_string(), None);
    };
    let suffix = &base[at..];
    if !suffix.is_empty() && !text.ends_with(suffix) {
        // The popup changed what came after: the words go at the end.
        let out = join(text, added.trim());
        let len = out.len() - text.len();
        return (out, Some((text.len(), len)));
    }
    let pos = text.len() - suffix.len();
    let out = format!("{}{added}{}", &text[..pos], &text[pos..]);
    (out, Some((pos, added.len())))
}

fn starts_word_gap(s: &str) -> bool {
    s.starts_with(char::is_whitespace)
}

/// `s` after its first `n` whitespace-separated words.
pub fn after_words(s: &str, n: usize) -> &str {
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
                text: "FIX THIS WORD".into(),
                at: None
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

    /// R23 v2: speech after a resume goes at the caret, the pause after it
    /// freezes it there, and the tail goes on after it.
    #[test]
    fn speech_after_a_resume_goes_at_the_caret() {
        let mut doc = EditDoc::new(1);
        doc.freeze("one two three", upper);
        doc.replace_frozen("One two three.");
        // The caret was before "two".
        doc.resume_at(Some(4));
        assert_eq!(doc.insert_at(), Some(4));
        doc.live("one two three four five");
        assert_eq!(doc.live_text(), "four five");
        assert_eq!(doc.final_text(upper), "One FOUR FIVE two three.");
        // The next pause freezes the new speech at the caret.
        doc.freeze("one two three four five", upper);
        assert_eq!(doc.frozen(), "One FOUR FIVE two three.");
        assert_eq!(doc.insert_at(), Some("One FOUR FIVE".len()));
        // The settling tail goes after it, not at the end.
        doc.freeze("one two three four five six", upper);
        assert_eq!(doc.frozen(), "One FOUR FIVE SIX two three.");
        assert_eq!(
            doc.pieces().last(),
            Some(&Piece::Spoken {
                raw: "six".into(),
                text: "SIX".into(),
                at: Some("One FOUR FIVE".len())
            })
        );
        // An edit takes the point away until the next resume says.
        doc.replace_frozen("Done.");
        assert_eq!(doc.insert_at(), None);
        doc.live("one two three four five six seven");
        assert_eq!(doc.final_text(upper), "Done. SEVEN");
    }

    #[test]
    fn a_caret_at_the_end_or_inside_a_letter_is_the_end() {
        let mut doc = EditDoc::new(1);
        doc.freeze("æble pie", |t| t.to_string());
        doc.resume_at(Some("æble pie".len()));
        assert_eq!(doc.insert_at(), None);
        doc.resume_at(Some(1));
        assert_eq!(doc.insert_at(), None, "inside æ");
        doc.resume_at(Some(99));
        assert_eq!(doc.insert_at(), None);
        doc.resume_at(Some(0));
        assert_eq!(doc.insert_at(), Some(0));
        doc.live("æble pie good");
        assert_eq!(doc.final_text(upper), "GOOD æble pie");
    }

    #[test]
    fn insert_puts_a_space_where_words_would_touch() {
        assert_eq!(insert("ab cd", 2, "x"), ("ab x cd".into(), 4));
        assert_eq!(insert("ab cd", 3, "x"), ("ab x cd".into(), 4));
        assert_eq!(insert("ab cd", 5, "x"), ("ab cd x".into(), 7));
        assert_eq!(insert("ab cd", 0, "x"), ("x ab cd".into(), 1));
        assert_eq!(insert("ab\ncd", 3, "x"), ("ab\nx cd".into(), 4));
        assert_eq!(insert("ab cd", 2, ""), ("ab cd".into(), 2));
    }

    #[test]
    fn unseen_words_go_where_the_daemon_put_them() {
        assert_eq!(insertion("a b", "a b c"), Some((3, " c")));
        assert_eq!(insertion("a b.", "a X b."), Some((2, "X ")));
        assert_eq!(insertion("a b", "x"), None);
        // At the end, as in v1, and only at the end without `inside`.
        assert_eq!(
            add_unseen("A b", "a b", "a b c", false),
            ("A b c".into(), Some((3, 2)))
        );
        assert_eq!(
            add_unseen("A b.", "a b.", "a X b.", false),
            ("A b.".into(), None)
        );
        // Inside: before the part of the base that the popup kept.
        assert_eq!(
            add_unseen("Hi a b.", "a b.", "a X b.", true),
            ("Hi a X b.".into(), Some((5, 2)))
        );
        // The popup changed that part too: the words go at the end.
        assert_eq!(
            add_unseen("a c", "a b.", "a X b.", true),
            ("a c X".into(), Some((3, 2)))
        );
    }

    fn same(s: &str) -> String {
        s.to_string()
    }

    fn edits(doc: &EditDoc) -> Vec<(&str, &str)> {
        doc.edits()
            .iter()
            .map(|e| (e.wrong.as_str(), e.right.as_str()))
            .collect()
    }

    /// Table 2.7, per edit session: which changes are kept to learn from
    /// and which make a rule for the rest of the dictation.
    #[test]
    fn keyboard_edits_follow_table_2_7() {
        let mut doc = EditDoc::new(1);
        doc.freeze("um teh cat sat on paris", same);
        // A swap, a change of case and a pure deletion.
        doc.replace_frozen("the cat sat on Paris");
        assert_eq!(edits(&doc), [("um teh", "the"), ("paris", "Paris")]);
        // A pure insertion and a long rewording are kept, with no rule.
        doc.freeze("um teh cat sat on paris and it was a long day", same);
        doc.replace_frozen("the big cat sat on Paris and we had fun all day");
        assert_eq!(
            edits(&doc),
            [
                ("um teh", "the"),
                ("paris", "Paris"),
                ("", "big"),
                ("it was a long", "we had fun all")
            ]
        );
        assert_eq!(
            doc.session_rules(),
            [
                ("um teh".to_string(), "the".to_string()),
                ("paris".to_string(), "Paris".to_string())
            ]
        );
    }

    /// Typed words are never the source of a change, and an edit undone
    /// in a later session is dropped.
    #[test]
    fn typed_words_and_put_back_edits_teach_nothing() {
        let mut doc = EditDoc::new(1);
        doc.freeze("teh cat", same);
        doc.replace_frozen("the cat xyz");
        assert_eq!(edits(&doc), [("teh", "the"), ("", "xyz")]);
        // The typed word is changed: no source. The spoken one goes back.
        doc.replace_frozen("teh cat abc");
        assert_eq!(edits(&doc), [("", "xyz")]);
        // New speech after the edits is spoken again.
        doc.freeze("teh cat dog", same);
        doc.replace_frozen("teh cat abc Dog");
        assert_eq!(edits(&doc), [("", "xyz"), ("dog", "Dog")]);
    }

    #[test]
    fn the_word_diff_finds_the_fewest_changes() {
        let a = ["a", "b", "c", "d"];
        let b = ["a", "x", "c", "d", "e"];
        assert_eq!(
            hunks(&word_diff(&a, &b)),
            [(vec![1], vec![1]), (vec![], vec![4])]
        );
        assert_eq!(hunks(&word_diff(&a, &a)), []);
        assert_eq!(hunks(&word_diff(&[], &b)), [(vec![], vec![0, 1, 2, 3, 4])]);
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
