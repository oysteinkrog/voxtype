//! The `aurora` look of the vo popup, after concept 19: an aurora mesh.
//!
//! A slot for now (RD-03): it draws the plain swiss panel until its look
//! bead lands.

use super::*;

impl Popup {
    pub(crate) fn aurora_view(&self) -> Element<Msg> {
        self.swiss_view()
    }
}
