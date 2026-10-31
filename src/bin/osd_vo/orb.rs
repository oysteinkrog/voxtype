//! The `orb` look of the vo popup, after concept 01: a glass aurora orb.
//!
//! A slot for now (RD-03): it draws the plain swiss panel until its look
//! bead lands.

use super::*;

impl Popup {
    pub(crate) fn orb_view(&self) -> Element<Msg> {
        self.swiss_view()
    }
}
