//! The `glass` look of the vo popup, after concept 08: spatial glass.
//!
//! A slot for now (RD-03): it draws the plain swiss panel until its look
//! bead lands.

use super::*;

impl Popup {
    pub(crate) fn glass_view(&self) -> Element<Msg> {
        self.swiss_view()
    }
}
