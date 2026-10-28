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
        .child(
            at(
                Element::label("rule", "newest line: baseline ±3 px, tracking 5 %, ink 70 %")
                    .text_style(look.mono(9.0))
                    .color(accent),
                8.0,
                rule_y(baseline, look.text_line, size.h),
            )
            .fill(Color::rgba(0.0, 0.0, 0.0, 0.7)),
        )
}

/// The status row's height at the foot of the popup, with some room.
const FOOT_H: f32 = 40.0;
/// The rule label's height.
const RULE_H: f32 = 13.0;

/// Where the rule label goes: under the band, or above the newest line
/// when under the band it would run into the status row.
fn rule_y(baseline: f32, text_line: f32, height: f32) -> f32 {
    let below = baseline + BASELINE_PX + 2.0;
    if below + RULE_H <= height - FOOT_H {
        below
    } else {
        baseline - 0.2 * text_line - 0.5 * text_line - RULE_H
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rule_label_keeps_clear_of_the_status_row() {
        // Room under the band: the label goes there.
        assert!(rule_y(100.0, 30.0, 300.0) > 100.0);
        // The newest line just over the status row: the label goes above it.
        let y = rule_y(255.0, 30.0, 300.0);
        assert!(y + RULE_H <= 255.0 - 0.2 * 30.0);
        assert!(y + RULE_H <= 300.0 - FOOT_H);
    }
}
