//! The `hologram` look of the vo popup, after concept 20: floating hologram text.
//!
//! A slot for now (RD-03): it draws the plain swiss panel until its look
//! bead lands.

use super::*;

impl Popup {
    pub(crate) fn hologram_view(&self) -> Element<Msg> {
        self.swiss_view()
    }
}
