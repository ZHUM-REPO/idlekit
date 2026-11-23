//! The warning badge for the tray icon: an amber triangle with an exclamation
//! mark in the bottom-right corner. IdleKit shows it while the Steam client
//! isn't signed in to the account selected in the window.

/// How much of the icon's side the badge takes. Large for a badge: the tray
/// shows the icon at 16 to 24 pixels, and the mark has to read there.
const SIDE: f32 = 0.64;
/// The see-through ring between the icon and the badge, and the badge's dark
/// edge, as parts of the badge's side.
const GAP: f32 = 0.06;
const EDGE: f32 = 0.085;
const AMBER: [f32; 3] = [245.0, 184.0, 61.0];
const DARK: [f32; 3] = [28.0, 28.0, 28.0];
/// Samples per pixel along each axis, for smooth edges at tray size.
const SAMPLES: usize = 4;

/// The picture with the badge drawn in its bottom-right corner. `rgba` is
/// `width` × `height` pixels of 4 bytes each, as Tauri keeps icons. A picture
/// that doesn't add up comes back as it was.
pub fn with_warning_badge(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    let mut out = rgba.to_vec();
    if w == 0 || h == 0 || rgba.len() != w * h * 4 {
        return out;
    }
    let badge = Badge::new(w as f32, h as f32);
    let first_x = badge.left.floor().max(0.0) as usize;
    let first_y = badge.top.floor().max(0.0) as usize;
    for y in first_y..h {
        for x in first_x..w {
            let cover = badge.cover(x as f32, y as f32);
            if cover.ring == 0.0 {
                continue;
            }
            let at = (y * w + x) * 4;
            let mut px = [out[at] as f32, out[at + 1] as f32, out[at + 2] as f32, out[at + 3] as f32 / 255.0];
            // The icon gives way to the badge and the ring around it.
            px[3] *= 1.0 - cover.ring;
            over(&mut px, DARK, cover.edge);
            over(&mut px, AMBER, cover.fill);
            over(&mut px, DARK, cover.mark);
            out[at..at + 4].copy_from_slice(&[
                px[0].round().clamp(0.0, 255.0) as u8,
                px[1].round().clamp(0.0, 255.0) as u8,
                px[2].round().clamp(0.0, 255.0) as u8,
                (px[3] * 255.0).round().clamp(0.0, 255.0) as u8,
            ]);
        }
    }
    out
}

/// Paints `color` at `alpha` over a pixel (straight, not premultiplied, alpha).
fn over(px: &mut [f32; 4], color: [f32; 3], alpha: f32) {
    if alpha <= 0.0 {
        return;
    }
    let below = px[3] * (1.0 - alpha);
    let total = alpha + below;
    for c in 0..3 {
        px[c] = (color[c] * alpha + px[c] * below) / total;
    }
    px[3] = total;
}

/// How much of a pixel each part of the badge covers, from 0 to 1.
struct Cover {
    /// The whole badge with the ring around it.
    ring: f32,
    edge: f32,
    fill: f32,
    mark: f32,
}

struct Badge {
    left: f32,
    top: f32,
    /// The outer triangle (the ring's outer line): its corners.
    corners: [(f32, f32); 3],
    gap: f32,
    edge: f32,
    /// The exclamation mark: its bar from `bar_top` to `bar_bottom`, and its dot.
    mark_x: f32,
    bar_half: f32,
    bar_top: f32,
    bar_bottom: f32,
    dot_y: f32,
    dot_radius: f32,
}

impl Badge {
    fn new(w: f32, h: f32) -> Self {
        let side = w.min(h) * SIDE;
        let (left, bottom) = (w - side, h);
        // An equilateral triangle as wide as the badge, standing on the icon's bottom edge.
        let height = side * 3f32.sqrt() / 2.0;
        let top = bottom - height;
        let corners = [(left + side / 2.0, top), (left + side, bottom), (left, bottom)];
        let (gap, edge) = (side * GAP, side * EDGE);
        // The amber triangle inside the ring and the edge is a smaller copy
        // of the outer one, about the same centre.
        let centre_y = bottom - height / 3.0;
        let inner_height = height - 3.0 * (gap + edge);
        let inner_top = centre_y - inner_height * 2.0 / 3.0;
        Badge {
            left,
            top,
            corners,
            gap,
            edge,
            mark_x: left + side / 2.0,
            bar_half: inner_height * 0.075,
            bar_top: inner_top + inner_height * 0.30,
            bar_bottom: inner_top + inner_height * 0.66,
            dot_y: inner_top + inner_height * 0.81,
            dot_radius: inner_height * 0.085,
        }
    }

    /// How far a point lies inside the outer triangle: the distance to its
    /// nearest side, negative outside.
    fn inside(&self, x: f32, y: f32) -> f32 {
        let mut nearest = f32::MAX;
        for i in 0..3 {
            let (ax, ay) = self.corners[i];
            let (bx, by) = self.corners[(i + 1) % 3];
            let (dx, dy) = (bx - ax, by - ay);
            let length = (dx * dx + dy * dy).sqrt();
            // The corners go clockwise on screen, so the inside is on the right of each side.
            let distance = ((x - ax) * dy - (y - ay) * dx) / length;
            nearest = nearest.min(-distance);
        }
        nearest
    }

    fn in_mark(&self, x: f32, y: f32) -> bool {
        let across = (x - self.mark_x).abs();
        let in_bar = across <= self.bar_half && y >= self.bar_top && y <= self.bar_bottom;
        let (dx, dy) = (x - self.mark_x, y - self.dot_y);
        in_bar || dx * dx + dy * dy <= self.dot_radius * self.dot_radius
    }

    fn cover(&self, x: f32, y: f32) -> Cover {
        let mut hits = [0u32; 4];
        for sy in 0..SAMPLES {
            for sx in 0..SAMPLES {
                let px = x + (sx as f32 + 0.5) / SAMPLES as f32;
                let py = y + (sy as f32 + 0.5) / SAMPLES as f32;
                let depth = self.inside(px, py);
                if depth < 0.0 {
                    continue;
                }
                hits[0] += 1;
                if depth >= self.gap {
                    hits[1] += 1;
                }
                if depth >= self.gap + self.edge {
                    hits[2] += 1;
                    if self.in_mark(px, py) {
                        hits[3] += 1;
                    }
                }
            }
        }
        let share = |n: u32| n as f32 / (SAMPLES * SAMPLES) as f32;
        Cover { ring: share(hits[0]), edge: share(hits[1]), fill: share(hits[2]), mark: share(hits[3]) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn icon(size: usize) -> Vec<u8> {
        // An opaque blue square, like an app icon.
        (0..size * size).flat_map(|_| [40u8, 90, 200, 255]).collect()
    }

    fn pixel(rgba: &[u8], size: usize, x: usize, y: usize) -> [u8; 4] {
        let at = (y * size + x) * 4;
        [rgba[at], rgba[at + 1], rgba[at + 2], rgba[at + 3]]
    }

    #[test]
    fn the_badge_sits_in_the_bottom_right_corner_and_leaves_the_rest() {
        for size in [16usize, 24, 32, 48, 256] {
            let before = icon(size);
            let after = with_warning_badge(&before, size as u32, size as u32);
            assert_eq!(after.len(), before.len());
            let changed: Vec<(usize, usize)> = (0..size * size)
                .filter(|i| before[i * 4..i * 4 + 4] != after[i * 4..i * 4 + 4])
                .map(|i| (i % size, i / size))
                .collect();
            assert!(!changed.is_empty(), "{size}: nothing drawn");
            let corner = (size as f32 * (1.0 - SIDE)).floor() as usize;
            assert!(changed.iter().all(|&(x, y)| x >= corner && y >= corner), "{size}: drawn outside the corner");
            // The top-left of the icon is untouched.
            assert_eq!(pixel(&after, size, 0, 0), [40, 90, 200, 255]);
        }
    }

    #[test]
    fn the_badge_reads_as_an_amber_triangle_with_a_dark_mark() {
        let size = 256;
        let after = with_warning_badge(&icon(size), size as u32, size as u32);
        let badge = Badge::new(size as f32, size as f32);
        let x = badge.mark_x as usize;
        // The bar of the exclamation mark is dark, beside it is amber.
        let bar = pixel(&after, size, x, ((badge.bar_top + badge.bar_bottom) / 2.0) as usize);
        assert!(bar[0] < 60 && bar[3] == 255, "{bar:?}");
        let beside = pixel(&after, size, x + (badge.bar_half * 4.0) as usize, ((badge.bar_top + badge.bar_bottom) / 2.0) as usize + 20);
        assert_eq!(beside, [245, 184, 61, 255]);
        // The dot is dark; between the bar and the dot it is amber.
        let dot = pixel(&after, size, x, badge.dot_y as usize);
        assert!(dot[0] < 60, "{dot:?}");
        let between = pixel(&after, size, x, ((badge.bar_bottom + badge.dot_y - badge.dot_radius) / 2.0) as usize);
        assert_eq!(between, [245, 184, 61, 255]);
        // Between the triangle and the icon's edge, the icon gives way to a see-through ring.
        assert_eq!(pixel(&after, size, x, size - 1)[3], 0);
        // Outside the triangle the icon stays.
        assert_eq!(pixel(&after, size, (badge.left + 1.0) as usize, size - 4), [40, 90, 200, 255]);
    }

    #[test]
    fn a_picture_that_doesnt_add_up_comes_back_as_it_was() {
        assert_eq!(with_warning_badge(&[1, 2, 3], 4, 4), vec![1, 2, 3]);
        assert!(with_warning_badge(&[], 0, 0).is_empty());
    }
}
