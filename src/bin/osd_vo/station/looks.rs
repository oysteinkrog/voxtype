//! One exhibit per look (RD-07): `LK-<look>` shows the look as it runs by
//! default, with its own motion set (`defaults(theme)`) and its idle motion,
//! playing the demo dictation, a cancel and the popup open on room tone in
//! turn. They come after every MO, EG and R2 exhibit and start a page of
//! their own.

use super::*;

/// Each look with its exhibit id and name, in `OsdTheme::VO_LOOKS` order.
const LOOKS: [(OsdTheme, &str, &str); 27] = [
    (OsdTheme::Swiss, "LK-swiss", "Swiss look"),
    (OsdTheme::Showcase, "LK-showcase", "Showcase look"),
    (OsdTheme::Type, "LK-type", "Type look"),
    (OsdTheme::Amber, "LK-amber", "Amber look"),
    (OsdTheme::Radial, "LK-radial", "Radial look"),
    (OsdTheme::Tron, "LK-tron", "Tron look"),
    (OsdTheme::Phosphor, "LK-phosphor", "Phosphor look"),
    (OsdTheme::Mission, "LK-mission", "Mission look"),
    (OsdTheme::Palette, "LK-palette", "Palette look"),
    (OsdTheme::Dotmatrix, "LK-dotmatrix", "Dot matrix look"),
    (OsdTheme::Swissfi, "LK-swissfi", "Swiss sci-fi look"),
    (OsdTheme::Scope, "LK-scope", "Scope look"),
    (OsdTheme::Captions, "LK-captions", "Captions look"),
    (OsdTheme::Karaoke, "LK-karaoke", "Karaoke look"),
    (OsdTheme::Island, "LK-island", "Island look"),
    (OsdTheme::Jarvis, "LK-jarvis", "Jarvis look"),
    (OsdTheme::Particles, "LK-particles", "Particles look"),
    (OsdTheme::Synthwave, "LK-synthwave", "Synthwave look"),
    (OsdTheme::Mecha, "LK-mecha", "Mecha look"),
    (OsdTheme::Cyberpunk, "LK-cyberpunk", "Cyberpunk look"),
    (OsdTheme::Orb, "LK-orb", "Orb look"),
    (OsdTheme::Chrome, "LK-chrome", "Chrome look"),
    (OsdTheme::Glass, "LK-glass", "Glass look"),
    (OsdTheme::Biolume, "LK-biolume", "Biolume look"),
    (OsdTheme::Aurora, "LK-aurora", "Aurora look"),
    (OsdTheme::Hologram, "LK-hologram", "Hologram look"),
    (OsdTheme::River, "LK-river", "River look"),
];

/// The look exhibits, one per look in `OsdTheme::VO_LOOKS`.
pub(super) fn exhibits() -> Vec<Exhibit> {
    LOOKS
        .iter()
        .map(|&(theme, id, name)| {
            let mut e = Exhibit::new(id, name, theme).moments(&[
                Moment::Dictation,
                Moment::Cancel,
                Moment::LongSilence,
            ]);
            e.features = super::super::defaults::defaults(theme).to_vec();
            e
        })
        .collect()
}

/// A look exhibit, which starts a page of its own.
pub(super) fn is_look(e: &Exhibit) -> bool {
    e.id.starts_with("LK-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_look_has_one_exhibit_with_its_own_set() {
        let all = exhibits();
        assert_eq!(all.len(), OsdTheme::VO_LOOKS.len());
        for (e, theme) in all.iter().zip(OsdTheme::VO_LOOKS) {
            assert_eq!(e.theme, theme, "{}", e.id);
            assert_eq!(e.id, format!("LK-{}", format!("{theme:?}").to_lowercase()));
            assert_eq!(
                e.features,
                super::super::super::defaults::defaults(theme),
                "{}",
                e.id
            );
        }
    }

    #[test]
    fn look_exhibits_start_their_own_page() {
        let all = super::super::exhibits();
        let first = all.iter().position(is_look).expect("look exhibits");
        assert!(all[first..].iter().all(is_look));
        assert_eq!(all.len() - first, 27);
        let starts = super::super::page_starts(&all, 12);
        assert!(starts.contains(&first), "{starts:?} {first}");
        // The pages before them are the same as without them.
        let before = super::super::page_starts(&all[..first], 12);
        assert_eq!(&starts[..before.len()], &before[..]);
    }
}
