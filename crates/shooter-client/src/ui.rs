//! The drawn UI's primitives (the HUD, the builder, the death prompt): rounded rectangles and text
//! in a real font, as items in frame pixels that [`crate::gfx::ui`] draws over the frame after
//! either renderer, natively and in the browser (WebGL 2 included).
//!
//! render-engine's screen overlay draws solid rectangles only (no text, no rounded corners), so
//! the client draws its UI in a pass of its own. The font is Barlow Semi Condensed SemiBold (SIL
//! Open Font License, `assets/fonts/OFL.txt`), rasterized on demand into a glyph atlas at the
//! size it is drawn.

use std::sync::LazyLock;

use render_engine::glam::{Vec2, Vec4};

/// The UI font.
pub static FONT: LazyLock<fontdue::Font> = LazyLock::new(|| {
    fontdue::Font::from_bytes(
        &include_bytes!("../assets/fonts/BarlowSemiCondensed-SemiBold.ttf")[..],
        fontdue::FontSettings::default(),
    )
    .expect("the bundled UI font parses")
});

/// The font's cap height as a share of its size (Barlow: 700 of 1000 units).
pub const CAP_HEIGHT: f32 = 0.7;

/// Extra space after every letter, as a share of the font size: capitals read better spaced.
pub const TRACKING: f32 = 0.04;

/// One thing the UI draws, in frame pixels (top left the origin, y down). Colours are linear RGB
/// with straight alpha, blended over the frame like render-engine's overlay quads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Item {
    /// A rectangle with corners rounded to `radius` pixels (0: square), its edges anti-aliased.
    Rect {
        position: Vec2,
        size: Vec2,
        radius: f32,
        color: Vec4,
    },
    /// A glyph of [`FONT`] at `size` pixels, its pen at `pen` (x, baseline).
    Glyph {
        pen: Vec2,
        character: char,
        size: f32,
        color: Vec4,
    },
}

impl Item {
    /// Its top left and bottom right corners in frame pixels (a glyph's bitmap's).
    #[must_use]
    pub fn bounds(&self) -> (Vec2, Vec2) {
        match *self {
            Self::Rect { position, size, .. } => (position, position + size),
            Self::Glyph {
                pen,
                character,
                size,
                ..
            } => {
                let metrics = FONT.metrics(character, size);
                #[allow(clippy::cast_precision_loss)]
                let (left, bottom) = (pen.x + metrics.xmin as f32, pen.y - metrics.ymin as f32);
                #[allow(clippy::cast_precision_loss)]
                let size = Vec2::new(metrics.width as f32, metrics.height as f32);
                (
                    Vec2::new(left, bottom - size.y),
                    Vec2::new(left + size.x, bottom),
                )
            }
        }
    }
}

/// `text`'s advance at `size` pixels: its letters', their kerning and the tracking, without the
/// trailing letter's tracking.
#[must_use]
pub fn advance(text: &str, size: f32) -> f32 {
    let mut width = 0.0;
    let mut previous = None;
    let mut count = 0.0;
    for character in text.chars() {
        if let Some(left) = previous {
            width += FONT.horizontal_kern(left, character, size).unwrap_or(0.0);
        }
        width += FONT.metrics(character, size).advance_width;
        count += 1.0;
        previous = Some(character);
    }
    width + (count - 1.0_f32).max(0.0) * TRACKING * size
}

/// The glyphs of `text` at `size` pixels from `pen` (x, baseline), each placed at its kerned and
/// tracked advance.
pub fn glyphs(text: &str, pen: Vec2, size: f32, color: Vec4) -> impl Iterator<Item = Item> + '_ {
    let mut x = pen.x;
    let mut previous: Option<char> = None;
    text.chars().map(move |character| {
        if let Some(left) = previous {
            x += FONT.horizontal_kern(left, character, size).unwrap_or(0.0) + TRACKING * size;
        }
        let item = Item::Glyph {
            pen: Vec2::new(x, pen.y),
            character,
            size,
            color,
        };
        x += FONT.metrics(character, size).advance_width;
        previous = Some(character);
        item
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_measured_and_laid_out_by_the_font() {
        let size = 20.0;
        let width = advance("ENERGY RIFLE", size);
        assert!(width > 5.0 * size && width < 12.0 * size, "{width}");
        // Twice the size, twice the width.
        assert!((advance("ENERGY RIFLE", 2.0 * size) - 2.0 * width).abs() < 0.5);
        assert!(advance("", size) == 0.0);
        let placed: Vec<Item> = glyphs("AV", Vec2::new(10.0, 50.0), size, Vec4::ONE).collect();
        let [Item::Glyph { pen: a, .. }, Item::Glyph { pen: v, .. }] = placed[..] else {
            panic!("{placed:?}");
        };
        assert_eq!(a, Vec2::new(10.0, 50.0));
        assert!(v.x > a.x && v.y == a.y);
        // The cap height is what the layout assumes.
        let capital = FONT.metrics('H', 100.0);
        assert!(
            (capital.height as f32 - CAP_HEIGHT * 100.0).abs() <= 2.0,
            "{capital:?}"
        );
    }
}
