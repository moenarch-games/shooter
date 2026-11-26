//! The drawn UI's primitives (the HUD, the builder, the death prompt, the main menu): rounded
//! rectangles, text in a real font and whole textures (the menu's map previews), as items in frame pixels, turned into overlay quads ([`Atlas::quads`]) that
//! either renderer draws over the frame (render-engine's `Scene::overlay`, the lite renderer's
//! overlay on WebGL 2), natively and in the browser.
//!
//! Text is quads showing glyphs of an atlas texture in the frame's world: each glyph is
//! rasterized once at the size it is drawn, in target pixels, and written into the atlas as it is
//! first needed. The font is Barlow Semi Condensed SemiBold (SIL Open Font License,
//! `assets/fonts/OFL.txt`).

use std::collections::HashMap;
use std::sync::LazyLock;

use render_engine::glam::{Vec2, Vec4};
use render_engine::material::{Sampler, TextureData};
use render_engine::wgpu;

use crate::gfx::{OverlayImage, OverlayQuad, TextureId, World};

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
    /// A whole texture of the frame's world (a map's preview, [`crate::menu`]) stretched over
    /// the rectangle, tinted by `color`, its corners rounded to `radius` pixels.
    Image {
        position: Vec2,
        size: Vec2,
        radius: f32,
        texture: TextureId,
        color: Vec4,
    },
}

impl Item {
    /// Its top left and bottom right corners in frame pixels (a glyph's bitmap's).
    #[must_use]
    pub fn bounds(&self) -> (Vec2, Vec2) {
        match *self {
            Self::Rect { position, size, .. } | Self::Image { position, size, .. } => {
                (position, position + size)
            }
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

/// The glyph atlas' side in texels.
const ATLAS: u32 = 1024;

/// A glyph's bitmap in the atlas and its offset from the pen (target pixels).
#[derive(Clone, Copy, Debug)]
struct Placed {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    xmin: i32,
    ymin: i32,
}

/// The UI's glyph atlas: an `ATLAS`² texture in the frame's world, white with each glyph's
/// coverage in its alpha, the glyphs shelf-packed as they are first drawn. A full atlas starts
/// over (glyphs placed earlier in that frame may show wrongly for the frame).
pub struct Atlas {
    texture: TextureId,
    glyphs: HashMap<(char, u32), Placed>,
    /// The shelf being filled: its next free column, its top and its height.
    shelf: (u32, u32, u32),
}

impl Atlas {
    /// An empty atlas in `world`.
    pub fn new(world: &mut World) -> Self {
        let texels = (ATLAS * ATLAS) as usize;
        let texture = world.add_texture(&TextureData {
            width: ATLAS,
            height: ATLAS,
            format: wgpu::TextureFormat::Rgba8Unorm,
            levels: vec![[255, 255, 255, 0].repeat(texels)],
            // Glyphs are drawn texel for pixel.
            sampler: Sampler {
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Nearest,
                min_filter: wgpu::FilterMode::Nearest,
                mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            },
        });
        Self {
            texture,
            glyphs: HashMap::new(),
            shelf: (0, 0, 0),
        }
    }

    /// `character` at `size` pixels, rasterized and written into the atlas on first use.
    fn place(&mut self, world: &mut World, character: char, size: u32) -> Placed {
        if let Some(placed) = self.glyphs.get(&(character, size)) {
            return *placed;
        }
        #[allow(clippy::cast_precision_loss)]
        let (metrics, coverage) = FONT.rasterize(character, size as f32);
        let (width, height) = (
            u32::try_from(metrics.width).unwrap_or(0).min(ATLAS),
            u32::try_from(metrics.height).unwrap_or(0).min(ATLAS),
        );
        let (mut x, mut y, mut row) = self.shelf;
        if width > 0 && height > 0 {
            if x + width > ATLAS {
                (x, y, row) = (0, y + row + 1, 0);
            }
            if y + height > ATLAS {
                self.glyphs.clear();
                (x, y, row) = (0, 0, 0);
            }
            let pixels: Vec<u8> = coverage
                .iter()
                .take((width * height) as usize)
                .flat_map(|&alpha| [255, 255, 255, alpha])
                .collect();
            world.write_texture(self.texture, (x, y), (width, height), &pixels);
            self.shelf = (x + width + 1, y, row.max(height));
        }
        let placed = Placed {
            x,
            y,
            width,
            height,
            xmin: metrics.xmin,
            ymin: metrics.ymin,
        };
        self.glyphs.insert((character, size), placed);
        placed
    }

    /// `items` (laid out in layout pixels) as overlay quads, the glyphs rasterized at `pixels`
    /// target pixels to a layout pixel and placed on whole target pixels.
    #[allow(clippy::cast_precision_loss)]
    pub fn quads(&mut self, world: &mut World, items: &[Item], pixels: f32) -> Vec<OverlayQuad> {
        let pixels = pixels.max(f32::EPSILON);
        let texel = 1.0 / ATLAS as f32;
        let mut quads = Vec::with_capacity(items.len());
        for item in items {
            match *item {
                Item::Rect {
                    position,
                    size,
                    radius,
                    color,
                } => quads.push(OverlayQuad {
                    position,
                    size,
                    color,
                    corner_radius: radius,
                    image: None,
                }),
                Item::Image {
                    position,
                    size,
                    radius,
                    texture,
                    color,
                } => quads.push(OverlayQuad {
                    position,
                    size,
                    color,
                    corner_radius: radius,
                    image: Some(OverlayImage {
                        texture,
                        uv_min: Vec2::ZERO,
                        uv_max: Vec2::ONE,
                    }),
                }),
                Item::Glyph {
                    pen,
                    character,
                    size,
                    color,
                } => {
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let target_size = (size * pixels).round().max(1.0) as u32;
                    let placed = self.place(world, character, target_size);
                    if placed.width == 0 || placed.height == 0 {
                        continue;
                    }
                    let (w, h) = (placed.width as f32, placed.height as f32);
                    let left = (pen.x * pixels).round() + placed.xmin as f32;
                    let top = (pen.y * pixels).round() - placed.ymin as f32 - h;
                    let uv_min = Vec2::new(placed.x as f32, placed.y as f32) * texel;
                    quads.push(OverlayQuad {
                        position: Vec2::new(left, top) / pixels,
                        size: Vec2::new(w, h) / pixels,
                        color,
                        corner_radius: 0.0,
                        image: Some(OverlayImage {
                            texture: self.texture,
                            uv_min,
                            uv_max: uv_min + Vec2::new(w, h) * texel,
                        }),
                    });
                }
            }
        }
        quads
    }
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
