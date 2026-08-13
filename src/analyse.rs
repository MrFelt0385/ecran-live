// ═══════════════════════════════════════════════════════════════════════════
// analyse.rs — ANALYSE PIXEL NATIVE (portage des scripts Python /tmp/*.py)
// Le tout-en-Rust : plus de boucles Python, un seul binaire.
// Remplacés : analyse_px.py, cluster_jaune.py, lire_compteur.py,
//             analyse_diag.py, localiser_bouton*.py
// ═══════════════════════════════════════════════════════════════════════════

use image::GenericImageView;
use image::ImageEncoder;
use std::collections::HashMap;

/// Critère de couleur : test sur un pixel RGB.
#[derive(Clone, Copy)]
pub struct Critere {
    pub nom: &'static str,
    pub r_min: u8, pub g_min: u8, pub b_min: u8,
    pub r_max: u8, pub g_max: u8, pub b_max: u8,
}

impl Critere {
    pub fn jaune() -> Self {
        Critere { nom: "jaune", r_min: 200, g_min: 150, b_min: 0, r_max: 255, g_max: 255, b_max: 130 }
    }
    pub fn blanc_vif() -> Self {
        Critere { nom: "blanc_vif", r_min: 230, g_min: 230, b_min: 230, r_max: 255, g_max: 255, b_max: 255 }
    }
    pub fn rose() -> Self {
        Critere { nom: "rose", r_min: 220, g_min: 0, b_min: 60, r_max: 255, g_max: 110, b_max: 140 }
    }
    pub fn teste(&self, r: u8, g: u8, b: u8) -> bool {
        r >= self.r_min && r <= self.r_max
            && g >= self.g_min && g <= self.g_max
            && b >= self.b_min && b <= self.b_max
    }
}

/// Charge une image PNG/JPEG depuis le disque.
pub fn charger(chemin: &str) -> Result<image::DynamicImage, String> {
    image::open(chemin).map_err(|e| format!("impossible d'ouvrir {} : {}", chemin, e))
}

/// Détection d'une couleur : grille CELL×CELL, comptage échantillonné pas=2,
/// cellules denses, regroupement en blocs, centroïdes pondérés.
/// Équivalent de analyse_px.py + cluster_jaune.py.
pub fn cluster_couleur(img: &image::DynamicImage, crit: &Critere, cell: u32, min_dense: u32) -> Vec<Bloc> {
    let (w, h) = (img.width(), img.height());
    let mut cellules: HashMap<(u32, u32), u32> = HashMap::new();
    // Échantillonnage pas=2 (comme le script Python : moitié des pixels, rapide)
    let mut y = 0;
    while y < h {
        let mut x = 0;
        while x < w {
            let px = img.get_pixel(x, y);
            let (r, g, b) = (px[0], px[1], px[2]);
            if crit.teste(r, g, b) {
                *cellules.entry((x / cell, y / cell)).or_insert(0) += 1;
            }
            x += 2;
        }
        y += 2;
    }

    // Cellules denses triées par densité décroissante
    let mut dense: Vec<(u32, u32, u32)> = cellules
        .into_iter()
        .filter(|(_, n)| *n > min_dense)
        .map(|((cx, cy), n)| (cx, cy, n))
        .collect();
    dense.sort_by(|a, b| b.2.cmp(&a.2));

    // Regroupement : blocs adjacents (≤2 cellules), centre moyen approx,
    // pixels cumulés (équivalent cluster_jaune.py)
    let mut blocs: Vec<Bloc> = Vec::new();
    for (cx, cy, n) in dense {
        let mut place = false;
        for b in blocs.iter_mut() {
            let dcx = b.cx as i64 - cx as i64;
            let dcy = b.cy as i64 - cy as i64;
            if dcx.abs() <= 2 && dcy.abs() <= 2 {
                b.cx = (b.cx + cx) / 2;
                b.cy = (b.cy + cy) / 2;
                b.pixels += n;
                place = true;
                break;
            }
        }
        if !place {
            blocs.push(Bloc { cx, cy, pixels: n });
        }
    }
    blocs.sort_by(|a, b| b.pixels.cmp(&a.pixels));
    blocs
}

/// Un bloc de pixels d'une couleur, en coordonnées cellules + pixels.
pub struct Bloc {
    pub cx: u32,
    pub cy: u32,
    pub pixels: u32,
}

impl Bloc {
    /// Centre du bloc en pixels d'image (centre de la cellule).
    pub fn centre(&self, cell: u32) -> (u32, u32) {
        (self.cx * cell + cell / 2, self.cy * cell + cell / 2)
    }
    pub fn zone(&self, cell: u32) -> (u32, u32, u32, u32) {
        (self.cx * cell, self.cy * cell, self.cx * cell + cell, self.cy * cell + cell)
    }
}

/// Analyse complète : pour chaque critère donné, cluster + centroïdes.
pub fn analyser(chemin: &str, criteres: &[Critere], cell: u32, min_dense: u32) -> Result<(), String> {
    let img = charger(chemin)?;
    let (w, h) = (img.width(), img.height());
    println!("Image {} : {}x{}", chemin, w, h);
    for crit in criteres {
        let blocs = cluster_couleur(&img, crit, cell, min_dense);
        println!("\n=== {} ({} bloc(s) dense(s)) ===", crit.nom, blocs.len());
        for (i, b) in blocs.iter().enumerate().take(12) {
            let (cx, cy) = b.centre(cell);
            let (x0, y0, x1, y1) = b.zone(cell);
            println!(
                "  bloc {}: centre=({}, {}) zone=({},{})-({},{}) pixels={}",
                i, cx, cy, x0, y0, x1, y1, b.pixels
            );
        }
        if blocs.is_empty() {
            println!("  (aucun pixel trouvé)");
        }
    }
    Ok(())
}

/// Diff pixel entre deux images : nombre de pixels différents + bbox du
/// changement. Équivalent de analyse_diag.py — la VÉRITÉ TERRAIN pour
/// vérifier qu'un clic a modifié l'écran.
pub fn diff(
    chemin_a: &str,
    chemin_b: &str,
    zone: Option<(u32, u32, u32, u32)>,
) -> Result<(), String> {
    let a = charger(chemin_a)?;
    let b = charger(chemin_b)?;
    if a.dimensions() != b.dimensions() {
        println!("DIFF: dimensions différentes {}x{} vs {}x{} — comparaison impossible",
                 a.width(), a.height(), b.width(), b.height());
        return Ok(());
    }
    let (w, h) = a.dimensions();
    let (zx0, zy0, zx1, zy1) = zone.unwrap_or((0, 0, w, h));
    let (zx0, zy0) = (zx0.min(w), zy0.min(h));
    let (zx1, zy1) = (zx1.min(w), zy1.min(h));

    let mut differents: u64 = 0;
    let mut minx = u32::MAX; let mut miny = u32::MAX;
    let mut maxx = 0u32; let mut maxy = 0u32;
    let mut seuil: i64 = 0;

    for y in zy0..zy1 {
        for x in zx0..zx1 {
            let pa = a.get_pixel(x, y);
            let pb = b.get_pixel(x, y);
            let d = (pa[0] as i64 - pb[0] as i64).abs()
                + (pa[1] as i64 - pb[1] as i64).abs()
                + (pa[2] as i64 - pb[2] as i64).abs();
            if d > 30 {
                differents += 1;
                seuil += d;
                if x < minx { minx = x; }
                if y < miny { miny = y; }
                if x > maxx { maxx = x; }
                if y > maxy { maxy = y; }
            }
        }
    }

    if differents == 0 {
        println!("DIFF: AUCUNE différence dans la zone ({},{})-({},{}) — immobile", zx0, zy0, zx1, zy1);
    } else {
        let pct = differents as f64 * 100.0 / ((zx1 - zx0) as f64 * (zy1 - zy0) as f64);
        println!(
            "DIFF: {} px différents ({:.3}% de la zone) bbox=({},{})-({},{}) distance cumulée={}",
            differents, pct, minx, miny, maxx, maxy, seuil
        );
    }
    Ok(())
}

/// Grille ASCII d'une zone (4 px/cellule, # = jaune/blanc vif) + total de
/// pixels jaunes. Équivalent de lire_compteur.py.
pub fn grille_ascii(chemin: &str, x0: u32, y0: u32, x1: u32, y1: u32) -> Result<(), String> {
    let img = charger(chemin)?;
    let (w, h) = img.dimensions();
    let (x0, y0) = (x0.min(w), y0.min(h));
    let (x1, y1) = (x1.min(w), y1.min(h));
    let j = Critere::jaune();
    let b = Critere::blanc_vif();

    println!("Grille ASCII zone ({},{})-({},{}) (4px/cellule, # = jaune/blanc vif):", x0, y0, x1, y1);
    let mut y = y0;
    while y < y1 {
        let mut ligne = String::new();
        let mut x = x0;
        while x < x1 {
            let px = img.get_pixel(x, y);
            if j.teste(px[0], px[1], px[2]) || b.teste(px[0], px[1], px[2]) {
                ligne.push('#');
            } else {
                ligne.push('.');
            }
            x += 4;
        }
        println!("{:4} {}", y, ligne);
        y += 4;
    }

    let mut total_jaune: u64 = 0;
    for yy in y0..y1 {
        for xx in x0..x1 {
            let px = img.get_pixel(xx, yy);
            if j.teste(px[0], px[1], px[2]) {
                total_jaune += 1;
            }
        }
    }
    println!("\nPixels jaunes dans la zone: {}", total_jaune);
    Ok(())
}

/// Crop une image vers un fichier (utile avant envoi VLM : réduire à ≤800px).
pub fn crop(chemin_in: &str, x0: u32, y0: u32, x1: u32, y1: u32, chemin_out: &str) -> Result<(), String> {
    let img = charger(chemin_in)?;
    let (w, h) = img.dimensions();
    let (x0, y0) = (x0.min(w), y0.min(h));
    let (x1, y1) = (x1.min(w), y1.min(h));
    if x1 <= x0 || y1 <= y0 {
        return Err(format!("zone crop invalide: ({},{})-({},{})", x0, y0, x1, y1));
    }
    let zone = img.crop_imm(x0, y0, x1 - x0, y1 - y0);
    zone.save(chemin_out).map_err(|e| format!("crop save {} : {}", chemin_out, e))?;
    println!("CROP: {} → {} ({}x{})", chemin_in, chemin_out, x1 - x0, y1 - y0);
    Ok(())
}

/// Crop + zoom (×facteur, LANCZOS) → PNG bytes en mémoire pour envoi VLM.
/// Équivalent de voir_zone.py : recadrer une zone et l'agrandir 3x pour
/// une lecture fine par le VLM, sans écrire de fichier intermédiaire.
pub fn crop_zoom_png(
    img: &image::DynamicImage,
    x0: u32, y0: u32, x1: u32, y1: u32,
    facteur: u32,
) -> Result<Vec<u8>, String> {
    let (w, h) = img.dimensions();
    let (x0, y0) = (x0.min(w), y0.min(h));
    let (x1, y1) = (x1.min(w), y1.min(h));
    if x1 <= x0 || y1 <= y0 {
        return Err(format!("zone crop invalide: ({},{})-({},{})", x0, y0, x1, y1));
    }
    let zone = img.crop_imm(x0, y0, x1 - x0, y1 - y0).to_rgb8();
    let (cw, ch) = (zone.width(), zone.height());
    let (nw, nh) = (cw * facteur, ch * facteur);
    let zoom = image::imageops::resize(&zone, nw, nh, image::imageops::FilterType::Triangle);
    let mut buf = Vec::new();
    image::codecs::png::PngEncoder::new(&mut std::io::Cursor::new(&mut buf))
        .write_image(
            zoom.as_raw(),
            nw,
            nh,
            image::ExtendedColorType::Rgb8,
        )
        .map_err(|e| format!("encodage PNG zoom : {}", e))?;
    Ok(buf)
}

/// Crop + réduction à ≤1024 px de côté → PNG bytes en mémoire.
/// Équivalent du crop de calib_grille.py (fenêtre cible → VLM non perturbé).
pub fn crop_reduit_png(
    chemin: &str,
    x0: u32, y0: u32, w: u32, h: u32,
) -> Result<Vec<u8>, String> {
    let img = charger(chemin)?;
    let (iw, ih) = img.dimensions();
    let (x0, y0) = (x0.min(iw), y0.min(ih));
    let (w, h) = (w.min(iw - x0), h.min(ih - y0));
    if w == 0 || h == 0 {
        return Err("crop vide".to_string());
    }
    let zone = img.crop_imm(x0, y0, w, h).to_rgb8();
    let max_side = w.max(h);
    let (nw, nh) = if max_side > 1024 {
        let scale = 1024.0 / max_side as f64;
        (
            (w as f64 * scale).round().max(1.0) as u32,
            (h as f64 * scale).round().max(1.0) as u32,
        )
    } else {
        (w, h)
    };
    let small = if (nw, nh) != (w, h) {
        image::imageops::resize(&zone, nw, nh, image::imageops::FilterType::Triangle)
    } else {
        zone
    };
    let mut buf = Vec::new();
    image::codecs::png::PngEncoder::new(&mut std::io::Cursor::new(&mut buf))
        .write_image(small.as_raw(), nw, nh, image::ExtendedColorType::Rgb8)
        .map_err(|e| format!("encodage PNG crop réduit : {}", e))?;
    Ok(buf)
}

/// Réduit une image (fichier) à ≤max_side px de côté → PNG bytes en mémoire.
/// Le VLM 1.6B est plus fiable et PLUS RAPIDE sur des images petites
/// (moins de tokens de préfill) : 1024 px ≈ 1.6s, 512 px ≈ 0.6s.
pub fn reduire_max(chemin: &str, max_side: u32) -> Result<Vec<u8>, String> {
    let img = charger(chemin)?;
    let (w, h) = img.dimensions();
    let max_side = max_side.max(64);
    if w.max(h) <= max_side {
        let mut buf = Vec::new();
        let rgb = img.to_rgb8();
        image::codecs::png::PngEncoder::new(&mut std::io::Cursor::new(&mut buf))
            .write_image(rgb.as_raw(), w, h, image::ExtendedColorType::Rgb8)
            .map_err(|e| format!("encodage PNG : {}", e))?;
        return Ok(buf);
    }
    let scale = max_side as f64 / w.max(h) as f64;
    let (nw, nh) = (
        (w as f64 * scale).round().max(1.0) as u32,
        (h as f64 * scale).round().max(1.0) as u32,
    );
    let rgb = img.to_rgb8();
    let small = image::imageops::resize(&rgb, nw, nh, image::imageops::FilterType::Triangle);
    let mut buf = Vec::new();
    image::codecs::png::PngEncoder::new(&mut std::io::Cursor::new(&mut buf))
        .write_image(small.as_raw(), nw, nh, image::ExtendedColorType::Rgb8)
        .map_err(|e| format!("encodage PNG réduit : {}", e))?;
    Ok(buf)
}

/// Réduit une image (fichier) à ≤1024 px de côté → PNG bytes en mémoire.
/// Le VLM 1.6B est plus fiable sur des images ≤1024 px (calib_grille.py
/// appliquait le même facteur avant envoi).
pub fn reduire_1024(chemin: &str) -> Result<Vec<u8>, String> {
    reduire_max(chemin, 1024)
}

/// BARYCENTRE du bouton : détecte les blocs jaunes, regroupe ceux qui sont
/// proches (≤ cell×2), et calcule le centre PONDÉRÉ par les pixels.
/// Le bloc unique (--analyse) rate le centre quand le texte « CLIQUE » au
/// milieu divise le massif — le barycentre des blocs voisins corrige ça.
/// Usage : ecran-live --centre <image> [--cell 10] [--min 3]
pub fn barycentre(chemin: &str, cell: u32, min_dense: u32) -> Result<(), String> {
    let img = charger(chemin)?;
    let crit = Critere::jaune();
    let blocs = cluster_couleur(&img, &crit, cell, min_dense);
    if blocs.is_empty() {
        println!("CENTRE: aucun bloc jaune");
        return Ok(());
    }
    // Regrouper les blocs proches (≤ 3 cellules) en un seul amas
    let mut groupes: Vec<(i64, i64, u64)> = Vec::new(); // (cx, cy, pixels cumulés)
    for b in blocs.iter() {
        let cx = b.cx as i64;
        let cy = b.cy as i64;
        let mut place = false;
        for g in groupes.iter_mut() {
            let dx = (g.0 - cx).abs();
            let dy = (g.1 - cy).abs();
            if dx <= 3 && dy <= 3 {
                // Moyenne pondérée par pixels
                let total = g.2 + b.pixels as u64;
                g.0 = (g.0 * g.2 as i64 + cx * b.pixels as i64) / total as i64;
                g.1 = (g.1 * g.2 as i64 + cy * b.pixels as i64) / total as i64;
                g.2 = total;
                place = true;
                break;
            }
        }
        if !place {
            groupes.push((cx, cy, b.pixels as u64));
        }
    }
    // Le plus gros amas = le bouton
    groupes.sort_by(|a, b| b.2.cmp(&a.2));
    if let Some((gcx, gcy, gpix)) = groupes.first() {
        let sx = (gcx * cell as i64 + cell as i64 / 2) as f64;
        let sy = (gcy * cell as i64 + cell as i64 / 2) as f64;
        println!(
            "CENTRE: bouton à écran ({:.0},{:.0}) — {} blocs groupés, {} px",
            sx, sy,
            groupes.len(),
            gpix
        );
        // Afficher aussi les amas suivants (autres boutons/éléments)
        for (i, (gcx, gcy, gpix)) in groupes.iter().enumerate().skip(1).take(5) {
            let sx = (gcx * cell as i64 + cell as i64 / 2) as f64;
            let sy = (gcy * cell as i64 + cell as i64 / 2) as f64;
            println!("  amas {}: ({:.0},{:.0}) {} px", i, sx, sy, gpix);
        }
    }
    Ok(())
}
