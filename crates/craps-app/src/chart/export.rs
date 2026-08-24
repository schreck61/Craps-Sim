// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! Device-independent chart export (spec Part I §7, Part II §5).
//!
//! ⌘⇧C / ⌘⇧S render the focused Chart Frame's epaint shape list offscreen at
//! 2× pixels-per-point — never a screen grab — so an export is identical on
//! every platform and monitor. The caller appends the chrome (title,
//! caption, Scenario Sentence, seed, n) as text shapes before handing over
//! the bundle, so provenance is baked into the pixels: no image can escape
//! its assumptions, and it survives cropping and re-posting.
//!
//! Coordinates: shapes arrive in frame-local logical points. [`rasterize`]
//! translates `bundle.rect.min` to the origin, hands the export scale to
//! epaint's [`Tessellator`] (so feather widths and pixel-grid rounding are
//! computed for the output raster, not the screen), and multiplies vertex
//! positions and clip rects by the same scale while scan-converting. Vertex
//! colors are premultiplied sRGBA (epaint's convention); triangles are
//! filled with edge functions and the top-left rule, Gouraud-interpolated in
//! gamma space, multiplied by a bilinear sample of the font atlas, and
//! composited source-over — the same arithmetic as egui's GPU backends.

use egui::epaint::{ClippedPrimitive, ClippedShape, Primitive, TessellationOptions, Tessellator};
use egui::{Color32, ColorImage, Pos2, Rect, TextureId, Vec2};

/// Everything needed to rasterize one chart frame off-screen.
pub struct ExportBundle {
    /// The frame's full shape list in frame-local coordinates, including
    /// chrome text (title, caption, scenario sentence, seed, n) which the
    /// CALLER has already appended as text shapes.
    pub shapes: Vec<egui::Shape>,
    /// The frame rect the shapes were laid out in (logical points).
    pub rect: egui::Rect,
    /// Background fill (theme surface color).
    pub background: egui::Color32,
}

/// Rasterize at `pixels_per_point` (the product uses 2.0): tessellate with
/// epaint, then software-rasterize the triangles (vertex-color Gouraud ×
/// font-atlas texture sampling with bilinear filtering) into an RGBA image.
///
/// The context must have completed at least one pass so its fonts exist, and
/// any galleys in the bundle must have been laid out through this context so
/// their glyphs are resident in the atlas (both are always true in the app).
pub fn rasterize(
    ctx: &egui::Context,
    bundle: &ExportBundle,
    pixels_per_point: f32,
) -> egui::ColorImage {
    let size = bundle.rect.size();
    let clip = Rect::from_min_size(Pos2::ZERO, size);
    let offset = -bundle.rect.min.to_vec2();
    let clipped: Vec<ClippedShape> = bundle
        .shapes
        .iter()
        .cloned()
        .map(|mut shape| {
            shape.translate(offset);
            ClippedShape {
                clip_rect: clip,
                shape,
            }
        })
        .collect();
    rasterize_clipped(ctx, clipped, size, bundle.background, pixels_per_point)
}

/// PNG-encode an [`egui::ColorImage`] (RGBA 8-bit, alpha unmultiplied).
pub fn encode_png(img: &egui::ColorImage) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, img.size[0] as u32, img.size[1] as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc
        .write_header()
        .expect("writing a PNG header to a Vec cannot fail");
    let bytes: Vec<u8> = img
        .pixels
        .iter()
        .flat_map(|c| c.to_srgba_unmultiplied())
        .collect();
    writer
        .write_image_data(&bytes)
        .expect("writing PNG image data to a Vec cannot fail");
    writer
        .finish()
        .expect("finishing a PNG in a Vec cannot fail");
    out
}

/// Copy to the system clipboard (arboard::ImageData).
pub fn copy_to_clipboard(img: &egui::ColorImage) -> Result<(), String> {
    // arboard covers macOS, Windows, and Linux (X11/Wayland); guard the body
    // so a hypothetical wasm target still compiles the module.
    #[cfg(target_arch = "wasm32")]
    {
        let _ = img;
        Err("clipboard image export is not supported on this platform".to_owned())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let bytes: Vec<u8> = img
            .pixels
            .iter()
            .flat_map(|c| c.to_srgba_unmultiplied())
            .collect();
        let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
        clipboard
            .set_image(arboard::ImageData {
                width: img.size[0],
                height: img.size[1],
                bytes: bytes.into(),
            })
            .map_err(|e| e.to_string())
    }
}

/// Ask for a path (rfd native save dialog, default file name given) and
/// write the PNG. Returns Ok(None) if the user cancelled.
pub fn save_with_dialog(
    img: &egui::ColorImage,
    default_name: &str,
) -> Result<Option<std::path::PathBuf>, String> {
    // rfd's blocking dialog covers macOS, Windows, and Linux; guard the body
    // so a hypothetical wasm target still compiles the module.
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (img, default_name);
        Err("the save dialog is not supported on this platform".to_owned())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("PNG image", &["png"])
            .set_file_name(default_name)
            .save_file()
        else {
            return Ok(None);
        };
        std::fs::write(&path, encode_png(img)).map_err(|e| e.to_string())?;
        Ok(Some(path))
    }
}

// ---------------------------------------------------------------------------
// The rasterizer
// ---------------------------------------------------------------------------

/// A framebuffer pixel / shading sample: premultiplied sRGBA in 0..=1,
/// gamma space — exactly the values egui's GPU backends interpolate and
/// blend, just as floats instead of a fixed-point framebuffer.
type Rgba = [f32; 4];

/// A vertex prepared for scan conversion: position already in output pixels.
struct ExportVertex {
    x: f32,
    y: f32,
    u: f32,
    v: f32,
    color: Rgba,
}

/// Tessellate and scan-convert `clipped` (frame-local points, clip rects in
/// the same space) into an opaque image of `size_points × pixels_per_point`.
fn rasterize_clipped(
    ctx: &egui::Context,
    clipped: Vec<ClippedShape>,
    size_points: Vec2,
    background: Color32,
    pixels_per_point: f32,
) -> ColorImage {
    // The current font atlas, straight from the context: an RGBA image whose
    // glyphs are white with premultiplied coverage alpha and whose (0,0)
    // texel is solid white (epaint::WHITE_UV) — so untextured geometry can
    // share the one sampling path.
    let (font_tex_size, atlas) = ctx.fonts(|f| (f.font_image_size(), f.image()));

    // epaint 0.36: Tessellator::new(pixels_per_point, options, font_tex_size,
    // prepared_discs). The prepared-disc list is documented safe to leave
    // empty (small circles then tessellate as feathered paths, which suits a
    // static export). Feathering at the default 1 physical pixel gives the
    // anti-aliasing; passing the export scale here sizes that feather (and
    // pixel-grid rounding) for the output raster.
    let options = TessellationOptions {
        feathering: true,
        ..Default::default()
    };
    let primitives = Tessellator::new(pixels_per_point, options, font_tex_size, Vec::new())
        .tessellate_shapes(clipped);

    let width = ((size_points.x * pixels_per_point).round() as usize).max(1);
    let height = ((size_points.y * pixels_per_point).round() as usize).max(1);
    let bg = premultiplied(background);
    let mut fb: Vec<Rgba> = vec![bg; width * height];

    for ClippedPrimitive {
        clip_rect,
        primitive,
    } in &primitives
    {
        let Primitive::Mesh(mesh) = primitive else {
            continue; // paint callbacks cannot exist in a chart shape list
        };

        // Scissor: clip rect (points) → whole pixels, like glScissor in the
        // GPU backends. A pixel is in if its index is in [min, max).
        let sx0 = ((clip_rect.min.x * pixels_per_point).round() as i64).clamp(0, width as i64);
        let sy0 = ((clip_rect.min.y * pixels_per_point).round() as i64).clamp(0, height as i64);
        let sx1 = ((clip_rect.max.x * pixels_per_point).round() as i64).clamp(0, width as i64);
        let sy1 = ((clip_rect.max.y * pixels_per_point).round() as i64).clamp(0, height as i64);
        if sx0 >= sx1 || sy0 >= sy1 {
            continue;
        }

        // Only the font atlas (TextureId::Managed(0)) is sampled; any other
        // texture falls back to white, leaving pure vertex color.
        let texture = (mesh.texture_id == TextureId::default()).then_some(&atlas);

        let verts: Vec<ExportVertex> = mesh
            .vertices
            .iter()
            .map(|v| ExportVertex {
                x: v.pos.x * pixels_per_point,
                y: v.pos.y * pixels_per_point,
                u: v.uv.x,
                v: v.uv.y,
                color: premultiplied(v.color),
            })
            .collect();

        for tri in mesh.indices.as_chunks::<3>().0 {
            fill_triangle(
                &mut fb,
                width,
                (sx0, sy0, sx1, sy1),
                [
                    &verts[tri[0] as usize],
                    &verts[tri[1] as usize],
                    &verts[tri[2] as usize],
                ],
                texture,
            );
        }
    }

    let pixels: Vec<Color32> = fb
        .iter()
        .map(|p| {
            let ch = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            Color32::from_rgba_premultiplied(ch(p[0]), ch(p[1]), ch(p[2]), ch(p[3]))
        })
        .collect();
    ColorImage::new([width, height], pixels)
}

fn premultiplied(c: Color32) -> Rgba {
    [
        c.r() as f32 / 255.0,
        c.g() as f32 / 255.0,
        c.b() as f32 / 255.0,
        c.a() as f32 / 255.0,
    ]
}

/// Signed double-area of (a, b, c); positive when the interior of a→b→c is
/// on the +side of every edge in y-down pixel coordinates.
fn edge(ax: f32, ay: f32, bx: f32, by: f32, px: f32, py: f32) -> f32 {
    (bx - ax) * (py - ay) - (by - ay) * (px - ax)
}

/// Top-left fill rule for the positive-interior orientation: a pixel exactly
/// on an edge belongs to the triangle only when the edge is a top edge
/// (horizontal, interior below) or a left edge (going up in y-down coords).
/// Shared edges between adjacent triangles are thus covered exactly once.
fn is_top_left(ax: f32, ay: f32, bx: f32, by: f32) -> bool {
    (ay == by && bx > ax) || by < ay
}

/// Fill one triangle: bounding box intersected with the scissor, three edge
/// functions stepped incrementally across each row, barycentric Gouraud
/// color and UV, bilinear atlas sample, source-over blend.
fn fill_triangle(
    fb: &mut [Rgba],
    fb_width: usize,
    scissor: (i64, i64, i64, i64),
    tri: [&ExportVertex; 3],
    texture: Option<&ColorImage>,
) {
    let [v0, mut v1, mut v2] = tri;
    let mut area = edge(v0.x, v0.y, v1.x, v1.y, v2.x, v2.y);
    if area == 0.0 {
        return; // degenerate
    }
    if area < 0.0 {
        std::mem::swap(&mut v1, &mut v2); // normalize to positive interior
        area = -area;
    }

    // Pixel-center bounding box, clamped to the scissor.
    let (sx0, sy0, sx1, sy1) = scissor;
    let min_x = v0.x.min(v1.x).min(v2.x);
    let max_x = v0.x.max(v1.x).max(v2.x);
    let min_y = v0.y.min(v1.y).min(v2.y);
    let max_y = v0.y.max(v1.y).max(v2.y);
    let x0 = ((min_x - 0.5).ceil() as i64).max(sx0);
    let x1 = (((max_x - 0.5).floor() as i64) + 1).min(sx1);
    let y0 = ((min_y - 0.5).ceil() as i64).max(sy0);
    let y1 = (((max_y - 0.5).floor() as i64) + 1).min(sy1);
    if x0 >= x1 || y0 >= y1 {
        return;
    }

    // Edge functions as e(x, y) = A·x + B·y + C, stepped by A along a row.
    // e01 is opposite v2, e12 opposite v0, e20 opposite v1.
    let coeffs = |a: &ExportVertex, b: &ExportVertex| {
        let (aa, bb) = (-(b.y - a.y), b.x - a.x);
        (aa, bb, (b.y - a.y) * a.x - (b.x - a.x) * a.y)
    };
    let (a01, b01, c01) = coeffs(v0, v1);
    let (a12, b12, c12) = coeffs(v1, v2);
    let (a20, b20, c20) = coeffs(v2, v0);
    let tl01 = is_top_left(v0.x, v0.y, v1.x, v1.y);
    let tl12 = is_top_left(v1.x, v1.y, v2.x, v2.y);
    let tl20 = is_top_left(v2.x, v2.y, v0.x, v0.y);
    let inv_area = 1.0 / area;

    let (px0, py0) = (x0 as f32 + 0.5, y0 as f32 + 0.5);
    let mut row01 = a01 * px0 + b01 * py0 + c01;
    let mut row12 = a12 * px0 + b12 * py0 + c12;
    let mut row20 = a20 * px0 + b20 * py0 + c20;

    for y in y0..y1 {
        let (mut e01, mut e12, mut e20) = (row01, row12, row20);
        let row = &mut fb[y as usize * fb_width..];
        for x in x0..x1 {
            let inside = (e01 > 0.0 || (e01 == 0.0 && tl01))
                && (e12 > 0.0 || (e12 == 0.0 && tl12))
                && (e20 > 0.0 || (e20 == 0.0 && tl20));
            if inside {
                // Barycentric weights: each vertex weighs by the edge
                // function opposite it.
                let w0 = e12 * inv_area;
                let w1 = e20 * inv_area;
                let w2 = e01 * inv_area;
                let mut src = [0.0f32; 4];
                for (i, ch) in src.iter_mut().enumerate() {
                    *ch = w0 * v0.color[i] + w1 * v1.color[i] + w2 * v2.color[i];
                }
                if let Some(tex) = texture {
                    let u = w0 * v0.u + w1 * v1.u + w2 * v2.u;
                    let v = w0 * v0.v + w1 * v1.v + w2 * v2.v;
                    let t = sample_bilinear(tex, u, v);
                    for i in 0..4 {
                        src[i] *= t[i];
                    }
                }
                // Source-over with premultiplied alpha (ONE,
                // ONE_MINUS_SRC_ALPHA — egui's blend equation).
                let dst = &mut row[x as usize];
                let inv_a = 1.0 - src[3];
                for i in 0..4 {
                    dst[i] = src[i] + dst[i] * inv_a;
                }
            }
            e01 += a01;
            e12 += a12;
            e20 += a20;
        }
        row01 += b01;
        row12 += b12;
        row20 += b20;
    }
}

/// Bilinear sample of the font atlas at normalized UV, clamped at the edges
/// (matching egui's clamping sampler, which the WHITE_UV corner relies on).
/// Values are returned as stored — gamma space, premultiplied — because
/// that is what egui's shaders multiply with vertex color.
fn sample_bilinear(tex: &ColorImage, u: f32, v: f32) -> Rgba {
    let (w, h) = (tex.size[0] as i64, tex.size[1] as i64);
    let x = u * w as f32 - 0.5;
    let y = v * h as f32 - 0.5;
    let (xf, yf) = (x.floor(), y.floor());
    let (fx, fy) = (x - xf, y - yf);
    let x0 = (xf as i64).clamp(0, w - 1) as usize;
    let x1 = (xf as i64 + 1).clamp(0, w - 1) as usize;
    let y0 = (yf as i64).clamp(0, h - 1) as usize;
    let y1 = (yf as i64 + 1).clamp(0, h - 1) as usize;
    let texel = |xi: usize, yi: usize| premultiplied(tex.pixels[yi * tex.size[0] + xi]);
    let (t00, t10) = (texel(x0, y0), texel(x1, y0));
    let (t01, t11) = (texel(x0, y1), texel(x1, y1));
    let mut out = [0.0f32; 4];
    for i in 0..4 {
        let top = t00[i] + fx * (t10[i] - t00[i]);
        let bot = t01[i] + fx * (t11[i] - t01[i]);
        out[i] = top + fy * (bot - top);
    }
    out
}

// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use egui::text::LayoutJob;
    use egui::{pos2, vec2, FontId, Shape};

    /// A bare headless context with fonts primed: `Context::fonts` panics
    /// until the first pass has run, so run one empty pass. The pixels-per-
    /// point is set first so galleys laid out afterwards match the export
    /// scale exactly (it takes effect at the start of the next pass).
    fn test_ctx(pixels_per_point: f32) -> egui::Context {
        let ctx = egui::Context::default();
        ctx.set_pixels_per_point(pixels_per_point);
        ctx.begin_pass(egui::RawInput::default());
        let _ = ctx.end_pass();
        ctx
    }

    #[test]
    fn rect_and_circle_golden() {
        let ctx = test_ctx(2.0);
        let bg = Color32::from_rgb(0x12, 0x14, 0x17);
        let red = Color32::from_rgb(0xE5, 0x48, 0x4D);
        let teal = Color32::from_rgb(0x3E, 0xB8, 0xA5);
        // A frame rect away from the origin proves the translation.
        let frame = Rect::from_min_size(pos2(100.0, 50.0), vec2(200.0, 100.0));
        let bundle = ExportBundle {
            shapes: vec![
                Shape::rect_filled(
                    Rect::from_min_max(pos2(110.0, 60.0), pos2(160.0, 90.0)),
                    0.0,
                    red,
                ),
                Shape::circle_filled(pos2(220.0, 100.0), 16.0, teal),
            ],
            rect: frame,
            background: bg,
        };
        let img = rasterize(&ctx, &bundle, 2.0);
        assert_eq!(img.size, [400, 200]);
        let px = |x: usize, y: usize| img.pixels[y * 400 + x];

        // Point p lands on pixel ((p.x − 100)·2, (p.y − 50)·2).
        assert_eq!(px(70, 50), red, "rect center");
        assert_eq!(px(240, 100), teal, "circle center");
        assert_eq!(px(1, 1), bg, "background corner");

        // Feathering: somewhere on the circle's rim a pixel must sit
        // strictly between background and fill.
        let blended = (60..140).any(|y| {
            (200..280).any(|x| {
                let c = px(x, y);
                c.g() > bg.g() && c.g() < teal.g()
            })
        });
        assert!(blended, "no antialiased pixel found on the circle rim");
    }

    #[test]
    fn text_renders_nonbackground_pixels() {
        let ctx = test_ctx(2.0);
        let bg = Color32::from_rgb(0x1B, 0x1A, 0x17);
        let ink = Color32::from_rgb(0xED, 0xE9, 0xDF);
        let galley = ctx.fonts_mut(|f| {
            f.layout_job(LayoutJob::simple_singleline(
                "n = 10 000 · seed 0xC0FFEE".to_owned(),
                FontId::proportional(15.0),
                ink,
            ))
        });
        let size = galley.size();
        assert!(size.x > 0.0 && size.y > 0.0, "galley laid out empty");
        let bundle = ExportBundle {
            shapes: vec![Shape::galley(pos2(8.0, 8.0), galley, ink)],
            rect: Rect::from_min_size(Pos2::ZERO, vec2(260.0, 40.0)),
            background: bg,
        };
        let img = rasterize(&ctx, &bundle, 2.0);

        // The text box must contain a healthy cluster of non-background
        // pixels (font-atlas sampling works end to end).
        let x1 = (((8.0 + size.x) * 2.0) as usize).min(img.size[0]);
        let y1 = (((8.0 + size.y) * 2.0) as usize).min(img.size[1]);
        let lit = (16..y1)
            .flat_map(|y| (16..x1).map(move |x| (x, y)))
            .filter(|&(x, y)| img.pixels[y * img.size[0] + x] != bg)
            .count();
        assert!(lit > 50, "only {lit} non-background pixels in the text box");
    }

    #[test]
    fn png_roundtrip() {
        let img = ColorImage::new([7, 5], vec![Color32::from_rgb(10, 200, 30); 35]);
        let bytes = encode_png(&img);
        assert_eq!(
            &bytes[..8],
            &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A],
            "PNG magic"
        );
        let decoder = png::Decoder::new(std::io::Cursor::new(&bytes[..]));
        let mut reader = decoder.read_info().expect("decodable PNG");
        let mut buf = vec![0u8; reader.output_buffer_size().expect("known size")];
        let info = reader.next_frame(&mut buf).expect("decodable frame");
        assert_eq!((info.width, info.height), (7, 5));
        assert_eq!(&buf[..4], &[10, 200, 30, 255]);
    }

    #[test]
    fn clip_rect_respected() {
        let ctx = test_ctx(2.0);
        let bg = Color32::from_rgb(0x12, 0x14, 0x17);
        let blue = Color32::from_rgb(0x58, 0xA6, 0xFF);
        // A rect covering the whole frame, scissored to a small window.
        let clipped = vec![ClippedShape {
            clip_rect: Rect::from_min_max(pos2(20.0, 20.0), pos2(60.0, 40.0)),
            shape: Shape::rect_filled(
                Rect::from_min_size(Pos2::ZERO, vec2(100.0, 60.0)),
                0.0,
                blue,
            ),
        }];
        let img = rasterize_clipped(&ctx, clipped, vec2(100.0, 60.0), bg, 2.0);
        assert_eq!(img.size, [200, 120]);
        let px = |x: usize, y: usize| img.pixels[y * 200 + x];

        assert_eq!(px(80, 60), blue, "inside the clip window");
        assert_eq!(px(20, 60), bg, "left of the clip window");
        assert_eq!(px(140, 60), bg, "right of the clip window");
        assert_eq!(px(80, 20), bg, "above the clip window");
        assert_eq!(px(80, 100), bg, "below the clip window");
    }
}
