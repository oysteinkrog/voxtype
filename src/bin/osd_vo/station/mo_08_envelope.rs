//! MO-08 envelope and budget checks: `voxtype-osd-vo --envelope` checks the
//! reading-line envelope frame by frame offline. This tile shows what it
//! guards: the swiss popup with the newest line's band drawn over it, the
//! 3 px either side of the baseline that the newest line may move within,
//! through revisions, a wrap and the final pass.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-08", "Envelope and budget checks", OsdTheme::Swiss)
        .moments(&[Moment::RevisionBurst, Moment::Wrap, Moment::FinalPass])
        .view(view)
}

/// The envelope rule 1 allows the newest line's baseline, in pixels.
const BASELINE_PX: f32 = 3.0;

fn view(p: &Popup, tokens: &vo::Tokens) -> Element<Msg> {
    let look = &p.look;
    let size = look.surface();
    let body = p.view(tokens);
    if p.closing {
        return body;
    }
    let Some(centre) = look.newest_line_centre(p.lines) else {
        return body;
    };
    let centre = centre + look.reading_shift(p.lines).unwrap_or(0.0);
    // The baseline sits about a fifth of a line below the line's centre.
    let baseline = centre + 0.2 * look.text_line;
    let accent = Color::rgba(1.0, 0.72, 0.2, 1.0);
    let band = Color::rgba(1.0, 0.72, 0.2, 0.16);
    Element::layer("envelope")
        .size(size.w, size.h)
        .child(body)
        .child(at(
            Element::panel("band")
                .size(size.w, 2.0 * BASELINE_PX)
                .fill(band),
            0.0,
            baseline - BASELINE_PX,
        ))
        .child(at(
            Element::panel("baseline").size(size.w, 1.0).fill(accent),
            0.0,
            baseline,
        ))
        .child(at(
            Element::label("rule", "newest line: baseline ±3 px, tracking 5 %, ink 70 %")
                .text_style(look.mono(9.0))
                .color(accent),
            8.0,
            baseline + BASELINE_PX + 2.0,
        ))
}
