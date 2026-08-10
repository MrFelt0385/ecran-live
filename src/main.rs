// ecran-live — capture l'écran du Mac mini avec ScreenCaptureKit (Apple),
// la méthode la plus rapide et la plus légère sur macOS. Capture directement
// à la taille voulue (zéro resize, zéro copie inutile), écrit un JPEG.
// Usage :
//   ecran-live [largeur_max]        → capture unique (défaut 1600px)
//   ecran-live --watch N [largeur]  → mode flux : capture toutes les N secondes
//   ecran-live --track [largeur]    → mode suivi : capture à ~5 fps quand la
//                                     souris bouge, + écrit la position souris
// Sortie : ~/ecran-live.jpg + ~/souris.json (mode --track)

use std::io::Write;
use std::time::{Duration, Instant};

use core_graphics::event::{CGEventType, CGMouseButton};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

use screencapturekit::prelude::*;
use screencapturekit::screenshot_manager::{CGImageExt, SCScreenshotManager};
use image::ImageEncoder;
use image::GenericImageView;

// Garde en cache le contenu partageable + le filtre : SCShareableContent::get()
// à chaque frame faisait crasher la boucle (segfault).
struct Capteur {
    filter: SCContentFilter,
    ratio: f64,
    display_w: u32,
}

impl Capteur {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let content = SCShareableContent::get()?;
        let display = &content.displays()[0];
        let frame = display.frame();
        let ratio = frame.size.height / frame.size.width;
        let display_w = frame.size.width as u32;
        let filter = SCContentFilter::create()
            .with_display(display)
            .with_excluding_windows(&[])
            .build();
        Ok(Self {
            filter,
            ratio,
            display_w,
        })
    }

    /// Facteur d'échelle capture → écran réel (leçon #8 Command Code) :
    /// les coordonnées OCR sont dans l'espace de la capture (ex: 1600px),
    /// il faut les multiplier par ce facteur pour cliquer au bon endroit.
    fn scale_to_display(&self, capture_w: u32) -> f64 {
        self.display_w as f64 / capture_w as f64
    }

    fn shoot(&self, width: u32, out_path: &str, png: bool) -> Result<u32, Box<dyn std::error::Error>> {
        let h = (width as f64 * self.ratio).round() as u32;

        let config = SCStreamConfiguration::new()
            .with_width(width)
            .with_height(h)
            .with_pixel_format(PixelFormat::BGRA);

        let img = SCScreenshotManager::capture_image(&self.filter, &config)?;
        let bgra = img.bgra_data()?;

        // BGRA → RGB (swap R/B) pour l'encodeur
        let mut rgb = Vec::with_capacity(bgra.len() / 4 * 3);
        for px in bgra.chunks_exact(4) {
            rgb.push(px[2]);
            rgb.push(px[1]);
            rgb.push(px[0]);
        }

        let mut out = std::fs::File::create(out_path)?;
        if png {
            // PNG sans perte : préserve les petits textes, évite les artefacts
            // qui font halluciner les modèles de vision.
            image::codecs::png::PngEncoder::new(&mut out).write_image(
                &rgb,
                width,
                h,
                image::ExtendedColorType::Rgb8,
            )?;
        } else {
            let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 80);
            encoder.encode(&rgb, width, h, image::ExtendedColorType::Rgb8)?;
        }
        out.flush()?;

        let size = std::fs::metadata(out_path)?.len();
        println!("Capture OK: {} ({}x{}, {} octets)", out_path, width, h, size);
        Ok(h)
    }

    /// Capture l'écran et retourne les bytes PNG en mémoire (aucun fichier écrit).
    fn capture_bytes(&self, width: u32, h: u32) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let config = SCStreamConfiguration::new()
            .with_width(width)
            .with_height(h)
            .with_pixel_format(PixelFormat::BGRA);

        let img = SCScreenshotManager::capture_image(&self.filter, &config)?;
        let bgra = img.bgra_data()?;

        let mut rgb = Vec::with_capacity(bgra.len() / 4 * 3);
        for px in bgra.chunks_exact(4) {
            rgb.push(px[2]);
            rgb.push(px[1]);
            rgb.push(px[0]);
        }

        let mut buf = Vec::new();
        image::codecs::png::PngEncoder::new(&mut std::io::Cursor::new(&mut buf)).write_image(
            &rgb,
            width,
            h,
            image::ExtendedColorType::Rgb8,
        )?;
        Ok(buf)
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let home = std::env::var("HOME")?;

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = args.first().map(|s| s.as_str()).unwrap_or("");
    let width: u32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(1600);

    // Format par défaut : PNG sans perte (précision maximale pour la vision).
    // `--jpg` force le JPEG compressé (plus léger, artefacts possibles).
    let png = !args.iter().any(|a| a == "--jpg");
    let ext = if png { "png" } else { "jpg" };
    let out_path = format!("{}/ecran-live.{}", home, ext);

    match mode {
        "--watch" => {
            let secs: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(3);
            let width: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1600);
            let cap = Capteur::new()?;
            println!("Mode flux : capture toutes les {}s (Ctrl+C pour arrêter)", secs);
            loop {
                let _ = cap.shoot(width, &out_path, png);
                std::thread::sleep(Duration::from_secs(secs));
            }
        }
        "--track" => {
            let cap = Capteur::new()?;
            println!("Mode suivi : capture quand la souris bouge (Ctrl+C pour arrêter)");
            let mut last = (0.0_f64, 0.0_f64);
            let mut last_shot = Instant::now() - Duration::from_secs(1);
            loop {
                let pos = mouse_pos();
                let moved = (pos.0 - last.0).abs() + (pos.1 - last.1).abs() > 1.0;
                let due = last_shot.elapsed() >= Duration::from_millis(200);
                if moved || due {
                    let _ = cap.shoot(width, &out_path, png);
                    write_mouse(&home, pos);
                    last = pos;
                    last_shot = Instant::now();
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        "--zoom" => {
            let cap = Capteur::new()?;
            // Grille de zoom : 2x2 par défaut, configurable (--grid 3 2)
            let gx: u32 = args.iter().position(|a| a == "--grid")
                .and_then(|i| args.get(i + 1)).and_then(|s| s.parse().ok()).unwrap_or(2);
            let gy: u32 = args.iter().position(|a| a == "--grid")
                .and_then(|i| args.get(i + 2)).and_then(|s| s.parse().ok()).unwrap_or(2);
            println!("Mode zoom : vision globale puis grille {}x{} (analyse fine)", gx, gy);

            // 1. Capture PNG en mémoire
            let h = (width as f64 * cap.ratio).round() as u32;
            let global_png = cap.capture_bytes(width, h)?;
            println!("Capture {}x{} en mémoire ({:.1} MB)", width, h, global_png.len() as f64 / 1048576.0);

            // 2. Vision GLOBALE (image réduite → décodage rapide côté serveur)
            println!("\n=== VISION GLOBALE (image réduite) ===");
            let small_png = downscale_png(&global_png, 384)?;
            let global_q = global_prompt(width, h, 640);
            let global = analyze_image(&small_png, &global_q)?;
            println!("{}", global);

            // 3. Zoom : découpage en grille + analyse fine de chaque zone
            let mut img = image::load_from_memory(&global_png)?;
            let (iw, ih) = img.dimensions();
            let zw = iw / gx;
            let zh = ih / gy;
            let mut zone_idx = 0;
            for row in 0..gy {
                for col in 0..gx {
                    zone_idx += 1;
                    let x = col * zw;
                    let y = row * zh;
                    let w = if col == gx - 1 { iw - x } else { zw };
                    let hh = if row == gy - 1 { ih - y } else { zh };
                    let crop = img.crop(x, y, w, hh);
                    let mut buf = Vec::new();
                    image::codecs::png::PngEncoder::new(&mut std::io::Cursor::new(&mut buf))
                        .write_image(
                            crop.as_bytes(),
                            crop.width(),
                            crop.height(),
                            image::ExtendedColorType::Rgb8,
                        )?;
                    println!("\n=== ZONE {}/{} (x:{}, y:{}, {}x{}) ===", zone_idx, gx * gy, x, y, w, hh);
                    let q = format!(
                        "Tu es dans une zone zoomée d'un écran (coin x:{}, y:{}). \
                         Lis TOUS les textes visibles mot à mot, décris les boutons, icônes et éléments d'interface précisément.",
                        x, y
                    );
                    match analyze_image(&buf, &q) {
                        Ok(ans) => println!("{}", ans),
                        Err(e) => println!("ERREUR zone {}: {}", zone_idx, e),
                    }
                }
            }
        }
        "--salient" => {
            let cap = Capteur::new()?;
            let top: usize = args
                .iter()
                .position(|a| a == "--top")
                .and_then(|i| args.get(i + 1))
                .and_then(|s| s.parse().ok())
                .unwrap_or(4);
            println!("Mode saillance : détection des zones d'intérêt puis zoom fin (top {})", top);

            // 1. Capture PNG en mémoire
            let h = (width as f64 * cap.ratio).round() as u32;
            let global_png = cap.capture_bytes(width, h)?;
            println!(
                "Capture {}x{} en mémoire ({:.1} MB)",
                width,
                h,
                global_png.len() as f64 / 1048576.0
            );

            // 2. Vision GLOBALE (image réduite → décodage rapide côté serveur)
            println!("\n=== VISION GLOBALE (image réduite) ===");
            let small_png = downscale_png(&global_png, 384)?;
            let global_q = global_prompt(width, h, 640);
            let global = analyze_image(&small_png, &global_q)?;
            println!("{}", global);

            // 3. CARTE DE SAILLANCE : repérer les zones qui ressortent
            //    (grille fine 8x5 = 40 cellules, on garde les `top` plus contrastées)
            println!("\n=== CARTE DE SAILLANCE ===");
            let zones = saliency(&global_png, 8, 5, top)?;
            for (i, z) in zones.iter().enumerate() {
                println!(
                    "Zone {}/{} : x={}, y={}, {}x{}, contraste={:.0}",
                    i + 1,
                    zones.len(),
                    z.x,
                    z.y,
                    z.w,
                    z.h,
                    z.score
                );
            }

            // 4. ZOOM FIN sur chaque zone saillante
            let mut img = image::load_from_memory(&global_png)?;
            for (i, z) in zones.iter().enumerate() {
                // Marge de 10% autour de la zone pour ne pas couper les bords
                let mw = (z.w as f64 * 0.1) as u32;
                let mh = (z.h as f64 * 0.1) as u32;
                let x = z.x.saturating_sub(mw);
                let y = z.y.saturating_sub(mh);
                let w = (z.w + 2 * mw).min(img.width() - x);
                let hh = (z.h + 2 * mh).min(img.height() - y);
                let crop = img.crop(x, y, w, hh);
                let mut buf = Vec::new();
                image::codecs::png::PngEncoder::new(&mut std::io::Cursor::new(&mut buf)).write_image(
                    crop.as_bytes(),
                    crop.width(),
                    crop.height(),
                    image::ExtendedColorType::Rgb8,
                )?;
                println!(
                    "\n=== ZONE SAILLANTE {}/{} (x:{}, y:{}, {}x{}) ===",
                    i + 1,
                    zones.len(),
                    x,
                    y,
                    w,
                    hh
                );
                let q = format!(
                    "Tu es dans une zone zoomée d'un écran (coin x:{}, y:{}). \
                     Lis TOUS les textes visibles mot à mot, décris les boutons, icônes et éléments d'interface précisément.",
                    x, y
                );
                match analyze_image(&buf, &q) {
                    Ok(ans) => println!("{}", ans),
                    Err(e) => println!("ERREUR zone {}: {}", i + 1, e),
                }
            }
        }
        "--salient-color" => {
            let cap = Capteur::new()?;
            let top: usize = args
                .iter()
                .position(|a| a == "--top")
                .and_then(|i| args.get(i + 1))
                .and_then(|s| s.parse().ok())
                .unwrap_or(4);
            println!("Mode saillance COULEUR : zones saturées/chaudes (top {})", top);

            let h = (width as f64 * cap.ratio).round() as u32;
            let global_png = cap.capture_bytes(width, h)?;
            println!("Capture {}x{} en mémoire", width, h);

            let zones = color_saliency(&global_png, 8, 5, top)?;
            for (i, z) in zones.iter().enumerate() {
                println!(
                    "Zone couleur {}/{} : x={}, y={}, {}x{}, score={:.0}",
                    i + 1,
                    zones.len(),
                    z.x,
                    z.y,
                    z.w,
                    z.h,
                    z.score
                );
            }
            zoom_zones(&global_png, &zones)?;
        }
        "--salient-motion" => {
            let cap = Capteur::new()?;
            let top: usize = args
                .iter()
                .position(|a| a == "--top")
                .and_then(|i| args.get(i + 1))
                .and_then(|s| s.parse().ok())
                .unwrap_or(4);
            println!("Mode saillance MOUVEMENT : zones qui changent (top {})", top);

            let h = (width as f64 * cap.ratio).round() as u32;
            // Deux captures espacées de 300ms pour détecter le changement
            // (suffisant pour le mouvement, et plus rapide que 1s)
            let prev = cap.capture_bytes(width, h)?;
            std::thread::sleep(Duration::from_millis(300));
            let curr = cap.capture_bytes(width, h)?;
            println!("Deux captures espacées de 300ms");

            let zones = motion_saliency(&prev, &curr, 8, 5, top)?;
            for (i, z) in zones.iter().enumerate() {
                println!(
                    "Zone mouvement {}/{} : x={}, y={}, {}x{}, diff={:.1}",
                    i + 1,
                    zones.len(),
                    z.x,
                    z.y,
                    z.w,
                    z.h,
                    z.score
                );
            }
            zoom_zones(&curr, &zones)?;
        }
        "--attention" => {
            let cap = Capteur::new()?;
            let top: usize = args
                .iter()
                .position(|a| a == "--top")
                .and_then(|i| args.get(i + 1))
                .and_then(|s| s.parse().ok())
                .unwrap_or(4);
            println!(
                "Mode ATTENTION combinée : contraste + couleur + mouvement (top {})",
                top
            );

            let h = (width as f64 * cap.ratio).round() as u32;
            // Deux captures espacées de 1s (nécessaire pour le canal mouvement)
            let prev = cap.capture_bytes(width, h)?;
            std::thread::sleep(Duration::from_millis(300));
            let curr = cap.capture_bytes(width, h)?;
            println!("Deux captures espacées de 300ms");

            // Vision globale sur la capture la plus récente (image réduite)
            println!("\n=== VISION GLOBALE (image réduite) ===");
            let small_png = downscale_png(&curr, 384)?;
            let mut global_q = global_prompt(width, h, 640);
            // Coordinate Priming (GUI-Lens) : injecter les textes OCR + coords
            let priming = ocr_texts(&curr);
            if !priming.is_empty() {
                global_q.push_str("\n\nTextes détectés par OCR (coordonnées en pixels de l'écran):\n");
                global_q.push_str(&priming);
                println!("[Coordinate Priming: {} textes OCR injectés]", priming.lines().count());
            }
            let global = analyze_image(&small_png, &global_q)?;
            println!("{}", global);

            // Carte d'attention combinée
            println!("\n=== CARTE D'ATTENTION (contraste+couleur+mouvement) ===");
            let zones = attention(&prev, &curr, 8, 5, top)?;
            for (i, z) in zones.iter().enumerate() {
                println!(
                    "Zone attention {}/{} : x={}, y={}, {}x{}, score={:.2}",
                    i + 1,
                    zones.len(),
                    z.x,
                    z.y,
                    z.w,
                    z.h,
                    z.score
                );
            }
            zoom_zones(&curr, &zones)?;
        }
        "--deep" => {
            let cap = Capteur::new()?;
            let depth: u32 = args
                .iter()
                .position(|a| a == "--depth")
                .and_then(|i| args.get(i + 1))
                .and_then(|s| s.parse().ok())
                .unwrap_or(3);
            println!("Mode zoom ITÉRATIF (Iterative Narrowing) : profondeur {}", depth);

            let h = (width as f64 * cap.ratio).round() as u32;
            let global_png = cap.capture_bytes(width, h)?;
            println!("Capture {}x{} en mémoire", width, h);

            // Vision globale (image réduite → décodage rapide côté serveur)
            println!("\n=== VISION GLOBALE (image réduite) ===");
            let small_png = downscale_png(&global_png, 384)?;
            let global_q = global_prompt(width, h, 640);
            let global = analyze_image(&small_png, &global_q)?;
            println!("{}", global);

            // Zoom itératif : re-zoom tant que des détails fins restent
            println!("\n=== ZOOM ITÉRATIF ===");
            zoom_deep(&global_png, depth, 40)?;
        }
        "--uizoomer" => {
            let cap = Capteur::new()?;
            println!("Mode UI-Zoomer : zoom conditionnel sur l'incertitude");

            let h = (width as f64 * cap.ratio).round() as u32;
            let global_png = cap.capture_bytes(width, h)?;
            println!("Capture {}x{} en mémoire", width, h);

            uizoomer(&global_png)?;
        }
        "--ocr" => {
            let cap = Capteur::new()?;
            println!("Mode OCR : extraction des textes + bounding boxes (coordinate priming)");
            let h = (width as f64 * cap.ratio).round() as u32;
            let global_png = cap.capture_bytes(width, h)?;
            // ocrs prend un chemin de fichier → écrire un temp
            let tmp = std::env::temp_dir().join("ecran-live-ocr.png");
            std::fs::write(&tmp, &global_png)?;
            let out = std::process::Command::new("ocrs")
                .arg("--json")
                .arg(&tmp)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .spawn();
            match out {
                Ok(mut child) => {
                    use std::io::Write;
                    if let Some(mut stdin) = child.stdin.take() {
                        let _ = stdin.write_all(&global_png);
                    }
                    let output = child.wait_with_output();
                    match output {
                        Ok(o) if o.status.success() => {
                            let text = String::from_utf8_lossy(&o.stdout).to_string();
                            println!("{}", text);
                        }
                        Ok(o) => {
                            eprintln!("ERREUR ocrs (status {:?})", o.status);
                        }
                        Err(e) => eprintln!("ERREUR ocrs: {}", e),
                    }
                }
                Err(e) => eprintln!("ERREUR lancement ocrs: {} (installer avec: cargo install ocrs-cli --locked)", e),
            }
        }
        "--locate" => {
            // Grounding : trouve un texte à l'écran, retourne ses coordonnées
            // réelles (pixels écran) via OCR + matching flou, puis vision en
            // secours si l'OCR ne trouve rien.
            let target = args.get(2).cloned().unwrap_or_else(|| "PINNED".to_string());
            let cap = Capteur::new()?;
            println!(
                "Mode LOCATE : recherche « {} » à l'écran (grounding OCR + vision)",
                target
            );
            let ocr_w = width.max(1600);
            let h = (ocr_w as f64 * cap.ratio).round() as u32;
            let global_png = cap.capture_bytes(ocr_w, h)?;
            println!(
                "Capture {}x{} pour OCR ({:.1} MB)",
                ocr_w,
                h,
                global_png.len() as f64 / 1048576.0
            );

            // 1. OCR + matching flou (fonction commune)
            if let Some((text, x, y)) = find_text_ocr(&global_png, &target) {
                let scale = cap.scale_to_display(ocr_w);
                println!(
                    "🎯 TROUVÉ « {} » en OCR pur à ({:.0}, {:.0})",
                    text, x, y
                );
                println!("↔️ Remap ×{:.2} → écran réel ({:.0}, {:.0})", scale, x * scale, y * scale);
                println!("LARGEUR_ECRAN {}", cap.display_w);
                return Ok(());
            }
            println!("[OCR n'a pas trouvé « {} » — tentative vision]", target);

            // 2. Fallback vision : image réduite + prompt ciblé avec priming
            let small_png = downscale_png(&global_png, 384)?;
            let priming = ocr_texts(&global_png);
            let mut q = format!(
                "Un texte « {} » est visible quelque part à l'écran. \
                 Donne ses coordonnées approximatives (x, y) en pixels de l'ÉCRAN RÉEL \
                 (largeur {}px). Réponds au format: COORD: x,y",
                target, cap.display_w
            );
            if !priming.is_empty() {
                q.push_str("\n\nTextes OCR détectés avec positions:\n");
                q.push_str(&priming);
            }
            let ans = analyze_image(&small_png, &q)?;
            println!("{}", ans);
            if let Some(idx) = ans.find("COORD:") {
                let coord = ans[idx + 6..].trim().to_string();
                println!("🎯 COORDONNÉES: {}", coord);
            }
        }
        "--click" | "--rightclick" | "--doubleclick" => {
            // Grounding + action : localise un texte, remappe les coordonnées
            // OCR → écran réel (facteur d'échelle), puis agit.
            let right = mode == "--rightclick";
            let dbl = mode == "--doubleclick";
            let target = args.get(2).cloned().unwrap_or_else(|| "PINNED".to_string());
            let cap = Capteur::new()?;
            println!(
                "Mode {} : recherche « {} » puis {} (grounding + remap + CGEvent)",
                mode,
                target,
                if right {
                    "clic droit"
                } else if dbl {
                    "double-clic"
                } else {
                    "clic"
                }
            );
            let ocr_w = width.max(1600);
            let h = (ocr_w as f64 * cap.ratio).round() as u32;
            let global_png = cap.capture_bytes(ocr_w, h)?;
            println!(
                "Capture {}x{} (écran réel: {}px de large)",
                ocr_w, h, cap.display_w
            );

            if let Some((text, cx, cy)) = find_text_ocr(&global_png, &target) {
                let scale = cap.scale_to_display(ocr_w);
                let real_x = cx * scale;
                let real_y = cy * scale;
                println!(
                    "🎯 TROUVÉ « {} » en capture ({:.0}, {:.0})",
                    text, cx, cy
                );
                println!(
                    "↔️ Remap ×{:.2} → écran réel ({:.0}, {:.0})",
                    scale, real_x, real_y
                );
                let real_y_center = real_y + 10.0 * scale;
                if right {
                    right_click_at(real_x, real_y_center)?;
                    println!("✅ Clic droit effectué sur « {} »", target);
                } else if dbl {
                    double_click_at(real_x, real_y_center)?;
                    println!("✅ Double-clic effectué sur « {} »", target);
                } else {
                    click_at(real_x, real_y_center)?;
                    println!("✅ Clic effectué sur « {} »", target);
                }
            } else {
                println!("❌ « {} » introuvable à l'écran", target);
            }
        }
        "--scroll" => {
            // Scroll : localise un texte, déplace la souris dessus puis scrolle.
            let target = args.get(2).cloned().unwrap_or_else(|| "PINNED".to_string());
            let lines: i32 = args
                .get(3)
                .and_then(|s| s.parse().ok())
                .unwrap_or(-5);
            let cap = Capteur::new()?;
            println!(
                "Mode SCROLL : recherche « {} » puis scroll {} lignes",
                target, lines
            );
            let ocr_w = width.max(1600);
            let h = (ocr_w as f64 * cap.ratio).round() as u32;
            let global_png = cap.capture_bytes(ocr_w, h)?;

            if let Some((text, cx, cy)) = find_text_ocr(&global_png, &target) {
                let scale = cap.scale_to_display(ocr_w);
                let real_x = cx * scale;
                let real_y = cy * scale;
                println!(
                    "🎯 TROUVÉ « {} » → remap ×{:.2} → ({:.0}, {:.0})",
                    text, scale, real_x, real_y
                );
                scroll_at(real_x, real_y + 10.0 * scale, lines)?;
            } else {
                println!("❌ « {} » introuvable à l'écran", target);
            }
        }
        _ => {
            let cap = Capteur::new()?;
            if let Err(e) = cap.shoot(width, &out_path, png) {
                eprintln!("ERREUR: {}", e);
                std::process::exit(1);
            }
        }
    }
    Ok(())
}

/// Construit le prompt de vision globale en divulguant le facteur d'échelle
/// (leçon #8 de Command Code) : l'image envoyée est réduite, le modèle doit
/// savoir remapper les coordonnées vers la résolution réelle de l'écran.
fn global_prompt(full_w: u32, full_h: u32, small_w: u32) -> String {
    let scale = full_w as f64 / small_w as f64;
    format!(
        "Décris précisément ce qu'on voit à l'écran : quelles fenêtres/applications, \
         quels éléments d'interface, où se trouvent les zones importantes (menus, textes, boutons). \
         NOTE: cette image est réduite de {}x{} à {}px de large (facteur {:.2}) — \
         si tu indiques des coordonnées, multiplie-les par {:.2} pour obtenir les pixels réels de l'écran.",
        full_w, full_h, small_w, scale, scale
    )
}

/// Coordinate Priming (GUI-Lens) : lance ocrs sur le PNG et retourne les
/// textes détectés avec leurs bounding boxes, formatés pour le prompt.
/// Retourne "" si ocrs est absent ou échoue (l'analyse continue sans priming).
fn ocr_texts(png_bytes: &[u8]) -> String {
    let tmp = std::env::temp_dir().join("ecran-live-ocr.png");
    if std::fs::write(&tmp, png_bytes).is_err() {
        return String::new();
    }
    let out = std::process::Command::new("ocrs")
        .arg("--json")
        .arg(&tmp)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output();
    let Ok(o) = out else { return String::new() };
    if !o.status.success() {
        return String::new();
    }
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(&o.stdout) else {
        return String::new();
    };
    let mut lines: Vec<String> = Vec::new();
    if let Some(paras) = v["paragraphs"].as_array() {
        for p in paras.iter().take(30) {
            if let Some(ls) = p["lines"].as_array() {
                for l in ls.iter().take(20) {
                    let text = l["text"].as_str().unwrap_or("").trim().to_string();
                    if text.is_empty() {
                        continue;
                    }
                    // Centre du bounding box (vertices[0]=haut-gauche, [1]=haut-droit)
                    let cx = l["vertices"][1][0].as_f64().unwrap_or(0.0);
                    let cy = l["vertices"][0][1].as_f64().unwrap_or(0.0);
                    lines.push(format!("«{}» @ ({:.0},{:.0})", text, cx, cy));
                }
            }
        }
    }
    lines.join("\n")
}

/// Grounding par OCR : lance ocrs sur le PNG et cherche le premier texte qui
/// matche (fuzzy, distance ≤ 2) la cible. Retourne (texte trouvé, x, y) en
/// coordonnées de la capture, ou None si introuvable.
fn find_text_ocr(png_bytes: &[u8], target: &str) -> Option<(String, f64, f64)> {
    let tmp = std::env::temp_dir().join("ecran-live-ocr.png");
    std::fs::write(&tmp, png_bytes).ok()?;
    let out = std::process::Command::new("ocrs")
        .arg("--json")
        .arg(&tmp)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    let paras = v["paragraphs"].as_array()?;
    for p in paras {
        let ls = p["lines"].as_array()?;
        for l in ls {
            let text = l["text"].as_str().unwrap_or("").to_string();
            if fuzzy_match(&text, target) {
                let x = l["vertices"][0][0].as_f64().unwrap_or(0.0);
                let y = l["vertices"][0][1].as_f64().unwrap_or(0.0);
                return Some((text, x, y));
            }
        }
    }
    None
}

/// Distance de Levenshtein (leçon #5 Command Code) : l'OCR fait des fautes
/// (« Messacine » vs « Messagerie »), un matching exact échoue. On accepte
/// les textes à distance ≤ 2 du texte cherché.
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            let min = (cur[j] + 1).min(prev[j + 1] + 1).min(prev[j] + cost);
            cur.push(min);
        }
        prev = cur;
    }
    prev[b.len()]
}

/// Matching flou : vrai si `text` contient un mot à distance ≤ 2 de `target`.
/// (la cible est normalisée en minuscules)
fn fuzzy_match(text: &str, target: &str) -> bool {
    let t = target.to_lowercase();
    let tl = t.len();
    if t.is_empty() || tl < 3 {
        return text.to_lowercase().contains(&t);
    }
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= tl.saturating_sub(2) && w.len() <= tl + 2)
        .any(|w| levenshtein(w, &t) <= 2)
}

/// Analyse une image PNG (bytes) via le serveur mlxcel local (port 8085,
/// Rust + MLX C++ natif — ~4-10x plus rapide que mistralrs).
/// Retourne le texte généré.
fn analyze_image(png_bytes: &[u8], question: &str) -> Result<String, String> {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(png_bytes);
    let payload = serde_json::json!({
        "model": "LFM2.5-VL-1.6B-4bit",
        "messages": [{
            "role": "user",
            "content": [
                {"type": "text", "text": question},
                {"type": "image_url", "image_url": {"url": format!("data:image/png;base64,{}", b64)}}
            ]
        }],
        "max_tokens": 100,
        "temperature": 0
    });
    let resp = ureq::post("http://localhost:8085/v1/chat/completions")
        .timeout(Duration::from_secs(180))
        .send_json(payload)
        .map_err(|e| e.to_string())?;
    let json: serde_json::Value = resp
        .into_json()
        .map_err(|e| e.to_string())?;
    let content = json["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("(réponse vide)")
        .to_string();
    Ok(content)
}

/// Réduit une image PNG en mémoire pour la passe GLOBALE (rapide) tout en
/// gardant l'original pour les zooms fins. Le goulot du serveur mistral.rs
/// est le décodage du PNG complet (1.5 MB) — une version ~640px se décodé
/// ~2x plus vite sans perte pour repérer les zones d'intérêt.
fn downscale_png(png_bytes: &[u8], target_w: u32) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let img = image::load_from_memory(png_bytes)?.to_rgb8();
    let (iw, ih) = img.dimensions();
    if iw <= target_w {
        return Ok(png_bytes.to_vec());
    }
    let target_h = ((target_w as f64) * (ih as f64) / (iw as f64)).round() as u32;
    let small = image::imageops::resize(&img, target_w, target_h, image::imageops::FilterType::Triangle);
    let mut buf = Vec::new();
    image::codecs::png::PngEncoder::new(&mut std::io::Cursor::new(&mut buf)).write_image(
        small.as_raw(),
        small.width(),
        small.height(),
        image::ExtendedColorType::Rgb8,
    )?;
    Ok(buf)
}

/// Zoom fin sur chaque zone saillante : crop + analyse détaillée via le modèle.
/// PARALLÉLISÉ : chaque zone est analysée dans son propre thread (le serveur
/// mistralrs corrigé gère les requêtes concurrentes).
fn zoom_zones(png_bytes: &[u8], zones: &[SalientZone]) -> Result<(), Box<dyn std::error::Error>> {
    let mut img = image::load_from_memory(png_bytes)?;
    let (iw, ih) = img.dimensions();

    // Prépare tous les crops (séquentiel, rapide)
    let mut crops: Vec<(u32, u32, u32, u32, Vec<u8>)> = Vec::new();
    for z in zones.iter() {
        let mw = (z.w as f64 * 0.1) as u32;
        let mh = (z.h as f64 * 0.1) as u32;
        let x = z.x.saturating_sub(mw);
        let y = z.y.saturating_sub(mh);
        let w = (z.w + 2 * mw).min(iw - x);
        let hh = (z.h + 2 * mh).min(ih - y);
        let crop = img.crop(x, y, w, hh);
        let mut buf = Vec::new();
        image::codecs::png::PngEncoder::new(&mut std::io::Cursor::new(&mut buf)).write_image(
            crop.as_bytes(),
            crop.width(),
            crop.height(),
            image::ExtendedColorType::Rgb8,
        )?;
        crops.push((x, y, w, hh, buf));
    }

    // Analyse PARALLÈLE des zones
    let mut handles = Vec::new();
    for (i, (x, y, w, hh, buf)) in crops.iter().enumerate() {
        let buf = buf.clone();
        let q = format!(
            "Tu es dans une zone zoomée d'un écran (coin x:{}, y:{}). \
             Lis TOUS les textes visibles mot à mot, décris les boutons, icônes et éléments d'interface précisément.",
            x, y
        );
        let xi = *x;
        let yi = *y;
        let wi = *w;
        let hi = *hh;
        handles.push(std::thread::spawn(move || {
            let start = std::time::Instant::now();
            let result = analyze_image(&buf, &q);
            let secs = start.elapsed().as_secs();
            (i, xi, yi, wi, hi, result, secs)
        }));
    }

    // Collecte dans l'ordre
    let mut results: Vec<(usize, u32, u32, u32, u32, Result<String, String>, u64)> = Vec::new();
    for h in handles {
        if let Ok(r) = h.join() {
            results.push(r);
        }
    }
    results.sort_by_key(|r| r.0);
    for (i, x, y, w, hh, result, secs) in results {
        println!("\n=== ZONE SAILLANTE {}/{} (x:{}, y:{}, {}x{}) [{}s] ===", i + 1, zones.len(), x, y, w, hh, secs);
        match result {
            Ok(ans) => println!("{}", ans),
            Err(e) => println!("ERREUR zone {}: {}", i + 1, e),
        }
    }
    Ok(())
}

/// Zoom ITÉRATIF (Iterative Narrowing) : analyse une zone, puis re-zoome les
/// sous-zones encore riches en détails — comme l'œil humain qui se pose
/// plusieurs fois pour lire un texte dense. Profondeur max `depth`.
fn zoom_deep(
    png_bytes: &[u8],
    depth: u32,
    min_cells: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    zoom_deep_rec(png_bytes, 0, depth, min_cells)
}

fn zoom_deep_rec(
    png_bytes: &[u8],
    level: u32,
    max_depth: u32,
    min_cells: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    if level >= max_depth {
        return Ok(());
    }
    let indent = "  ".repeat(level as usize);
    let mut img = image::load_from_memory(png_bytes)?;
    let (iw, ih) = img.dimensions();

    // Sous-grille 2x2 : on regarde où sont les détails dans CE crop
    let zones = saliency(png_bytes, 2, 2, 4)?;
    for (i, z) in zones.iter().enumerate() {
        let mw = (z.w as f64 * 0.1) as u32;
        let mh = (z.h as f64 * 0.1) as u32;
        let x = z.x.saturating_sub(mw);
        let y = z.y.saturating_sub(mh);
        let w = (z.w + 2 * mw).min(iw - x);
        let hh = (z.h + 2 * mh).min(ih - y);

        // Ne re-zoomer que les zones assez grandes (éviter les micro-crops)
        if w < min_cells || hh < min_cells {
            continue;
        }

        let crop = img.crop(x, y, w, hh);
        let mut buf = Vec::new();
        image::codecs::png::PngEncoder::new(&mut std::io::Cursor::new(&mut buf)).write_image(
            crop.as_bytes(),
            crop.width(),
            crop.height(),
            image::ExtendedColorType::Rgb8,
        )?;

        println!(
            "{}=== NIVEAU {} — sous-zone {}/{} (x:{}, y:{}, {}x{}) ===",
            indent,
            level + 1,
            i + 1,
            zones.len(),
            x,
            y,
            w,
            hh
        );
        let q = format!(
            "{}Tu es dans une sous-zone zoomée d'un écran (coin x:{}, y:{}). \
             Lis TOUS les textes mot à mot, décris les éléments précisément.",
            indent, x, y
        );
        match analyze_image(&buf, &q) {
            Ok(ans) => println!("{}{}", indent, ans),
            Err(e) => println!("{}ERREUR: {}", indent, e),
        }

        // Récursion : ce crop peut encore contenir des détails fins
        zoom_deep_rec(&buf, level + 1, max_depth, min_cells)?;
    }
    Ok(())
}

/// Détecte si la réponse du modèle exprime de l'incertitude (texte illisible,
/// éléments trop petits, « je ne peux pas lire »...). UI-Zoomer n'active le zoom
/// QUE si la confiance est faible — économie d'appels quand tout est clair.
fn responds_uncertain(text: &str) -> bool {
    let t = text.to_lowercase();
    let markers = [
        "pas sûr", "pas certain", "incertain", "illisible", "trop petit",
        "trop petite", "ne peux pas lire", "ne peut pas lire", "difficile à lire",
        "difficile de lire", "flou", "pas clair", "je ne vois pas", "ne vois pas",
        "peu visible", "impossible de lire", "peux pas lire", "pas de texte",
        "aucun texte", "pas lisible", "invisible", "indistinct", "pixelisé",
        "pixelise", "trop flou", "hallucin", "incertain", "pas identifi",
        "difficile à distinguer", "difficile de distinguer", "je suppose", "peut-être",
        "probablement", "semble être", "on dirait", "ressemble à",
    ];
    markers.iter().any(|m| t.contains(m))
}

/// UI-Zoomer : analyse globale, puis ne zoome que sur les zones où le modèle
/// est incertain OU qui restent riches en détails non lus. Si la réponse
/// globale est confiante, on s'arrête là (rapide).
fn uizoomer(png_bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    // 1. Vision globale avec consigne d'auto-évaluation (image réduite)
    println!("\n=== VISION GLOBALE (UI-Zoomer, image réduite) ===");
    let small_png = downscale_png(png_bytes, 384)?;
    let global_q = "Décris précisément ce qu'on voit à l'écran. Si certains textes ou \
                    éléments sont trop petits ou illisibles pour être lus avec certitude, \
                    dis-le explicitement ('texte illisible', 'trop petit pour lire').";
    let global = analyze_image(&small_png, &global_q)?;
    println!("{}", global);

    // 2. Décision de zoom : incertitude du modèle OU détails riches restants
    let uncertain = responds_uncertain(&global);
    println!("\n[Confiance globale: {}]", if uncertain { "INSUFFISANTE — zoom nécessaire" } else { "OK — pas de zoom" });

    if uncertain {
        // Zoom sur les zones saillantes pour résoudre l'incertitude
        let zones = saliency(png_bytes, 4, 3, 4)?;
        println!("[Zoom ciblé sur {} zones d'intérêt]", zones.len());
        zoom_zones(png_bytes, &zones)?;
    } else {
        println!("[Analyse globale suffisante — aucun appel supplémentaire]");
    }
    Ok(())
}

// Position de la souris via CGEventGetLocation (CoreGraphics, sans permission).
// CGEventGetLocation exige un event valide : on crée un event système (CGEventCreate)
// dont la localisation est la position actuelle du curseur.
fn mouse_pos() -> (f64, f64) {
    use core_graphics::event::CGEvent;
    

    match CGEventSource::new(CGEventSourceStateID::CombinedSessionState) {
        Ok(src) => match CGEvent::new(src) {
            Ok(ev) => {
                let p = ev.location();
                (p.x, p.y)
            }
            Err(_) => (0.0, 0.0),
        },
        Err(_) => (0.0, 0.0),
    }
}

fn write_mouse(home: &str, pos: (f64, f64)) {
    let _ = std::fs::write(
        format!("{}/souris.json", home),
        format!("{{\"x\":{:.0},\"y\":{:.0}}}\n", pos.0, pos.1),
    );
}

/// Déplace la souris puis clique (gauche/droite) à (x, y) via CGEvent.
/// Nécessite l'accessibilité (System Settings → Confidentialité → Accessibilité).
fn mouse_click(
    x: f64,
    y: f64,
    down_type: CGEventType,
    up_type: CGEventType,
    button: CGMouseButton,
) -> Result<(), Box<dyn std::error::Error>> {
    use core_graphics::event::{CGEvent, CGEventTapLocation};
    
    let src = CGEventSource::new(CGEventSourceStateID::CombinedSessionState)
        .map_err(|e| format!("CGEventSource: {:?}", e))?;
    let pt = core_graphics::geometry::CGPoint::new(x, y);
    let move_ev = CGEvent::new_mouse_event(src.clone(), CGEventType::MouseMoved, pt, button)
        .map_err(|_| "mouse move failed")?;
    move_ev.post(CGEventTapLocation::HID);
    std::thread::sleep(std::time::Duration::from_millis(80));
    let down = CGEvent::new_mouse_event(src.clone(), down_type, pt, button)
        .map_err(|_| "mouse down failed")?;
    down.post(CGEventTapLocation::HID);
    std::thread::sleep(std::time::Duration::from_millis(60));
    let up = CGEvent::new_mouse_event(src.clone(), up_type, pt, button)
        .map_err(|_| "mouse up failed")?;
    up.post(CGEventTapLocation::HID);
    Ok(())
}

/// Clic gauche à (x, y).
fn click_at(x: f64, y: f64) -> Result<(), Box<dyn std::error::Error>> {
    use core_graphics::event::{CGMouseButton, CGEventType};
    mouse_click(x, y, CGEventType::LeftMouseDown, CGEventType::LeftMouseUp, CGMouseButton::Left)?;
    println!("🖱️ Clic à ({:.0}, {:.0})", x, y);
    Ok(())
}

/// Clic droit à (x, y) — ouvre le menu contextuel.
fn right_click_at(x: f64, y: f64) -> Result<(), Box<dyn std::error::Error>> {
    use core_graphics::event::{CGMouseButton, CGEventType};
    mouse_click(
        x,
        y,
        CGEventType::RightMouseDown,
        CGEventType::RightMouseUp,
        CGMouseButton::Right,
    )?;
    println!("🖱️ Clic droit à ({:.0}, {:.0})", x, y);
    Ok(())
}

/// Double-clic gauche à (x, y).
fn double_click_at(x: f64, y: f64) -> Result<(), Box<dyn std::error::Error>> {
    click_at(x, y)?;
    std::thread::sleep(std::time::Duration::from_millis(120));
    click_at(x, y)
}

/// Fait défiler la molette à la position actuelle du curseur.
/// `lines` : positif = vers le bas, négatif = vers le haut (comme la molette).
fn scroll_at(x: f64, y: f64, lines: i32) -> Result<(), Box<dyn std::error::Error>> {
    use core_graphics::event::{CGEvent, CGEventTapLocation};
    let src = CGEventSource::new(CGEventSourceStateID::CombinedSessionState)
        .map_err(|e| format!("CGEventSource: {:?}", e))?;
    // Déplacer d'abord la souris sur la cible
    let pt = core_graphics::geometry::CGPoint::new(x, y);
    let move_ev = CGEvent::new_mouse_event(src.clone(), CGEventType::MouseMoved, pt, CGMouseButton::Left)
        .map_err(|_| "mouse move failed")?;
    move_ev.post(CGEventTapLocation::HID);
    std::thread::sleep(std::time::Duration::from_millis(80));
    // Événement de molette : unité = ligne, delta sur l'axe 1 (vertical)
    let scroll = CGEvent::new_scroll_event(
        src,
        core_graphics::event::ScrollEventUnit::LINE,
        1,
        lines,
        0,
        0,
    )
    .map_err(|_| "scroll event failed")?;
    scroll.post(CGEventTapLocation::HID);
    println!("🖱️ Scroll {} lignes à ({:.0}, {:.0})", lines, x, y);
    Ok(())
}

/// Zone d'intérêt détectée par l'analyse de saillance.
struct SalientZone {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    score: f64,
}

/// Carte de saillance : découpe l'image en cellules et calcule un score
/// d'intérêt pour chacune (contraste + variance de luminance), comme l'œil
/// humain qui repère d'abord ce qui ressort du reste.
/// Retourne les `top` zones les plus intéressantes, fusionnées si proches.
fn saliency(png_bytes: &[u8], cols: u32, rows: u32, top: usize) -> Result<Vec<SalientZone>, Box<dyn std::error::Error>> {
    let img = image::load_from_memory(png_bytes)?;
    let gray = img.to_luma8();
    let (iw, ih) = gray.dimensions();
    let cw = iw / cols;
    let ch = ih / rows;

    let mut cells: Vec<(u32, u32, f64)> = Vec::new();
    for r in 0..rows {
        for c in 0..cols {
            let x = c * cw;
            let y = r * ch;
            let w = if c == cols - 1 { iw - x } else { cw };
            let h = if r == rows - 1 { ih - y } else { ch };

            // Moyenne + variance de luminance sur la cellule
            let mut sum = 0.0_f64;
            let mut sum2 = 0.0_f64;
            let mut n = 0.0_f64;
            for py in y..y + h {
                for px in x..x + w {
                    let v = gray.get_pixel(px, py)[0] as f64;
                    sum += v;
                    sum2 += v * v;
                    n += 1.0;
                }
            }
            let mean = sum / n;
            let variance = (sum2 / n - mean * mean).max(0.0);
            // Le contraste local (variance) révèle texte, boutons, bordures.
            // Une zone totalement uniforme (fond) a variance ~0 → peu d'intérêt.
            let score = variance.sqrt(); // écart-type ≈ contraste perçu
            cells.push((x, y, score));
        }
    }

    // Tri décroissant par score, garde les `top` meilleures
    cells.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
    let best: Vec<(u32, u32, f64)> = cells.into_iter().take(top).collect();

    // Convertit chaque cellule gagnante en zone avec dimensions réelles
    let zones = best
        .into_iter()
        .map(|(x, y, score)| {
            let w = if x + cw > iw { iw - x } else { cw };
            let h = if y + ch > ih { ih - y } else { ch };
            SalientZone { x, y, w, h, score }
        })
        .collect();
    Ok(zones)
}

/// Carte de saillance COLORÉE : repère les zones dont les couleurs ressortent.
/// L'œil humain est instinctivement attiré par les couleurs chaudes et saturées
/// (rouge, orange = alertes, erreurs, boutons d'action) plutôt que les tons
/// neutres/uniformes. Score = saturation moyenne + bonus teinte chaude.
fn color_saliency(
    png_bytes: &[u8],
    cols: u32,
    rows: u32,
    top: usize,
) -> Result<Vec<SalientZone>, Box<dyn std::error::Error>> {
    let img = image::load_from_memory(png_bytes)?;
    let rgb = img.to_rgb8();
    let (iw, ih) = rgb.dimensions();
    let cw = iw / cols;
    let ch = ih / rows;

    let mut cells: Vec<(u32, u32, f64)> = Vec::new();
    for r in 0..rows {
        for c in 0..cols {
            let x = c * cw;
            let y = r * ch;
            let w = if c == cols - 1 { iw - x } else { cw };
            let h = if r == rows - 1 { ih - y } else { ch };

            let mut sat_sum = 0.0_f64;
            let mut warm_sum = 0.0_f64;
            let mut n = 0.0_f64;
            for py in y..y + h {
                for px in x..x + w {
                    let p = rgb.get_pixel(px, py);
                    let (r_, g, b) = (p[0] as f64, p[1] as f64, p[2] as f64);
                    let max = r_.max(g).max(b);
                    let min = r_.min(g).min(b);
                    // Saturation HSV : 0 = gris neutre, 1 = couleur pure
                    let sat = if max == 0.0 { 0.0 } else { (max - min) / max };
                    sat_sum += sat;
                    // Bonus "chaleur" : rouge/orange (r >> b, r élevé)
                    if r_ > 100.0 && r_ > b * 1.5 {
                        warm_sum += (r_ - b) / 255.0;
                    }
                    n += 1.0;
                }
            }
            let sat_mean = sat_sum / n;
            let warm_mean = warm_sum / n;
            // Score = saturation moyenne + 2× présence de couleurs chaudes
            let score = sat_mean * 100.0 + warm_mean * 200.0;
            cells.push((x, y, score));
        }
    }

    cells.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
    let best: Vec<(u32, u32, f64)> = cells.into_iter().take(top).collect();

    let zones = best
        .into_iter()
        .map(|(x, y, score)| {
            let w = if x + cw > iw { iw - x } else { cw };
            let h = if y + ch > ih { ih - y } else { ch };
            SalientZone { x, y, w, h, score }
        })
        .collect();
    Ok(zones)
}

/// Carte d'ATTENTION COMBINÉE : fusionne les 3 canaux de saillance
/// (contraste + couleur + mouvement) comme le cortex visuel humain intègre
/// forme, couleur et mouvement en parallèle. Chaque score est normalisé
/// entre 0 et 1 puis combiné avec des poids (contraste 1.0, couleur 0.8,
/// mouvement 1.2 — le mouvement est prioritaire car c'est ce qui change).
fn attention(
    prev: &[u8],
    curr: &[u8],
    cols: u32,
    rows: u32,
    top: usize,
) -> Result<Vec<SalientZone>, Box<dyn std::error::Error>> {
    let a = image::load_from_memory(prev)?;
    let b = image::load_from_memory(curr)?;
    let gray_a = a.to_luma8();
    let gray_b = b.to_luma8();
    let rgb_b = b.to_rgb8();
    let (iw, ih) = gray_a.dimensions();
    let cw = iw / cols;
    let ch = ih / rows;

    let mut cells: Vec<(u32, u32, f64, f64, f64)> = Vec::new();
    for r in 0..rows {
        for c in 0..cols {
            let x = c * cw;
            let y = r * ch;
            let w = if c == cols - 1 { iw - x } else { cw };
            let h = if r == rows - 1 { ih - y } else { ch };

            let mut sum = 0.0_f64;
            let mut sum2 = 0.0_f64;
            let mut sat_sum = 0.0_f64;
            let mut warm_sum = 0.0_f64;
            let mut diff_sum = 0.0_f64;
            let mut n = 0.0_f64;
            for py in y..y + h {
                for px in x..x + w {
                    let va = gray_a.get_pixel(px, py)[0] as f64;
                    let vb = gray_b.get_pixel(px, py)[0] as f64;
                    sum += vb;
                    sum2 += vb * vb;
                    diff_sum += (va - vb).abs();
                    let p = rgb_b.get_pixel(px, py);
                    let (r_, g, bl) = (p[0] as f64, p[1] as f64, p[2] as f64);
                    let max = r_.max(g).max(bl);
                    let min = r_.min(g).min(bl);
                    let sat = if max == 0.0 { 0.0 } else { (max - min) / max };
                    sat_sum += sat;
                    if r_ > 100.0 && r_ > bl * 1.5 {
                        warm_sum += (r_ - bl) / 255.0;
                    }
                    n += 1.0;
                }
            }
            let mean = sum / n;
            let variance = (sum2 / n - mean * mean).max(0.0);
            let contrast = variance.sqrt(); // écart-type ≈ contraste perçu
            let color = sat_sum / n * 100.0 + warm_sum / n * 200.0;
            let motion = diff_sum / n; // différence moyenne de luminance
            cells.push((x, y, contrast, color, motion));
        }
    }

    // Normalisation min-max de chaque canal pour les rendre comparables
    let max_contrast = cells.iter().map(|c| c.2).fold(0.0_f64, f64::max).max(1e-9);
    let max_color = cells.iter().map(|c| c.3).fold(0.0_f64, f64::max).max(1e-9);
    let max_motion = cells.iter().map(|c| c.4).fold(0.0_f64, f64::max).max(1e-9);

    // Score combiné pondéré (mouvement prioritaire)
    let mut scored: Vec<(u32, u32, f64)> = cells
        .into_iter()
        .map(|(x, y, contrast, color, motion)| {
            let s = contrast / max_contrast * 1.0
                + color / max_color * 0.8
                + motion / max_motion * 1.2;
            (x, y, s)
        })
        .collect();

    scored.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
    let best: Vec<(u32, u32, f64)> = scored.into_iter().take(top).collect();

    let zones = best
        .into_iter()
        .map(|(x, y, score)| {
            let w = if x + cw > iw { iw - x } else { cw };
            let h = if y + ch > ih { ih - y } else { ch };
            SalientZone { x, y, w, h, score }
        })
        .collect();
    Ok(zones)
}

/// Carte de saillance de MOUVEMENT : compare deux captures et repère les zones
/// qui ont changé. L'œil humain détecte le mouvement même infime (cellules
/// ganglionnaires directionnelles) — sur un écran, ce qui bouge = curseur,
/// notifications, animations, chargement = souvent critique.
fn motion_saliency(
    prev: &[u8],
    curr: &[u8],
    cols: u32,
    rows: u32,
    top: usize,
) -> Result<Vec<SalientZone>, Box<dyn std::error::Error>> {
    let a = image::load_from_memory(prev)?.to_luma8();
    let b = image::load_from_memory(curr)?.to_luma8();
    let (iw, ih) = a.dimensions();
    let cw = iw / cols;
    let ch = ih / rows;

    let mut cells: Vec<(u32, u32, f64)> = Vec::new();
    for r in 0..rows {
        for c in 0..cols {
            let x = c * cw;
            let y = r * ch;
            let w = if c == cols - 1 { iw - x } else { cw };
            let h = if r == rows - 1 { ih - y } else { ch };

            let mut diff_sum = 0.0_f64;
            let mut n = 0.0_f64;
            for py in y..y + h {
                for px in x..x + w {
                    let va = a.get_pixel(px, py)[0] as f64;
                    let vb = b.get_pixel(px, py)[0] as f64;
                    diff_sum += (va - vb).abs();
                    n += 1.0;
                }
            }
            let score = diff_sum / n; // différence moyenne de luminance = mouvement
            cells.push((x, y, score));
        }
    }

    cells.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
    let best: Vec<(u32, u32, f64)> = cells.into_iter().take(top).collect();

    let zones = best
        .into_iter()
        .map(|(x, y, score)| {
            let w = if x + cw > iw { iw - x } else { cw };
            let h = if y + ch > ih { ih - y } else { ch };
            SalientZone { x, y, w, h, score }
        })
        .collect();
    Ok(zones)
}
