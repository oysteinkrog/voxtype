//! The `chrome` look of the vo popup, after concept 05: liquid chrome.
//!
//! A slot for now (RD-03): it draws the plain swiss panel until its look
//! bead lands.

use super::*;

impl Popup {
    pub(crate) fn chrome_view(&self) -> Element<Msg> {
        self.swiss_view()
    }
}
