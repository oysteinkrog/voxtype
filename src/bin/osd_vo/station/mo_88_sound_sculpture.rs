//! MO-88 typographic sound sculpture on the captions look: extruded letter
//! bodies behind the flat text, as deep as each word was loud, laid flat
//! from left to right when the dictation finishes.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-88", "Typographic sound sculpture", OsdTheme::Captions)
        .feature(wild_space::SOUND_SCULPTURE)
        .moments(&[Moment::Dictation, Moment::FinalPass])
}
