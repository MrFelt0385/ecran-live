// make_cursor — transforme le logo Sypherine en curseur personnalisé.
// Étapes : détourage (fond noir → transparent) → inclinaison (~40°) → 72px.
// Usage: cargo run --example make_cursor <logo.jpg> <out.png>
use image::{GenericImage, GenericImageView, ImageBuffer, Rgba, RgbaImage};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let src = args.get(1).map(String::as_str).unwrap_or("/tmp/logo_sypherine.jpg");
    let dst = args.get(2).map(String::as_str).unwrap_or("/tmp/sypherine_cursor.png");

    // 1. Charger le logo
    let img = image::open(src)?.to_rgba8();
    println!("Logo chargé: {}x{}", img.width(), img.height());

    // 2. Détourage : fond noir → transparent (seuil progressif par luminance)
    let mut cut = RgbaImage::new(img.width(), img.height());
    for (x, y, p) in img.enumerate_pixels() {
        let lum = 0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32;
        // Fond noir (~0-40) → alpha 0 ; S doré/blanc (lum élevée) → alpha 255
        // Seuil strict (50-140) pour éliminer le bruit d'anti-aliasing JPG
        let alpha = ((lum - 50.0) / 90.0).clamp(0.0, 1.0) * 255.0;
        cut.put_pixel(x, y, Rgba([p[0], p[1], p[2], alpha as u8]));
    }
    // Nettoyage : supprimer les petits amas isolés (< 40 px opaques) qui sont
    // du bruit JPG, pas le S. On garde la plus grande composante connexe.
    cut = keep_largest_blob(&cut);
    println!("Détourage: fond noir → transparent + suppression bruit");

    // 3. Recadrer SERRÉ sur le contenu opaque (le S) — le logo a trop de noir
    // autour, sinon le S serait minuscule dans le curseur final.
    let mut min_x = img.width();
    let mut min_y = img.height();
    let mut max_x = 0u32;
    let mut max_y = 0u32;
    for (x, y, p) in cut.enumerate_pixels() {
        if p[3] > 40 {
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
    }
    let pad = 12u32; // petite marge autour du S
    let bx = min_x.saturating_sub(pad);
    let by = min_y.saturating_sub(pad);
    let bw = (max_x - min_x + 1 + pad * 2).min(cut.width() - bx);
    let bh = (max_y - min_y + 1 + pad * 2).min(cut.height() - by);
    let tight = image::imageops::crop_imm(&cut, bx, by, bw.max(1), bh.max(1)).to_image();
    println!("Recadré serré sur le S: {}x{}", tight.width(), tight.height());

    // 4. Redimensionner à 96px de large (marge pour la rotation)
    let scale = 96.0 / tight.width() as f32;
    let tw = 96u32;
    let th = (tight.height() as f32 * scale).round().max(1.0) as u32;
    let small = image::imageops::resize(&tight, tw, th, image::imageops::FilterType::Lanczos3);
    println!("Redimensionné: {}x{}", small.width(), small.height());

    // 5. Inclinaison de 40° (sens anti-horaire → pointe vers le haut-gauche)
    let rotated = rotate_arbitrary(&small, 40.0);
    println!("Incliné 40°: {}x{}", rotated.width(), rotated.height());

    // 6. Recadrer au centre (carré) pour enlever les coins vides
    let side = rotated.width().min(rotated.height());
    let cx = (rotated.width() - side) / 2;
    let cy = (rotated.height() - side) / 2;
    let cropped = image::imageops::crop_imm(&rotated, cx, cy, side, side).to_image();
    println!("Recadré: {}x{}", cropped.width(), cropped.height());

    // 7. Réduire à 48x48 (taille curseur — réduit d'un tiers vs 72px)
    let final_img = image::imageops::resize(&cropped, 48, 48, image::imageops::FilterType::Lanczos3);
    final_img.save(dst)?;
    println!("✓ Curseur final: {}x{} → {}", final_img.width(), final_img.height(), dst);
    Ok(())
}

/// Supprime les petits amas isolés : garde uniquement la plus grande
/// composante connexe de pixels opaques (le S), élimine le bruit JPG.
fn keep_largest_blob(img: &RgbaImage) -> RgbaImage {
    let (w, h) = img.dimensions();
    let mut visited = vec![false; (w * h) as usize];
    let mut best: Vec<(u32, u32)> = Vec::new();

    for y in 0..h {
        for x in 0..w {
            let idx = (y * w + x) as usize;
            if visited[idx] || img.get_pixel(x, y)[3] < 128 {
                continue;
            }
            // BFS pour trouver la composante
            let mut stack = vec![(x, y)];
            visited[idx] = true;
            let mut blob: Vec<(u32, u32)> = Vec::new();
            while let Some((cx, cy)) = stack.pop() {
                blob.push((cx, cy));
                // 4-voisinage
                let neigh = [
                    (cx.wrapping_sub(1), cy), (cx + 1, cy),
                    (cx, cy.wrapping_sub(1)), (cx, cy + 1),
                ];
                for (nx, ny) in neigh {
                    if nx < w && ny < h {
                        let nidx = (ny * w + nx) as usize;
                        if !visited[nidx] && img.get_pixel(nx, ny)[3] >= 128 {
                            visited[nidx] = true;
                            stack.push((nx, ny));
                        }
                    }
                }
            }
            if blob.len() > best.len() {
                best = blob;
            }
        }
    }

    let blob_size = best.len();
    let mut out = RgbaImage::new(w, h);
    for (x, y) in best {
        out.put_pixel(x, y, *img.get_pixel(x, y));
    }
    println!("Composante principale: {} px (bruit supprimé)", blob_size);
    out
}

/// Rotation d'angle arbitraire (degrés, anti-horaire) autour du centre,
/// interpolation bilinéaire, fond transparent.
fn rotate_arbitrary(img: &RgbaImage, angle_deg: f32) -> RgbaImage {
    let (w, h) = img.dimensions();
    let theta = angle_deg.to_radians();
    let (sin, cos) = theta.sin_cos();

    // Nouvelle taille du bounding box après rotation
    let new_w = (w as f32 * cos.abs() + h as f32 * sin.abs()).ceil().max(1.0) as u32;
    let new_h = (w as f32 * sin.abs() + h as f32 * cos.abs()).ceil().max(1.0) as u32;
    let mut out = RgbaImage::new(new_w, new_h);

    let cx = w as f32 / 2.0;
    let cy = h as f32 / 2.0;
    let ncx = new_w as f32 / 2.0;
    let ncy = new_h as f32 / 2.0;

    for ny in 0..new_h {
        for nx in 0..new_w {
            let dx = nx as f32 - ncx;
            let dy = ny as f32 - ncy;
            // Rotation inverse : source = R^-1 * (dest - centre) + centre
            let sx = dx * cos + dy * sin + cx;
            let sy = -dx * sin + dy * cos + cy;
            if sx >= 0.0 && sx < w as f32 - 1.0 && sy >= 0.0 && sy < h as f32 - 1.0 {
                // Bilinéaire
                let x0 = sx.floor() as u32;
                let y0 = sy.floor() as u32;
                let fx = sx - x0 as f32;
                let fy = sy - y0 as f32;
                let p00 = img.get_pixel(x0, y0);
                let p10 = img.get_pixel((x0 + 1).min(w - 1), y0);
                let p01 = img.get_pixel(x0, (y0 + 1).min(h - 1));
                let p11 = img.get_pixel((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
                let mut px = [0u8; 4];
                for c in 0..4 {
                    let v = (p00[c] as f32 * (1.0 - fx) * (1.0 - fy)
                        + p10[c] as f32 * fx * (1.0 - fy)
                        + p01[c] as f32 * (1.0 - fx) * fy
                        + p11[c] as f32 * fx * fy)
                        .round() as u8;
                    px[c] = v;
                }
                out.put_pixel(nx, ny, Rgba(px));
            }
        }
    }
    out
}
