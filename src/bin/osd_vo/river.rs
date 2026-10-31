//! The `river` look of the vo popup, after concept 24: a river of light.
//!
//! A slot for now (RD-03): it draws the plain swiss panel until its look
//! bead lands.

use super::*;

impl Popup {
    pub(crate) fn river_view(&self) -> Element<Msg> {
        self.swiss_view()
    }
}
