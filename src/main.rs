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
use foreign_types::ForeignType;
use screencapturekit::screenshot_manager::{CGImageExt, SCScreenshotManager};
use image::ImageEncoder;
use image::GenericImageView;

mod analyse;
mod palais;

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
        "--mousepos" => {
            // VÉRITÉ TERRAIN : position réelle du curseur en coordonnées écran.
            // C'est la référence pour apprendre le parallèle OCR → réalité.
            let (x, y) = mouse_pos();
            println!("🐭 SOURIS RÉELLE: ({:.0}, {:.0})", x, y);
        }
        "--cursor" => {
            // CURSEUR PERSISTANT : TON LOGO (S doré) reste TOUJOURS visible et
            // suit la souris en continu (leçon utilisateur : « ton curseur doit
            // être tout le temps visible »). Pattern cua-driver : UNE SEULE
            // fenêtre NSWindow créée une fois, DÉPLACÉE à chaque tick avec
            // setFrameOrigin — jamais de recréation (qui accumulait des
            // fenêtres fantômes et donnait un axe inversé).
            // Usage: ecran-live --cursor [durée_secondes]  (défaut: 60)
            use objc::{class, msg_send, runtime::Object, sel, sel_impl};
            let dur: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(60);
            let start = Instant::now();
            println!("👁️  Curseur Sypherine persistant pendant {}s (Ctrl+C pour arrêter)", dur);

            // Créer UNE SEULE fenêtre avec le logo
            let win = match create_cursor_window() {
                Some(w) => w,
                None => {
                    println!("❌ Impossible de créer la fenêtre curseur");
                    return Ok(());
                }
            };
            let run_loop: *mut Object = unsafe { msg_send![class!(NSRunLoop), currentRunLoop] };
            let mode: *mut Object = unsafe { msg_send![class!(NSString),
                stringWithUTF8String: c"kCFRunLoopDefaultMode".as_ptr().cast::<u8>()
            ] };
            unsafe {
                let _: () = msg_send![win, orderFrontRegardless];
            }
            while start.elapsed().as_secs() < dur {
                let (x, y) = mouse_pos();
                // RÉFÉRENTIELS (enfin compris, vérifié) :
                //   CGEvent::location() = HAUT-gauche (souris en haut → y petit)
                //   NSWindow setFrameOrigin = BAS-gauche (y=0 en bas)
                // DONC conversion OBLIGATOIRE : y_ns = 2160 - y - 48.
                // Sans elle, souris en haut (y=202) → fenêtre à y=202 du bas = EN BAS.
                // L'utilisateur a validé CETTE version — ne jamais la retirer.
                let y_ns = 2160.0 - y - 48.0;
                let frame = NSRect { x: x - 2.0, y: y_ns - 2.0, w: 48.0, h: 48.0 };
                unsafe {
                    let _: () = msg_send![win, setFrameOrigin: core_graphics::geometry::CGPoint::new(frame.x, frame.y)];
                }
                // Pump court (8ms) pour laisser Core Animation peindre
                let until: *mut Object = unsafe { msg_send![class!(NSDate),
                    dateWithTimeIntervalSinceNow: 0.008
                ] };
                let _: bool = unsafe { msg_send![run_loop, runMode: mode beforeDate: until] };
            }
            unsafe {
                let _: () = msg_send![win, close];
                let _: () = msg_send![win, release];
            }
            println!("⌛ Curseur Sypherine terminé");
        }
        "--movepid" => {
            // DÉPLACE le curseur à (x, y) + marqueur rose visible, SANS cliquer.
            // Usage: ecran-live --movepid X Y [pid]
            // On VOIT où le curseur est avant de cliquer (leçon : œil → main).
            let x: f64 = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(0.0);
            let y: f64 = args.get(2).and_then(|a| a.parse().ok()).unwrap_or(0.0);
            let pt = core_graphics::geometry::CGPoint::new(x, y);
            use core_graphics::display::CGDisplay;
            match CGDisplay::warp_mouse_cursor_position(pt) {
                Ok(_) => {
                    unsafe {
                        extern "C" {
                            fn CGAssociateMouseAndMouseCursorPosition(connected: bool) -> i32;
                        }
                        CGAssociateMouseAndMouseCursorPosition(true);
                    }
                    println!("🎯 Curseur déplacé à ({:.0}, {:.0}) + marqueur", x, y);
                    show_marker(x, y, 1500);
                }
                Err(e) => println!("❌ warp failed: {:?}", e),
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
        "--stream" => {
            // FLUX CONTINU (vision en direct, pattern « yeux toujours ouverts »).
            // Usage: ecran-live --stream [fps] [largeur]
            //   fps   : images par seconde (défaut 2 — léger en RAM/CPU)
            //   largeur: résolution de capture (défaut 1600)
            // ÉCONOMIE RAM (Mac 8 Go — point sensible) :
            //   • buffer tournant : UNE seule image en mémoire à la fois
            //     (Vec locale, drop() automatique à chaque itération)
            //   • détection de changement : on n'analyse QUE si l'image a
            //     bougé (empreinte perceptive 64px) — le monde est souvent
            //     statique, on évite de ré-analyser le vide
            //   • downscale 384px pour l'analyse VLM (÷10 du prefill)
            //   • aucun fichier écrit sur disque (tout en mémoire)
            let fps: f64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(2.0);
            let stream_w: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1600);
            let fps = fps.clamp(0.2, 10.0);
            let cap = Capteur::new()?;
            let h = (stream_w as f64 * cap.ratio).round() as u32;
            let interval = Duration::from_secs_f64(1.0 / fps);
            println!(
                "📺 Mode STREAM : {} fps, capture {}x{} (vision continue, RAM minimale)",
                fps, stream_w, h
            );
            println!("    Buffer tournant : 1 image en mémoire (Ctrl+C pour arrêter)");

            let mut last_fp: Option<Vec<u8>> = None;
            let mut frame_n = 0u64;
            let mut changed_n = 0u64;
            let mut last_report = Instant::now();
            let mut last_analysis = Instant::now() - Duration::from_secs(10);
            // ══ VISION DIRECTE AVEC TÂCHES EN PARALLÈLE (12/08) ══
            // Les yeux (capture) ne s'arrêtent JAMAIS : pendant que le cerveau
            // (VLM) analyse une frame dans un thread de fond, la boucle
            // continue de capturer. On ne rate aucun événement.
            let mut vlm_thread: Option<std::thread::JoinHandle<(u64, String)>> = None;

            loop {
                let frame_start = Instant::now();
                // 1. Capture PNG en mémoire (buffer tournant — l'ancienne Vec
                //    est libérée quand on réassigne)
                let png = match cap.capture_bytes(stream_w, h) {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!("⚠️ capture: {}", e);
                        std::thread::sleep(interval);
                        continue;
                    }
                };
                frame_n += 1;

                // 2. Détection de changement : empreinte perceptive 64px.
                //    On compare avec un SEUIL de différence (≥1% des pixels
                //    changés) pour ignorer les micro-changements (curseur,
                //    clignotements HUD) qui saturaient le VLM (bug freeze).
                let fp = match fingerprint(&png) {
                    Ok(f) => f,
                    Err(_) => vec![],
                };
                let changed = match &last_fp {
                    Some(old) => fp_diff_ratio(old, &fp) >= 0.01,
                    None => true,
                };
                last_fp = Some(fp);
                if !changed {
                    std::thread::sleep(interval.saturating_sub(frame_start.elapsed()));
                    continue;
                }
                changed_n += 1;

                // 2b. RATE-LIMIT : au plus 1 analyse VLM toutes les 2s, même
                //     si l'écran change en continu (sinon mlxcel est saturé
                //     → freeze système). C'est le fix du bug de saturation.
                let min_interval = Duration::from_millis(2000);
                if last_analysis.elapsed() < min_interval {
                    std::thread::sleep(interval.saturating_sub(frame_start.elapsed()));
                    continue;
                }
                last_analysis = Instant::now();

                // ══ ANALYSE EN THREAD DE FOND (vision directe parallèle) ══
                // La capture continue PENDANT que le VLM analyse : on empile
                // la frame à analyser, le thread de fond s'en occupe, et la
                // boucle repart immédiatement capturer la suivante.
                if let Some(t) = vlm_thread.take() {
                    if let Ok((f, ans)) = t.join() {
                        let one_line: String = ans
                            .lines()
                            .map(str::trim)
                            .filter(|l| !l.is_empty() && !l.starts_with("COORD:"))
                            .take(1)
                            .collect::<Vec<_>>()
                            .join(" ");
                        println!("🟢 frame {} (changée): {}", f, one_line);
                    }
                }

                // 3. Analyse VLM rapide (image réduite 384px — prefill ÷10)
                //    Le prefix-cache mlxcel accélère encore : images voisines
                //    partagent le préfixe du prompt → tokens économisés.
                let small_png = match downscale_png(&png, 384) {
                    Ok(s) => s,
                    Err(_) => {
                        std::thread::sleep(interval);
                        continue;
                    }
                };
                let q = format!(
                    "Analyse d'écran en direct (frame {}) : que se passe-t-il ? \
                     Décris les changements visibles, les textes, les éléments actifs.",
                    frame_n
                );
                let f = frame_n;
                vlm_thread = Some(std::thread::spawn(move || {
                    let ans = analyze_image(&small_png, &q).unwrap_or_else(|e| e);
                    (f, ans)
                }));

                // Rapport périodique (RAM + stats) toutes les 10s
                if last_report.elapsed() >= Duration::from_secs(10) {
                    let mem = process_mem_mb();
                    println!(
                        "📊 {}s : {} frames ({} changées) — RAM processus ≈ {} MB",
                        last_report.elapsed().as_secs(),
                        frame_n,
                        changed_n,
                        mem
                    );
                    last_report = Instant::now();
                }

                // Garder la cadence (intervalle - temps déjà passé)
                let remain = interval.saturating_sub(frame_start.elapsed());
                std::thread::sleep(remain);
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
        // ecran-live --scan [largeur] [top_zones] [question]
        //   SCAN RAPIDE — la boucle sous 1s : capture → saillance pixels (0.02s)
        //   → crops ciblés → VLM sur les zones (pas l'écran entier).
        //   Le vision tower sur l'écran complet coûte ~6s (157 tokens) vs ~0.9s
        //   sur un crop (90 tokens) — cibler les zones divise par 5 le temps.
        //   Contrainte mlxcel --parallel 1 : les analyses VLM se sérialisent,
        //   donc on limite à top_zones=2 par défaut (2 × ~0.9s).
        "--scan" => {
            let cap = Capteur::new()?;
            let scan_w: u32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(1600);
            let top: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(2);
            let question = args.get(3).cloned()
                .unwrap_or_else(|| "Décris précisément ce que tu vois dans cette zone.".to_string());
            let h = (scan_w as f64 * cap.ratio).round() as u32;
            let t0 = std::time::Instant::now();
            let png = cap.capture_bytes(scan_w, h)?;
            println!("📸 Capture {}x{} en {:.2}s", scan_w, h, t0.elapsed().as_secs_f64());

            // 1. Saillance pixel native (0.02s) — les yeux
            let t1 = std::time::Instant::now();
            let zones = saliency(&png, 8, 5, top)?;
            println!("🎯 {} zone(s) saillante(s) en {:.3}s", zones.len(), t1.elapsed().as_secs_f64());
            for (i, z) in zones.iter().enumerate() {
                println!("   zone {}/{} : x={}, y={}, {}x{}", i + 1, zones.len(), z.x, z.y, z.w, z.h);
            }

            // 2. VLM sur les crops (ciblés, pas l'écran entier)
            let t2 = std::time::Instant::now();
            zoom_zones(&png, &zones)?;
            println!("⏱️  Scan complet en {:.2}s (pixels {:.2}s + VLM zones {:.2}s)",
                t0.elapsed().as_secs_f64(),
                t1.elapsed().as_secs_f64(),
                t2.elapsed().as_secs_f64());
        }
        // ecran-live --fovea [largeur] [top_zones] [question]
        //   FOVÉATION EN 1 PASSE (biomimétique, 13/08) — comme le cerveau :
        //   1. Les yeux (pixels 0.02s) trouvent les zones saillantes
        //   2. La fovéa (VLM) analyse TOUTES les zones dans UNE mosaïque
        //   → 1 seul appel VLM au lieu de N : 4 zones = 1.6s vs 8.8s (5.5x)
        "--fovea" => {
            let cap = Capteur::new()?;
            let fovea_w: u32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(1600);
            let top: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(4);
            let question = args.get(3).cloned()
                .unwrap_or_else(|| "Décris brièvement ce que tu vois dans cette grille de zones.".to_string());
            let h = (fovea_w as f64 * cap.ratio).round() as u32;
            let t0 = std::time::Instant::now();
            let png = cap.capture_bytes(fovea_w, h)?;
            println!("📸 Capture {}x{} en {:.2}s", fovea_w, h, t0.elapsed().as_secs_f64());

            // 1. Les yeux : saillance pixel native (0.02s)
            let t1 = std::time::Instant::now();
            let zones = saliency(&png, 8, 5, top)?;
            println!("🎯 {} zone(s) saillante(s) en {:.3}s", zones.len(), t1.elapsed().as_secs_f64());
            if zones.is_empty() {
                println!("⚠️  Aucune zone saillante détectée");
                return Ok(());
            }

            // 2. La fovéa : mosaïque de TOUTES les zones en UNE image
            let t2 = std::time::Instant::now();
            let mosaic = build_fovea_mosaic(&png, &zones)?;
            let vlm_result = analyze_image(&mosaic, &question)?;
            println!("🟢 Fovéa (1 appel): {vlm_result}");
            println!("⏱️  Fovéation en {:.2}s (pixels {:.2}s + mosaïque+VLM {:.2}s)",
                t0.elapsed().as_secs_f64(),
                t1.elapsed().as_secs_f64(),
                t2.elapsed().as_secs_f64());
            // Positions des zones pour le grounding
            for (i, z) in zones.iter().enumerate() {
                println!("   zone {} : x={}, y={}, {}x{}", i + 1, z.x, z.y, z.w, z.h);
            }
        }
        // ecran-live --veille [largeur] [secondes] [question]
        //   VISION PRÉDICTIVE + HABITUATION + ANTICIPATION (biomimétique, 13/08)
        //   Le cerveau complet :
        //   - predictive coding : ne traite QUE l'erreur de prédiction
        //   - habituation (Montaldi 2006) : dépense moins sur les scènes familières
        //   - mémoire de travail 7±2 (Miller 1956) : hot cache des réponses
        //   - ANTICIPATION CYCLIQUE : apprend les cycles A→B→C→A et PRÉDIT
        //     la frame suivante (le cortex prédictif — ne regarde plus ce
        //     qu'il connaît déjà par cœur)
        //   - VOCABULAIRE DE DELTAS : mémorise (empreinte du CHANGEMENT →
        //     signification) — quand un même delta revient, on connaît déjà
        //     l'événement (la rétine ne transmet que les événements)
        //   - NEUROGENÈSE : les seuils s'auto-ajustent selon le contexte
        //     (écran animé → inhibition plus permissive ; écran statique →
        //     habituation plus agressive)
        "--veille" => {
            let cap = Capteur::new()?;
            let veille_w: u32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(1600);
            let secondes: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(30);
            let question = args.get(3).cloned()
                .unwrap_or_else(|| "Que s'est-il passé à l'écran ? Réponds en une phrase.".to_string());
            let h = (veille_w as f64 * cap.ratio).round() as u32;
            let mut prev: Option<Vec<u8>> = None;
            let debut = std::time::Instant::now();
            let mut tours = 0u64;
            let mut analyses = 0u64;
            let mut habitudes = 0u64;
            let mut anticipations = 0u64;
            let mut deltas_connus = 0u64;
            // Hippocampe : empreinte perceptive → réponse VLM (habituation)
            let mut hippocampe: Vec<(Vec<u8>, String)> = Vec::new();
            // Cortex prédictif : empreinte → (empreinte suivante prédite, force)
            // L'anticipation cyclique = quand A a toujours été suivi de B,
            // le cerveau s'attend à B quand il voit A.
            let mut cortex: Vec<(Vec<u8>, Vec<u8>, u32)> = Vec::new();
            // Vocabulaire de deltas : empreinte du changement → signification.
            // La rétine ne transmet que les ÉVÉNEMENTS ; le cerveau apprend
            // « ce type de changement = ce type d'événement ».
            let mut vocabulaire: Vec<(Vec<u8>, String)> = Vec::new();
            // Neurogenèse : seuil d'inhibition adaptatif (démarre à 0.05%)
            let mut seuil_inhibition: f64 = 0.05;
            let mut micros_vus: u64 = 0;
            let mut reels_vus: u64 = 0;
            // Vision entrelacée (micro-saccades × TV) : phase qui tourne 0,1,2
            // → chaque scan ne coûte qu'1/3 du diff, tout l'écran couvert en
            // 3 tours (comme la rétine qui rafraîchit par petites touches).
            let mut phase_entrelace: u32 = 0;

            let chercher_familier = |empr: &[u8], mem: &[(Vec<u8>, String)]| -> Option<String> {
                let mut best: Option<(usize, &String)> = None;
                for (e, r) in mem {
                    let dist = e.iter().zip(empr).filter(|(a, b)| a != b).count();
                    if let Some((bd, _)) = best {
                        if dist < bd { best = Some((dist, r)); }
                    } else {
                        best = Some((dist, r));
                    }
                }
                best.and_then(|(d, r)| if d <= 12 { Some(r.clone()) } else { None })
            };
            // Recherche dans le vocabulaire de deltas (distance Hamming)
            let chercher_delta = |empr: &[u8], voc: &[(Vec<u8>, String)]| -> Option<String> {
                let mut best: Option<(usize, &String)> = None;
                for (e, r) in voc {
                    let dist = e.iter().zip(empr).filter(|(a, b)| a != b).count();
                    if let Some((bd, _)) = best {
                        if dist < bd { best = Some((dist, r)); }
                    } else {
                        best = Some((dist, r));
                    }
                }
                best.and_then(|(d, r)| if d <= 12 { Some(r.clone()) } else { None })
            };
            // Anticipation : à partir de l'empreinte courante, la prochaine
            // attendue (si le cortex a appris le cycle)
            let predire = |empr: &[u8], cx: &[(Vec<u8>, Vec<u8>, u32)]| -> Option<Vec<u8>> {
                let mut best: Option<(u32, &Vec<u8>)> = None;
                for (e, suiv, force) in cx {
                    let dist = e.iter().zip(empr).filter(|(a, b)| a != b).count();
                    if dist <= 12 {
                        if let Some((bf, _)) = best {
                            if *force > bf { best = Some((*force, suiv)); }
                        } else {
                            best = Some((*force, suiv));
                        }
                    }
                }
                best.map(|(_, s)| s.clone())
            };

            println!("🧠  Cerveau complet — {}s, capture {}x{} (prédiction + habituation + anticipation + deltas)", secondes, veille_w, h);
            while debut.elapsed().as_secs() < secondes {
                tours += 1;
                let png = cap.capture_bytes(veille_w, h)?;
                let empr = fingerprint(&png)?;

                // 1) ANTICIPATION CYCLIQUE : le cortex prédit cette frame ?
                let predite = predire(&empr, &cortex);
                if let Some(pred) = &predite {
                    let dist = pred.iter().zip(&empr).filter(|(a, b)| a != b).count();
                    if dist <= 12 {
                        anticipations += 1;
                        println!("   [t{}] ⚡ ANTICIPÉ (cycle connu, 0 capture d'analyse) | {}",
                                 tours, chercher_familier(&empr, &hippocampe).unwrap_or_else(|| "cycle".to_string()));
                        prev = Some(png);
                        std::thread::sleep(std::time::Duration::from_millis(500));
                        continue;
                    }
                }

                // 2) DIFF ENTRELACÉ : le monde a-t-il changé ? (1/3 du coût,
                // phase qui tourne → tout couvert en 3 tours, micro-saccades)
                if let Some(prev_bytes) = &prev {
                    match analyse::diff_bbox_entrelace(prev_bytes, &png, 30, 3, phase_entrelace)? {
                        None => {
                            if let Some(reponse) = chercher_familier(&empr, &hippocampe) {
                                habitudes += 1;
                                println!("   [t{}] immobile + familier → habituation (0 VLM) | {}", tours, reponse);
                            } else {
                                println!("   [t{}] immobile — aucune analyse", tours);
                            }
                        }
                        Some((x0, y0, x1, y1, pct)) => {
                            let m = 30u32;
                            let cx0 = x0.saturating_sub(m);
                            let cy0 = y0.saturating_sub(m);
                            let cx1 = (x1 + m).min(veille_w);
                            let cy1 = (y1 + m).min(h);
                            if pct < seuil_inhibition {
                                micros_vus += 1;
                                println!("   [t{}] micro-changement ({:.3}%) ignoré (inhibition {:.2}%)", tours, pct, seuil_inhibition);
                                if let Some(reponse) = chercher_familier(&empr, &hippocampe) {
                                    habitudes += 1;
                                    println!("   [t{}] familier → réutilise la réponse (0 VLM) | {}", tours, reponse);
                                }
                            } else {
                                // 3) VOCABULAIRE DE DELTAS : ce changement a-t-il
                                // déjà été vu et compris ?
                                let empr_delta = fingerprint(&png)?;
                                if let Some(reponse) = chercher_delta(&empr_delta, &vocabulaire) {
                                    deltas_connus += 1;
                                    println!("   [t{}] CHANGEMENT {:.2}% CONNU (vocabulaire deltas, 0 VLM) | {}", tours, pct, reponse);
                                } else {
                                    // VLM sur la zone CHANGÉE (predictive coding)
                                    let crop = analyse::crop_bytes_png(&png, cx0, cy0, cx1, cy1, 1)?;
                                    let t0 = std::time::Instant::now();
                                    let reponse = analyze_image(&crop, &question)?;
                                    analyses += 1;
                                    reels_vus += 1;
                                    // Apprend : la frame courante devient familière
                                    hippocampe.push((empr.clone(), reponse.clone()));
                                    // Apprend le DELTA (rétine → vocabulaire)
                                    vocabulaire.push((empr_delta, reponse.clone()));
                                    println!("   [t{}] CHANGEMENT {:.2}% bbox=({},{})-({},{}) → {:.2}s | {}", tours, pct, cx0, cy0, cx1, cy1, t0.elapsed().as_secs_f64(), reponse);
                                }
                            }
                        }
                    }
                } else {
                    let t0 = std::time::Instant::now();
                    let reponse = analyze_image(&png, &question)?;
                    analyses += 1;
                    reels_vus += 1;
                    hippocampe.push((empr.clone(), reponse.clone()));
                    println!("   [t{}] état initial → {:.2}s | {}", tours, t0.elapsed().as_secs_f64(), reponse);
                }

                // 4) APPRENTISSAGE DU CYCLE (cortex prédictif) : l'ancienne
                // empreinte (prev) a mené à la nouvelle (empr). Renforce la
                // transition — c'est comme ça que le cerveau apprend A→B.
                if let Some(prev_bytes) = &prev {
                    let empr_prev = fingerprint(prev_bytes)?;
                    let mut trouve = false;
                    for (e, suiv, force) in cortex.iter_mut() {
                        let d = e.iter().zip(&empr_prev).filter(|(a, b)| a != b).count();
                        if d <= 12 {
                            // Transition déjà connue → renforce
                            let d2 = suiv.iter().zip(&empr).filter(|(a, b)| a != b).count();
                            if d2 <= 12 {
                                *force = (*force + 1).min(100);
                            } else {
                                *force = force.saturating_sub(1);
                            }
                            trouve = true;
                        }
                    }
                    if !trouve {
                        cortex.push((empr_prev, empr.clone(), 1));
                    }
                    // Limite la taille du cortex (neurogenèse : garde les forts)
                    if cortex.len() > 50 {
                        cortex.sort_by_key(|(_, _, f)| std::cmp::Reverse(*f));
                        cortex.truncate(50);
                    }
                }

                // 5) NEUROGENÈSE : ajuste le seuil d'inhibition selon le
                // contexte (si beaucoup de micros, l'écran est animé → on
                // inhibe plus ; si beaucoup de réels, l'écran est actif → on
                // inhibe moins pour ne rien manquer)
                if micros_vus + reels_vus >= 3 {
                    let ratio_micros = micros_vus as f64 / (micros_vus + reels_vus) as f64;
                    if ratio_micros > 0.6 {
                        seuil_inhibition = (seuil_inhibition * 1.5).min(0.5); // écran animé
                    } else if ratio_micros < 0.3 {
                        seuil_inhibition = (seuil_inhibition * 0.8).max(0.02); // écran actif
                    }
                    micros_vus = 0;
                    reels_vus = 0;
                }

                prev = Some(png);
                // Micro-saccade : phase suivante de l'entrelacement
                phase_entrelace = (phase_entrelace + 1) % 3;
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            println!("⏱️  Cerveau terminé : {} tours | {} analyses VLM | {} habituation | {} anticipations ⚡ | {} deltas connus | seuil inhibition final {:.2}%",
                tours, analyses, habitudes, anticipations, deltas_connus, seuil_inhibition);
        }
        // ecran-live --blob [largeur] [secondes] [question]
        //   LE BLOB-RÉSEAU (Physarum polycephalum, biomimétique) :
        //   comme le réseau de veines du blob — les veines où passe la
        //   nourriture deviennent ÉPAISSES (débit ↑), les inutilisées
        //   s'ATROPHIENT (débit ↓). Ici : l'écran est divisé en zones,
        //   chaque zone a un SCORE DE PASSAGE (activité mesurée par les
        //   pixels). Les zones actives reçoivent PLUS de requêtes VLM
        //   (débit ↑), les zones calmes MOINS (débit ↓) → la RAM et les
        //   requêtes s'allouent dynamiquement selon le contexte réel.
        //   Pipeline :
        //   1. GRILLE : découpe l'écran en G×G zones (défaut 3×3)
        //   2. MESURE : diff entrelacé → incrémente le score de la zone
        //      où il y a du mouvement (le "passage" de nourriture)
        //   3. ADAPTE : les zones chaudes sont analysées souvent (débit
        //      élevé), les tièdes moins, les froides jamais (atrophie)
        //   4. RAPPORTE : "il se passe X à (zone chaud, x,y)" + direction
        "--blob" => {
            let cap = Capteur::new()?;
            let blob_w: u32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(1600);
            let secondes: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(30);
            let question = args.get(3).cloned()
                .unwrap_or_else(|| "Que se passe-t-il dans cette zone ? Décris l'action en une phrase.".to_string());
            let h = (blob_w as f64 * cap.ratio).round() as u32;
            let debut = std::time::Instant::now();
            let mut tours = 0u64;
            let mut analyses = 0u64;
            let mut suivis = 0u64;
            let mut phase_entrelace: u32 = 0;
            let mut prev: Option<Vec<u8>> = None;
            let mut derniere_requete = std::time::Instant::now() - std::time::Duration::from_secs(10);
            // GRILLE : 3×3 zones — chaque zone a un SCORE DE PASSAGE
            // (combien de fois le blob y a vu du mouvement). Les zones
            // chaudes → débit élevé, les froides → atrophie.
            let (gz, gc) = (3usize, 3usize); // grille 3×3
            let mut scores = vec![0u32; gz * gc];
            // Le score décroît avec le temps (une zone qui se calme
            // redevient froide — l'atrophie des veines)
            let mut description_par_zone_idx: Vec<Option<String>> = vec![None; gz * gc];

            println!("🕵️  BLOB-RÉSEAU — {}s, grille {}×{} zones, débit adaptatif (veines épaisses où ça passe)", secondes, gz, gc);
            while debut.elapsed().as_secs() < secondes {
                tours += 1;
                let png = cap.capture_bytes(blob_w, h)?;

                // 1) MESURE : diff entrelacé → bbox du mouvement
                let mouvement = if let Some(p) = &prev {
                    analyse::diff_bbox_entrelace(p, &png, 30, 3, phase_entrelace)?
                } else { None };

                if let Some((x0, y0, x1, y1, pct)) = mouvement {
                    let cx = (x0 + x1) / 2;
                    let cy = (y0 + y1) / 2;
                    // Quelle zone de la grille ? (le "passage" du blob)
                    let zx = ((cx as f64 / blob_w as f64) * gc as f64).min((gc - 1) as f64) as usize;
                    let zy = ((cy as f64 / h as f64) * gz as f64).min((gz - 1) as f64) as usize;
                    let zi = zy * gc + zx;
                    // 2) ADAPTE : renforce la veine (score ↑)
                    scores[zi] = scores[zi].saturating_add(1).min(100);

                    // Débit adaptatif : intervalle selon le score de la zone
                    // (chaude = 3s, tiède = 6s, froide = jamais)
                    let debit_s = if scores[zi] >= 5 { 3 } else if scores[zi] >= 2 { 6 } else { 15 };
                    let intervalle = std::time::Duration::from_secs(debit_s);
                    let intervalle_ecoule = derniere_requete.elapsed() >= intervalle;
                    let jamais_analyse = description_par_zone_idx[zi].is_none();

                    if intervalle_ecoule || jamais_analyse {
                        // 3) ANALYSE (débit élevé sur la veine épaisse)
                        let bw = (x1 - x0).max(1);
                        let bh = (y1 - y0).max(1);
                        let m = bw.max(bh).max(30) * 2;
                        let cx0 = cx.saturating_sub(m);
                        let cy0 = cy.saturating_sub(m);
                        let cx1 = (cx + m).min(blob_w);
                        let cy1 = (cy + m).min(h);
                        let crop = analyse::crop_bytes_png(&png, cx0, cy0, cx1, cy1, 1)?;
                        let t0 = std::time::Instant::now();
                        let reponse = analyze_image(&crop, &question)?;
                        analyses += 1;
                        derniere_requete = std::time::Instant::now();
                        description_par_zone_idx[zi] = Some(reponse.clone());
                        println!("   [t{}] 🕵️  VEINE ÉPAISSE zone[{},{}] score={} (débit {}s) → {:.2}s | {}", tours, zx, zy, scores[zi], debit_s, t0.elapsed().as_secs_f64(), reponse);
                    } else {
                        // 4) SUIVI gratuit (0 requête — la veine est fine ici)
                        suivis += 1;
                        let desc = description_par_zone_idx[zi].as_deref().unwrap_or("—");
                        println!("   [t{}] 🕵️  VEINE FINE zone[{},{}] score={} (0 requête — atrophie) | {}", tours, zx, zy, scores[zi], desc);
                    }
                } else {
                    // CALME : toutes les veines s'atrophient (score décroît)
                    for s in scores.iter_mut() {
                        if *s > 0 { *s -= 1; }
                    }
                    println!("   [t{}] 🕵️  calme — veines s'atrophient (0 requête)", tours);
                }
                prev = Some(png);
                phase_entrelace = (phase_entrelace + 1) % 3;
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            // Bilan des veines (carte thermique du trafic)
            println!("⏱️  BLOB-RÉSEAU terminé : {} tours | {} requêtes VLM | {} suivis | carte des veines :", tours, analyses, suivis);
            for y in 0..gz {
                let mut ligne = String::from("     ");
                for x in 0..gc {
                    let s = scores[y * gc + x];
                    ligne.push_str(&format!("[{:>3}] ", s));
                }
                println!("{}", ligne);
            }
        }
        // ecran-live --blob-test <dossier> [question]
        //   BLOB-ESPION EN MODE TEST : lit les frames PNG d'un dossier
        //   (mouvement synthétique) comme des captures successives.
        //   Déterministe — valide l'observation du mouvement et l'information.
        "--blob-test" => {
            let dossier = args.get(1).cloned().unwrap_or_default();
            let question = args.get(2).cloned()
                .unwrap_or_else(|| "Que se passe-t-il dans cette zone ? Décris l'action en une phrase.".to_string());
            if dossier.is_empty() {
                println!("Usage: ecran-live --blob-test <dossier> [question]");
                return Ok(());
            }
            let mut frames: Vec<std::path::PathBuf> = std::fs::read_dir(&dossier)
                .map_err(|e| format!("lecture {} : {}", dossier, e))?
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().map(|x| x == "png").unwrap_or(false))
                .collect();
            frames.sort();
            println!("🕵️  BLOB-ESPION TEST — {} frames depuis {} (rate-limit: 1 requête/3 frames)", frames.len(), dossier);
            let mut analyses = 0u64;
            let mut suivis = 0u64;
            let mut derniere_pos: Option<(u32, u32)> = None;
            let mut trajectoire: Vec<(u32, u32)> = Vec::new();
            let mut prev: Option<Vec<u8>> = None;
            let mut description_courante = String::new();
            for (i, f) in frames.iter().enumerate() {
                let png = std::fs::read(f).map_err(|e| format!("{} : {}", f.display(), e))?;
                let mouvement = if let Some(p) = &prev {
                    analyse::diff_bbox_entrelace(p, &png, 30, 3, (i as u32) % 3)?
                } else { None };
                if let Some((x0, y0, x1, y1, pct)) = mouvement {
                    let cx = (x0 + x1) / 2;
                    let cy = (y0 + y1) / 2;
                    derniere_pos = Some((cx, cy));
                    trajectoire.push((cx, cy));
                    if trajectoire.len() > 5 { trajectoire.remove(0); }
                    let dir = if trajectoire.len() >= 2 {
                        let (ax, ay) = trajectoire[trajectoire.len() - 2];
                        let (bx, by) = trajectoire[trajectoire.len() - 1];
                        let (dx, dy) = (bx as i64 - ax as i64, by as i64 - ay as i64);
                        if dx.abs() > dy.abs() {
                            if dx > 0 { "→ droite" } else { "← gauche" }
                        } else {
                            if dy > 0 { "↓ bas" } else { "↑ haut" }
                        }
                    } else { "" };
                    // RATE-LIMIT : 1 requête toutes les 3 frames + nouvel objet
                    let doit_analyser = description_courante.is_empty() || i % 3 == 0;
                    if doit_analyser {
                        let bw = (x1 - x0).max(1);
                        let bh = (y1 - y0).max(1);
                        let m = bw.max(bh).max(30) * 2;
                        let cx0 = cx.saturating_sub(m);
                        let cy0 = cy.saturating_sub(m);
                        let (w, hh) = image::load_from_memory(&png).map(|im| (im.width(), im.height()))?;
                        let cx1 = (cx + m).min(w);
                        let cy1 = (cy + m).min(hh);
                        let crop = analyse::crop_bytes_png(&png, cx0, cy0, cx1, cy1, 1)?;
                        let t0 = std::time::Instant::now();
                        let reponse = analyze_image(&crop, &question)?;
                        analyses += 1;
                        description_courante = reponse.clone();
                        println!("   [f{}] 🕵️  RAPPORT à ({},{}) → {:.2}s | {} | {}", i, cx, cy, t0.elapsed().as_secs_f64(), reponse, dir);
                    } else {
                        suivis += 1;
                        println!("   [f{}] 🕵️  SUIT à ({},{}) {} (0 requête — rate-limit) | {}", i, cx, cy, dir, description_courante);
                    }
                } else if derniere_pos.is_some() {
                    println!("   [f{}] 🕵️  CALME — plus de mouvement (0 analyse)", i);
                    derniere_pos = None;
                    trajectoire.clear();
                    description_courante.clear();
                } else {
                    println!("   [f{}] 🕵️  calme — rien à signaler (0 analyse)", i);
                }
                prev = Some(png);
            }
            println!("⏱️  BLOB-ESPION TEST terminé : {} frames | {} requêtes VLM | {} suivis pixels | économie {} requêtes évitées",
                frames.len(), analyses, suivis, suivis.saturating_sub(analyses));
        }
        // ecran-live --contexte [largeur] [question] [x0 y0 x1 y1]
        //   PRISE DE CONSCIENCE DU CONTEXTE GLOBAL (biomimétique 13/08) :
        //   le cerveau maintient un MODÈLE MENTAL de la scène — il ne
        //   re-regarde jamais tout, il ANCRE chaque regard dans ce qu'il
        //   sait déjà. Testé : le VLM comprend bien mieux avec un cadre
        //   (5.15s, décrit la zone) que sans (perdu, 1.87s).
        //   Pipeline :
        //   1. ANALYSE GLOBALE : écran réduit 512px → le VLM établit le
        //      contexte (« c'est un éditeur ») — le MODÈLE MENTAL
        //   2. ANALYSE LOCALE (si zone donnée) : crop de la zone + question
        //      ENRICHIE du contexte → qualité fine ET fiable (le VLM sait
        //      où il est, il ne perd plus le fil)
        "--contexte" => {
            let cap = Capteur::new()?;
            let ctx_w: u32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(1600);
            let question = args.get(2).cloned()
                .unwrap_or_else(|| "Décris précisément ce que tu vois à l'écran.".to_string());
            // Zone optionnelle : x0 y0 x1 y1 (analyse locale enrichie)
            let zx0: u32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0);
            let zy0: u32 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(0);
            let zx1: u32 = args.get(5).and_then(|s| s.parse().ok()).unwrap_or(0);
            let zy1: u32 = args.get(6).and_then(|s| s.parse().ok()).unwrap_or(0);
            let a_zone = zx1 > 0 && zy1 > 0;
            let h = (ctx_w as f64 * cap.ratio).round() as u32;
            let png = cap.capture_bytes(ctx_w, h)?;

            println!("🧠  CONTEXTE GLOBAL — capture {}x{}, établit le modèle mental", ctx_w, h);

            // 1) ANALYSE GLOBALE : l'écran entier réduit (le cerveau regarde
            // la pièce avant de regarder les détails)
            let img = image::load_from_memory(&png)?;
            let (w, ih) = img.dimensions();
            // Réduit à 512px max (sweet spot VLM)
            let scale = 512.0 / w.max(ih) as f64;
            let nw = (w as f64 * scale).max(1.0) as u32;
            let nh = (ih as f64 * scale).max(1.0) as u32;
            let small = img.resize(nw, nh, image::imageops::FilterType::Triangle);
            let mut buf = Vec::new();
            image::codecs::png::PngEncoder::new(&mut std::io::Cursor::new(&mut buf)).write_image(
                &small.to_rgb8().into_raw(), nw, nh, image::ExtendedColorType::Rgb8)?;
            let t0 = std::time::Instant::now();
            let contexte = analyze_image(&buf, &question)?;
            println!("   [global] écran {}x{} → {}x{} → {:.2}s", w, ih, nw, nh, t0.elapsed().as_secs_f64());
            println!("   🧠 MODÈLE MENTAL : {}", contexte);

            // 2) ANALYSE LOCALE ENRICHIE (si zone donnée)
            if a_zone {
                let crop = analyse::crop_bytes_png(&png, zx0, zy0, zx1, zy1, 1)?;
                let t1 = std::time::Instant::now();
                let locale = analyze_image_ctx(&crop, &question, &contexte)?;
                println!("   [local] zone ({},{})-({},{}) avec contexte → {:.2}s", zx0, zy0, zx1, zy1, t1.elapsed().as_secs_f64());
                println!("   🔍 ANALYSE LOCALE (enrichie) : {}", locale);
            }
            return Ok(());
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
            let target = args.get(1).cloned().unwrap_or_else(|| "PINNED".to_string());
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

            // 1. OCR + matching flou (meilleur match, centre du bbox)
            if let Some((text, x, y, score)) = find_text_ocr_best(&global_png, &target) {
                let scale = cap.scale_to_display(ocr_w);
                println!(
                    "🎯 TROUVÉ « {} » en OCR pur à ({:.0}, {:.0}) [score {}/2]",
                    text, x, y, score
                );
                println!("↔️ Remap ×{:.2} → écran réel ({:.0}, {:.0})", scale, x * scale, y * scale);
                println!("LARGEUR_ECRAN {}\n", cap.display_w);
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
            let target = args.get(1).cloned().unwrap_or_else(|| "PINNED".to_string());
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

            if let Some((text, cx, cy, score)) = find_text_ocr_best(&global_png, &target) {
                let scale = cap.scale_to_display(ocr_w);
                let real_x = cx * scale;
                let real_y = cy * scale;
                println!(
                    "🎯 TROUVÉ « {} » au centre ({:.0}, {:.0}) [score {}/2]",
                    text, cx, cy, score
                );
                println!(
                    "↔️ Remap ×{:.2} → écran réel ({:.0}, {:.0})",
                    scale, real_x, real_y
                );
                // find_text_ocr_best retourne déjà le CENTRE du bounding box →
                // pas d'offset arbitraire (leçon computer-use : viser le centre).
                if right {
                    right_click_at(real_x, real_y)?;
                    println!("✅ Clic droit effectué sur « {} »", target);
                } else if dbl {
                    double_click_at(real_x, real_y)?;
                    println!("✅ Double-clic effectué sur « {} »", target);
                } else {
                    click_at(real_x, real_y)?;
                    println!("✅ Clic effectué sur « {} »", target);
                }
            } else {
                println!("❌ « {} » introuvable à l'écran", target);
            }
        }
        "--marker" => {
            // Test isolé du marqueur visuel : affiche un carré rose à (x, y)
            // pendant `ms` ms. Usage: ecran-live --marker <x> <y> [ms]
            let x: f64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(800.0);
            let y: f64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(450.0);
            let ms: u64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(1500);
            println!("🎨 Marqueur à ({:.0}, {:.0}) pendant {}ms", x, y, ms);
            show_marker(x, y, ms);
        }
        "--ax-prompt" => {
            // Déclenche la POPUP système de permission Accessibility (TCC).
            // L'utilisateur valide → macOS accorde la permission à ecran-live.
            // (Les rebuilds changent le cdhash → permission révoquée → les
            //  CGEventPost sont ignorés silencieusement. Cette popup la restaure.)
            unsafe {
                extern "C" {
                    fn AXIsProcessTrustedWithOptions(options: *mut std::ffi::c_void) -> bool;
                    fn CFDictionaryCreateMutable(
                        allocator: *mut std::ffi::c_void,
                        capacity: std::os::raw::c_long,
                        keyCallBacks: *const std::ffi::c_void,
                        valueCallBacks: *const std::ffi::c_void,
                    ) -> *mut std::ffi::c_void;
                    fn CFDictionarySetValue(
                        dict: *mut std::ffi::c_void,
                        key: *const std::ffi::c_void,
                        value: *const std::ffi::c_void,
                    );
                    fn CFStringCreateWithCString(
                        allocator: *mut std::ffi::c_void,
                        cStr: *const std::os::raw::c_char,
                        encoding: u32,
                    ) -> *mut std::ffi::c_void;
                    fn CFBooleanGetTypeID() -> u64;
                    fn kCFBooleanTrue() -> *mut std::ffi::c_void;
                    fn kCFBooleanFalse() -> *mut std::ffi::c_void;
                }
                let kCFAllocatorDefault: *mut std::ffi::c_void = std::ptr::null_mut();
                let dict = CFDictionaryCreateMutable(kCFAllocatorDefault, 0, std::ptr::null(), std::ptr::null());
                let key = CFStringCreateWithCString(kCFAllocatorDefault, c"AXTrustedCheckOptionPrompt".as_ptr().cast(), 0x08000100);
                let val = kCFBooleanTrue();
                CFDictionarySetValue(dict, key as *const std::ffi::c_void, val as *const std::ffi::c_void);
                let trusted = AXIsProcessTrustedWithOptions(dict);
                println!("AX prompt affiché → trusted: {}", trusted);
                println!("→ Va dans Réglages Système > Confidentialité > Accessibilité");
                println!("→ Cocher « ecran-live » si la popup n'apparaît pas.");
                let _ = CFBooleanGetTypeID();
                let _ = kCFBooleanFalse();
            }
        }
        "--ax-trusted" => {
            // Teste si CE processus a la permission Accessibility (TCC).
            // Les rebuilds changent le cdhash → macOS révoque la permission →
            // les CGEventPost sont ignorés SILENCIEUSEMENT.
            unsafe {
                extern "C" {
                    fn AXIsProcessTrusted() -> bool;
                }
                println!("AX trusted: {}", AXIsProcessTrusted());
            }
        }
        "--clic-cua" => {
            // TEST : pattern EXACT du blog cua "The primer click" :
            // primer down/up à (-1,-1) sur HID, puis down/up réel sur HID.
            // Usage: ecran-live --clic-cua <x> <y>
            use core_graphics::event::{CGEvent, CGEventTapLocation, CGEventType};
            use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
            use core_graphics::event::CGMouseButton;
            let x: f64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(800.0);
            let y: f64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(450.0);
            let src = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
                .map_err(|e| format!("CGEventSource: {:?}", e))?;
            let primer_pt = core_graphics::geometry::CGPoint::new(-1.0, -1.0);
            let pd = CGEvent::new_mouse_event(src.clone(), CGEventType::LeftMouseDown, primer_pt, CGMouseButton::Left)
                .map_err(|_| "primer down failed")?;
            pd.post(CGEventTapLocation::HID);
            std::thread::sleep(std::time::Duration::from_millis(20));
            let pu = CGEvent::new_mouse_event(src.clone(), CGEventType::LeftMouseUp, primer_pt, CGMouseButton::Left)
                .map_err(|_| "primer up failed")?;
            pu.post(CGEventTapLocation::HID);
            std::thread::sleep(std::time::Duration::from_millis(50));
            let pt = core_graphics::geometry::CGPoint::new(x, y);
            let down = CGEvent::new_mouse_event(src.clone(), CGEventType::LeftMouseDown, pt, CGMouseButton::Left)
                .map_err(|_| "down failed")?;
            down.post(CGEventTapLocation::HID);
            std::thread::sleep(std::time::Duration::from_millis(50));
            let up = CGEvent::new_mouse_event(src, CGEventType::LeftMouseUp, pt, CGMouseButton::Left)
                .map_err(|_| "up failed")?;
            up.post(CGEventTapLocation::HID);
            println!("✅ clic cua (primer HID + clic HID) à ({:.0}, {:.0})", x, y);
        }
        "--clic-osxrdp" => {
            // TEST : combinaison EXACTE du fix bho3538/osxrdp :
            // Session tap + MOUSE_EVENT_NUMBER croissant, SANS primer.
            // Usage: ecran-live --clic-osxrdp <x> <y>
            use core_graphics::event::{CGEvent, CGEventTapLocation, CGEventType, EventField};
            use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
            use core_graphics::event::CGMouseButton;
            use std::sync::atomic::{AtomicU32, Ordering};
            static CNT: AtomicU32 = AtomicU32::new(1);
            let x: f64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(800.0);
            let y: f64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(450.0);
            let num = CNT.fetch_add(1, Ordering::SeqCst) as i64;
            let src = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
                .map_err(|e| format!("CGEventSource: {:?}", e))?;
            let pt = core_graphics::geometry::CGPoint::new(x, y);
            let down = CGEvent::new_mouse_event(src.clone(), CGEventType::LeftMouseDown, pt, CGMouseButton::Left)
                .map_err(|_| "down failed")?;
            down.set_integer_value_field(EventField::MOUSE_EVENT_CLICK_STATE, 1);
            down.set_integer_value_field(EventField::MOUSE_EVENT_NUMBER, num);
            down.post(CGEventTapLocation::Session);
            std::thread::sleep(std::time::Duration::from_millis(50));
            let up = CGEvent::new_mouse_event(src, CGEventType::LeftMouseUp, pt, CGMouseButton::Left)
                .map_err(|_| "up failed")?;
            up.set_integer_value_field(EventField::MOUSE_EVENT_CLICK_STATE, 1);
            up.set_integer_value_field(EventField::MOUSE_EVENT_NUMBER, num);
            up.post(CGEventTapLocation::Session);
            println!("✅ clic osxrdp (Session+num {}) à ({:.0}, {:.0})", num, x, y);
        }
        "--clic-pid-nu" => {
            // TEST : clic PID comme le CLAVIER (qui fonctionne !) :
            // post_to_pid(auth=false pour souris — leçon blog cua) vers le PID
            // principal, SANS primer, SANS warp. Le blog : « CGEvent.postToPid
            // works great on everything except Chrome » — la recette SIMPLE
            // (down/up direct, sans primer Chromium) est celle que WebKit
            // accepte. Usage: ecran-live --clic-pid-nu <x> <y> <pid>
            use core_graphics::event::{CGEvent, CGEventType};
            use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
            use core_graphics::event::CGMouseButton;
            let x: f64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(800.0);
            let y: f64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(450.0);
            let pid: i32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(653);
            let src = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
                .map_err(|e| format!("CGEventSource: {:?}", e))?;
            let pt = core_graphics::geometry::CGPoint::new(x, y);
            let down = CGEvent::new_mouse_event(src.clone(), CGEventType::LeftMouseDown, pt, CGMouseButton::Left)
                .map_err(|_| "down failed")?;
            let ptr = &*down as *const _ as *mut std::ffi::c_void;
            unsafe {
                skylight::set_integer_field(ptr, 1, 1);   // clickState=1
                skylight::set_integer_field(ptr, 3, 0);   // bouton gauche
                skylight::set_integer_field(ptr, 7, 3);   // NSEventSubtypeTouch
                skylight::set_integer_field(ptr, 40, pid as i64);
            }
            let _ = unsafe { skylight::post_to_pid(pid, ptr, false) };
            down.post_to_pid(pid);
            std::thread::sleep(std::time::Duration::from_millis(50));
            let up = CGEvent::new_mouse_event(src, CGEventType::LeftMouseUp, pt, CGMouseButton::Left)
                .map_err(|_| "up failed")?;
            let ptr2 = &*up as *const _ as *mut std::ffi::c_void;
            unsafe {
                skylight::set_integer_field(ptr2, 1, 1);
                skylight::set_integer_field(ptr2, 3, 0);
                skylight::set_integer_field(ptr2, 7, 3);
                skylight::set_integer_field(ptr2, 40, pid as i64);
            }
            let _ = unsafe { skylight::post_to_pid(pid, ptr2, false) };
            up.post_to_pid(pid);
            println!("✅ clic PID nu (auth=false, simple) à ({:.0}, {:.0}) → pid {}", x, y, pid);
        }
        "--clic-ax" => {
            // ══ CLIC PAR ÉLÉMENT AX — MÉCANISME EXACT DE COMPUTER_USE ══
            // cua-driver : « element-indexed clicks fire the underlying AX
            // action directly, work on hidden targets, and don't involve
            // coordinates » → JAMAIS de curseur système touché.
            // 1. AXUIElementCopyElementAtPosition(app, x, y, &el) → élément
            //    sous le point (coordonnées POINTS de l'écran principal)
            // 2. AXUIElementPerformAction(el, kAXPressAction) → AXPress
            // Usage: ecran-live --clic-ax <pid> <x_pixels> <y_pixels>
            use accessibility::AXUIElement;
            use core_foundation::base::TCFType;
            use core_foundation::string::CFString;
            let pid_ax: i32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
            let x_px: f64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0.0);
            let y_px: f64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0.0);
            if pid_ax == 0 {
                println!("Usage: ecran-live --clic-ax <pid> <x_pixels> <y_pixels>");
                return Ok(());
            }
            // Conversion pixels physiques → points (scale = pixels / points)
            use core_graphics::display::CGDisplay;
            let disp = CGDisplay::main();
            let b = disp.bounds();
            let scale_x = disp.pixels_wide() as f64 / b.size.width as f64;
            let scale_y = disp.pixels_high() as f64 / b.size.height as f64;
            let x_pt = (x_px / scale_x) as f32;
            let y_pt = (y_px / scale_y) as f32;

            let app_elem = AXUIElement::application(pid_ax);
            let mut el: accessibility_sys::AXUIElementRef = std::ptr::null_mut();
            let err = unsafe {
                accessibility_sys::AXUIElementCopyElementAtPosition(
                    app_elem.as_concrete_TypeRef() as *mut _,
                    x_pt,
                    y_pt,
                    &mut el,
                )
            };
            if err != 0 || el.is_null() {
                println!("❌ Aucun élément AX sous ({:.0},{:.0}) px = ({:.1},{:.1}) pt — erreur {}", x_px, y_px, x_pt, y_pt, err);
                return Ok(());
            }
            // L'élément trouvé : quel est-il ? (role + description)
            let el_obj = unsafe { AXUIElement::wrap_under_create_rule(el) };
            let role = el_obj
                .attribute::<CFString>(&accessibility::AXAttribute::role())
                .map(|v| v.to_string())
                .unwrap_or_default();
            let label = el_obj
                .attribute::<CFString>(&accessibility::AXAttribute::title())
                .or_else(|_| el_obj.attribute::<CFString>(&accessibility::AXAttribute::description()))
                .map(|v| v.to_string())
                .unwrap_or_default();
            println!("🎯 Élément AX sous ({:.0},{:.0})px : role={} label=«{}»", x_px, y_px, role, label);

            // AXPress — l'action directe (le clic de computer_use)
            let press = CFString::new("AXPress");
            let r = el_obj.perform_action(&press);
            match r {
                Ok(_) => println!("✅ AXPress réussi sur {} «{}»", role, label),
                Err(e) => println!("❌ AXPress échoué : {:?}", e),
            }
        }
        "--clic-ax-label" => {
            // ══ AXPRESS PAR LABEL — LE CLIC DE COMPUTER_USE, EN NATIF ══
            // Parcourt l'AX tree de l'app, trouve l'élément dont le label
            // contient `target`, et déclenche AXPress DIRECTEMENT dessus.
            // Aucune coordonnée, aucun event → le curseur système n'est JAMAIS
            // touché (prouvé par computer_use : compteur 6→7 sans mouvement).
            // Usage: ecran-live --clic-ax-label <pid> <label>
            let pid_axl: i32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
            let label_target = args.get(2).cloned().unwrap_or_else(|| "CLIQUE".to_string());
            if pid_axl == 0 {
                println!("Usage: ecran-live --clic-ax-label <pid> <label>");
                return Ok(());
            }
            match ax_press_by_label(pid_axl, &label_target) {
                Some((label, cx, cy)) => {
                    println!("🎯 AXPress réussi sur «{}» au centre ({:.0},{:.0}) px", label, cx, cy);
                }
                None => {
                    println!("❌ Aucun élément «{}» trouvé ou AXPress échoué (pid {})", label_target, pid_axl);
                }
            }
        }
        "--clic-wc" => {
            // CLIC DIRECT AU RENDERER WEB CONTENT (piste blog cua) : le clic
            // posté au process principal Safari est FILTRÉ à la frontière IPC
            // (« your click lands in the outer window process, then vanishes »).
            // Ici on poste la recette COMPLÈTE cua (mouseMoved + primer +
            // down/up + window-local + tous les champs) DIRECTEMENT au PID du
            // renderer WebContent. post_to_pid → curseur système JAMAIS touché.
            // Usage: ecran-live --clic-wc <x> <y> <wx> <wy> <wid> <pid_wc>
            let sx: f64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(0.0);
            let sy: f64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0.0);
            let wx: f64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0.0);
            let wy: f64 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(0.0);
            let wid: u32 = args.get(5).and_then(|s| s.parse().ok()).unwrap_or(0);
            let pid_wc: i32 = args.get(6).and_then(|s| s.parse().ok()).unwrap_or(0);
            if pid_wc == 0 {
                println!("Usage: ecran-live --clic-wc <x> <y> <wx> <wy> <wid> <pid_wc>");
                return Ok(());
            }
            let avant = mouse_pos();
            click_cua_events(pid_wc, wid, sx, sy, wx, wy)?;
            std::thread::sleep(std::time::Duration::from_millis(300));
            let apres = mouse_pos();
            println!("🎯 Clic complet → renderer WebContent pid {} (wid {}) à ({:.0},{:.0}) — curseur AVANT ({:.0},{:.0}) APRÈS ({:.0},{:.0})",
                pid_wc, wid, sx, sy, avant.0, avant.1, apres.0, apres.1);
            if (avant.0 - apres.0).abs() < 1.0 && (avant.1 - apres.1).abs() < 1.0 {
                println!("✅ CURSEUR INTACT — post_to_pid ne touche jamais au curseur système");
            } else {
                println!("⚠️ CURSEUR A BOUGÉ !");
            }
        }
        "--clic-simple" => {
            // TEST : clic MINIMAL sans primer click, sans warp, sans numéro
            // (isoler le vrai problème). Usage: ecran-live --clic-simple <x> <y>
            use core_graphics::event::{CGEvent, CGEventTapLocation, CGEventType};
            use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
            use core_graphics::event::CGMouseButton;
            let x: f64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(800.0);
            let y: f64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(450.0);
            let src = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
                .map_err(|e| format!("CGEventSource: {:?}", e))?;
            let pt = core_graphics::geometry::CGPoint::new(x, y);
            let down = CGEvent::new_mouse_event(src.clone(), CGEventType::LeftMouseDown, pt, CGMouseButton::Left)
                .map_err(|_| "down failed")?;
            down.post(CGEventTapLocation::HID);
            std::thread::sleep(std::time::Duration::from_millis(50));
            let up = CGEvent::new_mouse_event(src, CGEventType::LeftMouseUp, pt, CGMouseButton::Left)
                .map_err(|_| "up failed")?;
            up.post(CGEventTapLocation::HID);
            println!("✅ clic simple HID à ({:.0}, {:.0})", x, y);
        }
        "--clic-tap-disso" => {
            // TEST DÉCISIF (12/08, interrompu) : down/up au tap SESSION avec
            // CGAssociateMouseAndMouseCursorPosition(false) AUTOUR — le clic
            // au tap Session arrive à Safari (prouvé : compteur 5) ; la
            // désassociation DEVRAIT empêcher le curseur de bouger.
            // Usage: ecran-live --clic-tap-disso <x> <y>
            use core_graphics::event::{CGEvent, CGEventTapLocation, CGEventType};
            use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
            use core_graphics::event::CGMouseButton;
            let x: f64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(800.0);
            let y: f64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(450.0);
            // Position AVANT (lue par CGEvent — la VÉRITÉ système)
            let before = mouse_pos();
            let src = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
                .map_err(|e| format!("CGEventSource: {:?}", e))?;
            let pt = core_graphics::geometry::CGPoint::new(x, y);
            unsafe extern "C" {
                fn CGAssociateMouseAndMouseCursorPosition(associated: bool);
            }
            unsafe {
                CGAssociateMouseAndMouseCursorPosition(false);
            }
            let down = CGEvent::new_mouse_event(src.clone(), CGEventType::LeftMouseDown, pt, CGMouseButton::Left)
                .map_err(|_| "down failed")?;
            down.post(CGEventTapLocation::AnnotatedSession);
            std::thread::sleep(std::time::Duration::from_millis(50));
            let up = CGEvent::new_mouse_event(src, CGEventType::LeftMouseUp, pt, CGMouseButton::Left)
                .map_err(|_| "up failed")?;
            up.post(CGEventTapLocation::AnnotatedSession);
            std::thread::sleep(std::time::Duration::from_millis(100));
            unsafe {
                CGAssociateMouseAndMouseCursorPosition(true);
            }
            let after = mouse_pos();
            println!("🎯 clic tap Session+disso à ({:.0},{:.0}) — curseur AVANT ({:.0},{:.0}) APRÈS ({:.0},{:.0})",
                x, y, before.0, before.1, after.0, after.1);
            if (before.0 - after.0).abs() < 1.0 && (before.1 - after.1).abs() < 1.0 {
                println!("✅ CURSEUR INTACT — la désassociation empêche le mouvement !");
            } else {
                println!("⚠️ CURSEUR A BOUGÉ (interdit) — cette voie ne convient pas");
            }
        }
        "--clickxy" => {
            // Clic par COORDONNÉES directes (écran réel) — le chaînon manquant :
            // nos YEUX (OCR) trouvent le texte, puis on clique exactement là.
            // Usage: ecran-live --clickxy <x> <y>
            let x: f64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(800.0);
            let y: f64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(450.0);
            println!("🎯 Clic direct à ({:.0}, {:.0}) — nos yeux ont trouvé, nous cliquons", x, y);
            click_at(x, y)?;
        }
        "--clickpid" => {
            // Clic par coordonnées envoyé DIRECTEMENT au PID cible (post_to_pid,
            // pattern cua) — fonctionne même si l'app n'est pas au premier plan.
            // Usage: ecran-live --clickpid <x> <y> <pid>
            let x: f64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(800.0);
            let y: f64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(450.0);
            let pid: i32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(40423);
            println!("🎯 Clic PID à ({:.0}, {:.0}) → pid {} (post_to_pid, pattern cua)", x, y, pid);
            mouse_click(x, y, CGEventType::LeftMouseDown, CGEventType::LeftMouseUp, CGMouseButton::Left, Some(pid))?;
            println!("✅ Clic PID effectué");
        }
        "--type" => {
            // Tape du texte au clavier (CGEventKeyboardEvent) — le chaînon
            // manquant : nos yeux trouvent → nous cliquons → nous tapons.
            // Usage: ecran-live --type "texte à taper"
            let text = args.get(1).cloned().unwrap_or_default();
            if text.is_empty() {
                println!("⚠️ usage: ecran-live --type \"texte\"");
            } else {
                // PID Safari par défaut (post_to_pid = trusted pour champs web)
                let pid = pgrep_first("Safari").unwrap_or(40423);
                type_text_at(&text, Some(pid))?;
            }
        }
        "--typehid" => {
            // Type au HID GLOBAL (l'app active reçoit) — pour les champs du
            // chrome (barre d'adresse Safari) où post_to_pid est ignoré.
            let text = args.get(1).cloned().unwrap_or_default();
            if !text.is_empty() {
                type_text_at(&text, None)?;
            }
        }
        "--key" => {
            // Touche spéciale : Escape (fermer modal), Return (valider),
            // Tab (naviguer), flèches...
            // Usage: ecran-live --key escape
            let key = args.get(1).cloned().unwrap_or_else(|| "escape".to_string());
            key_at(&key)?;
        }
        "--axclick" => {
            // Clic via l'AX tree (leçon computer-use/cua-driver) : trouve
            // l'élément par son label même s'il est VIDE (un champ de saisie
            // n'a pas de texte OCR !), calcule le centre via AXPosition+AXSize,
            // puis clique par CGEvent (Tier 3 — fiable sur Safari, contrairement
            // à AXPress qui ment sur les vues web).
            // Usage: ecran-live --axclick Safari "Description"
            let app = args.get(1).cloned().unwrap_or_else(|| "Safari".to_string());
            let target = args.get(2).cloned().unwrap_or_default();
            if target.is_empty() {
                println!("Usage: ecran-live --axclick <APP> \"label\"");
                return Ok(());
            }
            match ax_find_element(&app, &target) {
                Some((label, cx, cy)) => {
                    println!("🔍 AX: « {} » au centre ({:.0}, {:.0})", label, cx, cy);
                    click_at(cx, cy)?;
                    println!("✅ Clic AX effectué sur « {} »", target);
                }
                None => {
                    // Notre binaire n'a pas la permission Accessibilité (TCC) —
                    // on délègue à cua-driver qui l'a (le PONT).
                    println!("🔌 AX direct indisponible — pont vers cua-driver...");
                    match cua_find_and_click(&app, &target) {
                        Some(_) => println!("✅ Clic via cua réussi"),
                        None => println!("❌ Élément « {} » introuvable (AX + cua)", target),
                    }
                }
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

            if let Some((text, cx, cy, score)) = find_text_ocr_best(&global_png, &target) {
                let scale = cap.scale_to_display(ocr_w);
                let real_x = cx * scale;
                let real_y = cy * scale;
                println!(
                    "🎯 TROUVÉ « {} » au centre → remap ×{:.2} → ({:.0}, {:.0}) [score {}/2]",
                    text, scale, real_x, real_y, score
                );
                scroll_at(real_x, real_y, lines)?;
            } else {
                println!("❌ « {} » introuvable à l'écran", target);
            }
        }
        "--watch-ram" => {
            // Watcher mémoire continu : surveille RAM process + RAM système,
            // nettoie /tmp quand la RAM LIBRE passe sous le seuil.
            // Usage: ecran-live --watch-ram [--seuil 20] [--int 5]
            //   --seuil : % de RAM libre en dessous duquel on nettoie (défaut 20)
            //   --int   : intervalle en secondes entre deux mesures (défaut 5)
            let seuil: f64 = args
                .iter()
                .position(|a| a == "--seuil")
                .and_then(|i| args.get(i + 1))
                .and_then(|s| s.parse().ok())
                .unwrap_or(20.0);
            let interval: u64 = args
                .iter()
                .position(|a| a == "--int")
                .and_then(|i| args.get(i + 1))
                .and_then(|s| s.parse().ok())
                .unwrap_or(5);
            println!(
                "🧠 Watcher RAM : nettoie si libre < {}%, mesure toutes les {}s (Ctrl+C pour arrêter)",
                seuil, interval
            );
            loop {
                let proc_mb = process_mem_mb();
                let free_pct = system_ram_free_percent();
                println!(
                    "RAM process: {} Mo | RAM système libre: {:.0}%",
                    proc_mb, free_pct
                );
                if free_pct < seuil {
                    let removed = cleanup_tmp_captures(10);
                    if removed > 0 {
                        println!(
                            "🧹 RAM libre critique ({:.0}%) — {} vieilles captures supprimées de /tmp",
                            free_pct, removed
                        );
                    }
                } else {
                    // Garde-fou CONTINU : même quand la RAM va bien, ne JAMAIS
                    // laisser /tmp dépasser ~500 Mo de captures (leçon 11/08 :
                    // les captures 4K font 8-30 Mo chacune, 2,8 Go accumulés).
                    let removed = cleanup_tmp_captures_until(10, 500);
                    if removed > 0 {
                        println!(
                            "🧹 /tmp > 500 Mo — {} vieilles captures supprimées",
                            removed
                        );
                    }
                }
                std::thread::sleep(Duration::from_secs(interval));
            }
        }
        "--flash" => {
            // ⚡ FLASH DE CALIBRATION — le parallèle entre ce qu'on CALCULE et
            // ce qui est VISIBLE. Leçons 11/08 (testées, prouvées) :
            //   • Le logo Sypherine est BLANC/GRIS (~224,224,224), PAS rose/violet
            //   • show_marker(x, y) attend des coordonnées NSWindow (BAS-gauche)
            //     → convertir y_ns = hauteur - y - 48 (pattern --cursor validé)
            //   • La capture doit venir d'un process EXTERNE (screencapture) :
            //     ScreenCaptureKit du même process ne voit pas sa propre fenêtre
            //   • ⚠️ show_marker est SYNCHRONE (affiche → pompe → FERME) : la
            //     capture doit se faire PENDANT le pump, pas après la fermeture.
            //     On recrée le pattern --cursor : UNE fenêtre + boucle pump,
            //     et screencapture est lancé PENDANT que la fenêtre est visible.
            // Usage: ecran-live --flash x y [--ms 1500]
            use objc::{class, msg_send, runtime::Object, sel, sel_impl};
            let x: f64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(1920.0);
            let y: f64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1080.0);
            let ms: u64 = args
                .iter()
                .position(|a| a == "--ms")
                .and_then(|i| args.get(i + 1))
                .and_then(|s| s.parse().ok())
                .unwrap_or(1500);
            let cap = Capteur::new()?;
            let screen_h = cap.display_w as f64 * cap.ratio;
            // Conversion Y : NSWindow = BAS-gauche.
            let y_ns = screen_h - y - 48.0;
            println!("⚡ Flash à ({:.0}, {:.0}) pendant {}ms", x, y, ms);

            // 1. Capture AVANT (screencapture externe)
            let _ = std::process::Command::new("screencapture")
                .args(["-x", "/tmp/flash_avant.png"])
                .status();
            std::thread::sleep(Duration::from_millis(150));

            // 2. Créer LA fenêtre du logo (pattern --cursor : UNE fenêtre)
            let win = match create_cursor_window() {
                Some(w) => w,
                None => {
                    println!("❌ Impossible de créer la fenêtre flash");
                    return Ok(());
                }
            };
            let run_loop: *mut Object = unsafe { msg_send![class!(NSRunLoop), currentRunLoop] };
            let rl_mode: *mut Object = unsafe { msg_send![class!(NSString),
                stringWithUTF8String: c"kCFRunLoopDefaultMode".as_ptr().cast::<u8>()
            ] };
            let frame = NSRect { x: x - 2.0, y: y_ns - 2.0, w: 48.0, h: 48.0 };
            unsafe {
                let _: () = msg_send![win, setFrameOrigin: core_graphics::geometry::CGPoint::new(frame.x, frame.y)];
                let _: () = msg_send![win, orderFrontRegardless];
            }

            // 3. Pump 400ms pour laisser Core Animation peindre le flash
            let paint_deadline = Instant::now() + Duration::from_millis(400);
            while Instant::now() < paint_deadline {
                let until: *mut Object = unsafe { msg_send![class!(NSDate),
                    dateWithTimeIntervalSinceNow: 0.016
                ] };
                let _: bool = unsafe { msg_send![run_loop, runMode: rl_mode beforeDate: until] };
            }

            // 4. Capture PENDANT — la fenêtre est VISIBLE maintenant.
            let _ = std::process::Command::new("screencapture")
                .args(["-x", "/tmp/flash_pendant.png"])
                .status();

            // 5. Fermer proprement
            unsafe {
                let _: () = msg_send![win, close];
                let _: () = msg_send![win, release];
            }

            // 6. DIFFÉRENCE : le flash = le seul changement + blanc/gris
            let avant = std::fs::read("/tmp/flash_avant.png").unwrap_or_default();
            let pendant = std::fs::read("/tmp/flash_pendant.png").unwrap_or_default();
            match diff_marker_position(&avant, &pendant) {
                Some((fx, fy)) => {
                    println!("📍 Flash DÉTECTÉ par différence à ({}, {})", fx, fy);
                    println!(
                        "📐 ÉCART: cible ({:.0}, {:.0}) vs flash réel ({:.0}, {:.0}) → dx={:.0}, dy={:.0}",
                        x, y, fx, fy, fx as f64 - x, fy as f64 - y
                    );
                }
                None => println!("❌ Flash non détecté par différence (rien n'a changé ?)"),
            }
        }
        "--croix" => {
            // ✚ CROIX DE TIR — la trace visible du tir pour la calibration
            // (idée de mon humain) : après un clic, on affiche une croix rose à
            // l'endroit visé pendant `ms` ms et on capture PENDANT l'affichage.
            // Mes yeux (VLM) voient ensuite la croix SUR la grille → ils peuvent
            // identifier où le tir a atterri et on mesure le décalage visé→touché.
            // Usage: ecran-live --croix <x> <y> [--ms 1500] [--out /tmp/croix.png]
            use objc::{class, msg_send, runtime::Object, sel, sel_impl};
            let x: f64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(1920.0);
            let y: f64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1080.0);
            let ms: u64 = args
                .iter()
                .position(|a| a == "--ms")
                .and_then(|i| args.get(i + 1))
                .and_then(|s| s.parse().ok())
                .unwrap_or(1500);
            let out: String = args
                .iter()
                .position(|a| a == "--out")
                .and_then(|i| args.get(i + 1))
                .cloned()
                .unwrap_or_else(|| "/tmp/croix_pendant.png".to_string());
            let cap = Capteur::new()?;
            let screen_h = cap.display_w as f64 * cap.ratio;
            // Conversion Y : NSWindow = BAS-gauche. ⚠️ PAS de -48 ici : le -48
            // était calibré pour le curseur 72px (pointe en haut-gauche). La
            // croix 64px a son CENTRE à (x,y) — testé 11/08 : sans -48, la
            // croix tombe EXACTEMENT sur la cible (dy=0).
            let y_ns = screen_h - y;
            println!("✚ Croix de tir à ({:.0}, {:.0}) pendant {}ms → {}", x, y, ms, out);

            // 1. Capture AVANT (screencapture externe)
            let _ = std::process::Command::new("screencapture")
                .args(["-x", "/tmp/croix_avant.png"])
                .status();
            std::thread::sleep(Duration::from_millis(150));

            // 2. Créer la fenêtre croix
            let win = match create_cross_window() {
                Some(w) => w,
                None => {
                    println!("❌ Impossible de créer la fenêtre croix");
                    return Ok(());
                }
            };
            let run_loop: *mut Object = unsafe { msg_send![class!(NSRunLoop), currentRunLoop] };
            let rl_mode: *mut Object = unsafe { msg_send![class!(NSString),
                stringWithUTF8String: c"kCFRunLoopDefaultMode".as_ptr().cast::<u8>()
            ] };
            // Centre de la croix = (x, y_ns) → fenêtre 64x64 décalée de 32px
            let frame = NSRect { x: x - 32.0, y: y_ns - 32.0, w: 64.0, h: 64.0 };
            unsafe {
                let _: () = msg_send![win, setFrameOrigin: core_graphics::geometry::CGPoint::new(frame.x, frame.y)];
                let _: () = msg_send![win, orderFrontRegardless];
            }

            // 3. Pump 400ms pour peindre la croix
            let paint_deadline = Instant::now() + Duration::from_millis(400);
            while Instant::now() < paint_deadline {
                let until: *mut Object = unsafe { msg_send![class!(NSDate),
                    dateWithTimeIntervalSinceNow: 0.016
                ] };
                let _: bool = unsafe { msg_send![run_loop, runMode: rl_mode beforeDate: until] };
            }

            // 4. Capture PENDANT — la croix est visible
            let _ = std::process::Command::new("screencapture")
                .args(["-x", &out])
                .status();

            // 5. Fermer proprement
            unsafe {
                let _: () = msg_send![win, close];
                let _: () = msg_send![win, release];
            }

            // 6. Détecter le centre rose de la croix
            let pendant = std::fs::read(&out).unwrap_or_default();
            match detect_rose_center(&pendant) {
                Some((fx, fy)) => {
                    println!("📍 Croix DÉTECTÉE (rose) à ({}, {})", fx, fy);
                    println!(
                        "📐 ÉCART: cible ({:.0}, {:.0}) vs détectée ({:.0}, {:.0}) → dx={:.0}, dy={:.0}",
                        x, y, fx, fy, fx as f64 - x, fy as f64 - y
                    );
                }
                None => println!("❌ Croix non détectée par couleur rose"),
            }
        }
        "--clickbg" => {
            // CLIC ARRIÈRE-PLAN (recette cua-driver) : clic sur une fenêtre
            // SANS déplacer le curseur système. Usage :
            //   ecran-live --clickbg <pid> <window_id> <screen_x> <screen_y> <win_local_x> <win_local_y>
            // Le point window-local est en coordonnées QUARTZ (bas-gauche) :
            //   wx = x_écran - fenêtre_x
            //   wy = (fenêtre_y + fenêtre_h) - y_écran   (inversion Y !)
            let pid: i32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
            let wid: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
            let sx: f64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0.0);
            let sy: f64 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(0.0);
            let wx: f64 = args.get(5).and_then(|s| s.parse().ok()).unwrap_or(0.0);
            let wy: f64 = args.get(6).and_then(|s| s.parse().ok()).unwrap_or(0.0);
            if pid == 0 || wid == 0 {
                println!("Usage: ecran-live --clickbg <pid> <window_id> <screen_x> <screen_y> <win_local_x> <win_local_y>");
                return Ok(());
            }
            click_background(pid, wid, sx, sy, wx, wy)?;
        }
        // ecran-live --tire-visible <pid> <window_id> <screen_x> <screen_y> <win_local_x> <win_local_y>
        //   MÉTHODE BATAILLE NAVALE : mon curseur Sypherine GLISSE visiblement
        //   vers la cible → clic cua (curseur système INTACT) → CROIX ROSE
        //   d'impact 6 secondes. L'utilisateur ET mes yeux voient l'impact.
        "--tire-visible" => {
            let pid: i32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
            let wid: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
            let sx: f64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0.0);
            let sy: f64 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(0.0);
            let wx: f64 = args.get(5).and_then(|s| s.parse().ok()).unwrap_or(0.0);
            let wy: f64 = args.get(6).and_then(|s| s.parse().ok()).unwrap_or(0.0);
            if pid == 0 || wid == 0 {
                println!("Usage: ecran-live --tire-visible <pid> <window_id> <screen_x> <screen_y> <win_local_x> <win_local_y>");
                return Ok(());
            }
            tire_visible(pid, wid, sx, sy, wx, wy)?;
        }
        // ecran-live --activate <pid> <wid>
        //   Active la fenêtre cible (make_exact_window_key + activate_without_raise)
        //   SANS la déplacer — requis par Safari/WebKit pour accepter les clics
        //   quand le renderer WebContent a été recréé.
        "--activate" => {
            let pid: i32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
            let wid: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
            if pid == 0 || wid == 0 {
                println!("Usage: ecran-live --activate <pid> <wid>");
                return Ok(());
            }
            unsafe {
                let ok1 = skylight::make_exact_window_key(pid, wid);
                let ok2 = skylight::activate_without_raise(pid, wid);
                println!("⚡ Activation fenêtre {} (pid {}): make_key={} activate={}", wid, pid, ok1, ok2);
            }
        }
        "--winmove" => {
            // Déplace/redimensionne une fenêtre par window_id (SPI SkyLight).
            // Usage: ecran-live --winmove <wid> <x> <y> <w> <h>
            // Coordonnées QUARTZ (bas-gauche) : x,y = coin bas-gauche, w,h = taille.
            let wid: u32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
            let x: f64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0.0);
            let y: f64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0.0);
            let w: f64 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(0.0);
            let h: f64 = args.get(5).and_then(|s| s.parse().ok()).unwrap_or(0.0);
            if wid == 0 {
                println!("Usage: ecran-live --winmove <wid> <x> <y> <w> <h> (Quartz bas-gauche)");
                return Ok(());
            }
            let ok = unsafe { skylight::set_window_bounds(wid, x, y, w, h) };
            println!(
                "🪟 Fenêtre {} → ({:.0},{:.0} {:.0}x{:.0}) {}",
                wid,
                x,
                y,
                w,
                h,
                if ok { "✓" } else { "✗ SPI échoué" }
            );
        }
        // ── Analyse pixel native (Rust, remplace les scripts Python) ──────
        // ecran-live --analyse <image> [--cell 40] [--min 5]
        //   Détecte les clusters jaunes + blancs vifs + rose (boutons, compteurs)
        "--analyse" => {
            let chemin = args.get(1).cloned().unwrap_or_default();
            if chemin.is_empty() {
                println!("Usage: ecran-live --analyse <image.png> [--cell 40] [--min 5]");
                return Ok(());
            }
            let cell: u32 = args.iter().position(|a| a == "--cell")
                .and_then(|i| args.get(i + 1)).and_then(|s| s.parse().ok()).unwrap_or(40);
            let min: u32 = args.iter().position(|a| a == "--min")
                .and_then(|i| args.get(i + 1)).and_then(|s| s.parse().ok()).unwrap_or(5);
            let criteres = [
                analyse::Critere::jaune(),
                analyse::Critere::blanc_vif(),
                analyse::Critere::rose(),
            ];
            analyse::analyser(&chemin, &criteres, cell, min)?;
        }
        // ecran-live --diff <img1> <img2> [--zone x0 y0 x1 y1]
        //   VÉRITÉ TERRAIN : pixels différents entre deux captures (clic reçu ?)
        "--diff" => {
            let a = args.get(1).cloned().unwrap_or_default();
            let b = args.get(2).cloned().unwrap_or_default();
            if a.is_empty() || b.is_empty() {
                println!("Usage: ecran-live --diff <img1> <img2> [--zone x0 y0 x1 y1]");
                return Ok(());
            }
            let zone = args.iter().position(|s| s == "--zone").map(|i| {
                let x0: u32 = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(0);
                let y0: u32 = args.get(i + 2).and_then(|s| s.parse().ok()).unwrap_or(0);
                let x1: u32 = args.get(i + 3).and_then(|s| s.parse().ok()).unwrap_or(u32::MAX);
                let y1: u32 = args.get(i + 4).and_then(|s| s.parse().ok()).unwrap_or(u32::MAX);
                (x0, y0, x1, y1)
            });
            analyse::diff(&a, &b, zone)?;
        }
        // ecran-live --compteur <image> <x0> <y0> <x1> <y1>
        //   Grille ASCII d'une zone (lecture compteur par pixels, PAS par VLM)
        "--compteur" => {
            let chemin = args.get(1).cloned().unwrap_or_default();
            let x0: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
            let y0: u32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0);
            let x1: u32 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(0);
            let y1: u32 = args.get(5).and_then(|s| s.parse().ok()).unwrap_or(0);
            if chemin.is_empty() || x1 == 0 || y1 == 0 {
                println!("Usage: ecran-live --compteur <image> <x0> <y0> <x1> <y1>");
                return Ok(());
            }
            analyse::grille_ascii(&chemin, x0, y0, x1, y1)?;
        }
        // ecran-live --crop <image> <x0> <y0> <x1> <y1> <out.png>
        //   Crop vers fichier (préparation VLM : réduire à ≤800px, éviter 4K)
        "--crop" => {
            let chemin = args.get(1).cloned().unwrap_or_default();
            let x0: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
            let y0: u32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0);
            let x1: u32 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(0);
            let y1: u32 = args.get(5).and_then(|s| s.parse().ok()).unwrap_or(0);
            let out = args.get(6).cloned().unwrap_or_default();
            if chemin.is_empty() || out.is_empty() {
                println!("Usage: ecran-live --crop <image> <x0> <y0> <x1> <y1> <out.png>");
                return Ok(());
            }
            analyse::crop(&chemin, x0, y0, x1, y1, &out)?;
        }
        // ecran-live --centre <image> [--cell 10] [--min 3]
        //   Barycentre du bouton : groupe les blocs jaunes proches et calcule
        //   le centre pondéré — corrige le texte « CLIQUE » qui divise le massif.
        "--centre" => {
            let chemin = args.get(1).cloned().unwrap_or_default();
            if chemin.is_empty() {
                println!("Usage: ecran-live --centre <image.png> [--cell 10] [--min 3]");
                return Ok(());
            }
            let cell: u32 = args.iter().position(|a| a == "--cell")
                .and_then(|i| args.get(i + 1)).and_then(|s| s.parse().ok()).unwrap_or(10);
            let min: u32 = args.iter().position(|a| a == "--min")
                .and_then(|i| args.get(i + 1)).and_then(|s| s.parse().ok()).unwrap_or(3);
            analyse::barycentre(&chemin, cell, min)?;
        }
        // ecran-live --vlm <image> [question] [--max 512]
        //   Envoie une image au VLM mlxcel local (:8085) et affiche la réponse.
        //   Remplace vlm_voir.py — tout en Rust, un seul binaire.
        "--vlm" => {
            let chemin = args.get(1).cloned().unwrap_or_default();
            if chemin.is_empty() {
                println!("Usage: ecran-live --vlm <image.png> [question] [--max 512]");
                return Ok(());
            }
            let question = args.get(2).cloned().unwrap_or_else(|| "Décris l'image.".to_string());
            // --max : taille max du côté le plus long (défaut 512).
            // Plus petit = préfill VLM plus rapide (512 px ≈ 1.2s vs 1024 ≈ 1.8s)
            // SANS perte de fiabilité (testé 3/3 identique à 384/512/1024).
            let max_side: u32 = args.iter().position(|a| a == "--max")
                .and_then(|i| args.get(i + 1)).and_then(|s| s.parse().ok()).unwrap_or(512);
            let png = analyse::reduire_max(&chemin, max_side)?;
            match analyze_image(&png, &question) {
                Ok(reponse) => println!("{}", reponse),
                Err(e) => eprintln!("ERREUR VLM: {}", e),
            }
        }
        // ecran-live --conv <image> <question>   (1er tour : image + question)
        // ecran-live --conv <question>            (tours suivants : texte seul, cache KV)
        // ecran-live --conv --reset               (efface la conversation)
        //   Conversation VLM persistée : la 1ère question embarque l'image,
        //   les suivantes sont du texte seul → le serveur garde l'image en
        //   KV cache (prefix-cache VLM) → ~0.6s/tour au lieu de ~2.2s.
        "--conv" => {
            if args.get(1).map(|s| s.as_str()) == Some("--reset") {
                let _ = std::fs::remove_file("/tmp/vlm_conv.json");
                println!("Conversation VLM effacée");
                return Ok(());
            }
            let premier = args.get(1).cloned().unwrap_or_default();
            let question = args.get(2).cloned().unwrap_or_default();
            // Premier argument = chemin d'image existant ? → 1er tour avec image.
            // Sinon = la question elle-même (tour suivant, texte seul).
            let (image, q) = if std::path::Path::new(&premier).exists() {
                let q = if question.is_empty() { "Décris l'image.".to_string() } else { question };
                (Some(premier), q)
            } else {
                (None, premier)
            };
            let reponse = match image {
                Some(chemin) => {
                    let png = analyse::reduire_max(&chemin, 512)?;
                    vlm_conv_tour(Some(&png), &q)?
                }
                None => {
                    if q.is_empty() {
                        println!("Usage: ecran-live --conv <image> <question> | --conv <question> | --conv --reset");
                        return Ok(());
                    }
                    vlm_conv_tour(None, &q)?
                }
            };
            println!("{}", reponse);
        }
        // ecran-live --vlmzone <image> <x0,y0,x1,y1> [question]
        //   Crop + zoom 3x → VLM (lecture fine d'une zone).
        //   Remplace voir_zone.py
        "--vlmzone" => {
            let chemin = args.get(1).cloned().unwrap_or_default();
            let zone: Vec<u32> = args.get(2)
                .map(|s| s.split(',').filter_map(|v| v.parse().ok()).collect())
                .unwrap_or_default();
            if chemin.is_empty() || zone.len() != 4 {
                println!("Usage: ecran-live --vlmzone <image> <x0,y0,x1,y1> [question]");
                return Ok(());
            }
            let question = args.get(3).cloned().unwrap_or_else(|| "Que vois-tu ?".to_string());
            let img = analyse::charger(&chemin)?;
            let png = analyse::crop_zoom_png(&img, zone[0], zone[1], zone[2], zone[3], 3)?;
            match analyze_image(&png, &question) {
                Ok(reponse) => println!("{}", reponse),
                Err(e) => eprintln!("ERREUR VLM: {}", e),
            }
        }
        // ecran-live --grille <image> [grille_N] [--crop x0,y0,w,h]
        //   CALIBRATION « TOUCHÉ-COULÉ » : grille NxN virtuelle → le VLM prédit
        //   la case (ligne,colonne) du bouton → centre de case = cible de tir.
        //   Remplace calib_grille.py (mode zone). Le tir reste --clickbg/--croix.
        "--grille" => {
            let chemin = args.get(1).cloned().unwrap_or_default();
            if chemin.is_empty() {
                println!("Usage: ecran-live --grille <image> [grille_N] [--crop x0,y0,w,h]");
                return Ok(());
            }
            let grille: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(8);
            let crop: Option<(u32, u32, u32, u32)> = args.iter().position(|s| s == "--crop").map(|i| {
                let v: Vec<u32> = args.get(i + 1)
                    .map(|s| s.split(',').filter_map(|x| x.parse().ok()).collect())
                    .unwrap_or_default();
                if v.len() == 4 { Some((v[0], v[1], v[2], v[3])) } else { None }
            }).flatten();

            // Image travaillée : crop fenêtre (évite que le HUD perturbe le VLM)
            // sinon image entière réduite à 512px (rapide, fiable — testé).
            let (png, off_x, off_y, cw, ch) = match crop {
                Some((x0, y0, w, h)) => {
                    let png = analyse::crop_reduit_png(&chemin, x0, y0, w, h)?;
                    (png, x0 as f64, y0 as f64, w as f64, h as f64)
                }
                None => {
                    let png = analyse::reduire_max(&chemin, 512)?;
                    (png, 0.0, 0.0, 3840.0, 2160.0)
                }
            };

            let question = format!(
                "Une grille virtuelle de {g}x{g} cases est appliquée sur cette image. \
                 La case (ligne 0, colonne 0) est en haut à gauche. \
                 Sur la page web visible, il y a un bouton JAUNE. \
                 Dans quelle case est le CENTRE de ce bouton jaune ? \
                 Réponds exactement au format: ZONE: ligne,colonne (deux nombres entiers entre 0 et {g}).",
                g = grille
            );
            let ans = analyze_image(&png, &question).unwrap_or_else(|e| format!("ERREUR: {}", e));
            println!("VLM: {}", ans.trim().chars().take(120).collect::<String>());

            // Parser "ZONE: l,c" (ou 2 premiers nombres)
            let mut zone_vlm: Option<(i64, i64)> = None;
            for line in ans.lines() {
                let l = line.trim();
                if l.to_uppercase().starts_with("ZONE:") {
                    let vals: Vec<&str> = l[5..].trim().split(',').collect();
                    if vals.len() == 2 {
                        if let (Ok(a), Ok(b)) = (vals[0].trim().parse::<i64>(), vals[1].trim().parse::<i64>()) {
                            zone_vlm = Some((a, b));
                            break;
                        }
                    }
                }
            }
            if zone_vlm.is_none() {
                let nums: Vec<i64> = ans.split_whitespace()
                    .filter_map(|w| w.trim_matches(|c: char| !c.is_ascii_digit() && c != '-').parse().ok())
                    .collect();
                if nums.len() >= 2 {
                    zone_vlm = Some((nums[0], nums[1]));
                }
            }

            match zone_vlm {
                Some((l, c)) => {
                    let l = l.clamp(0, grille as i64 - 1) as f64;
                    let c = c.clamp(0, grille as i64 - 1) as f64;
                    let cell_w = cw / grille as f64;
                    let cell_h = ch / grille as f64;
                    let sx = off_x + c * cell_w + cell_w / 2.0;
                    let sy = off_y + l * cell_h + cell_h / 2.0;
                    println!("📍 Zone prédite (l={}, c={}) → écran ({:.0},{:.0})", l, c, sx, sy);
                }
                None => println!("❌ Zone non détectée"),
            }
        }
        // ecran-live --palais <sous-commande> ...
        //   PALAIS DE MÉMOIRE SPATIAL (méthode des loci) :
        //   ranger <img> <l> <c> [nom]  → range une capture dans la pièce (l,c)
        //   chercher <l> <c>            → liste les captures de la pièce (O(1))
        //   visiter                     → vue d'ensemble du palais
        //   tir <l> <c> <vx> <vy> <tx> <ty>  → apprend le biais de la pièce
        //   purge <l> <c> [--garder N]  → garde les N dernières captures
        //   diff <l> <c> <i> <j>        → diff pixel entre 2 captures de la pièce
        "--palais" => {
            let sous = args.get(1).cloned().unwrap_or_default();
            match sous.as_str() {
                "ranger" => {
                    let img = args.get(2).cloned().unwrap_or_default();
                    let l: u32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0);
                    let c: u32 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(0);
                    let nom = args.get(5).cloned().unwrap_or_else(|| "capture".to_string());
                    if img.is_empty() {
                        println!("Usage: ecran-live --palais ranger <img> <l> <c> [nom]");
                        return Ok(());
                    }
                    palais::ranger(&img, l, c, &nom)?;
                }
                "chercher" => {
                    let l: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
                    let c: u32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0);
                    palais::chercher(l, c)?;
                }
                "retrouver" => {
                    let nom = args.get(2).cloned().unwrap_or_default();
                    if nom.is_empty() {
                        println!("Usage: ecran-live --palais retrouver <nom>");
                        return Ok(());
                    }
                    palais::retrouver(&nom)?;
                }
                "travail" => {
                    let index = palais::charger_index()?;
                    palais::afficher_travail(&index)?;
                }
                // Sommeil du palais (consolidation ADN) : transforme les
                // captures PNG (~1 Mo) en empreintes perceptives (2 Ko) +
                // réponses VLM — le palais devient ~500× plus léger.
                "sommeil" => {
                    palais::sommeil()?;
                }
                "visiter" => {
                    palais::visiter()?;
                }
                "tir" => {
                    let l: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
                    let c: u32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0);
                    let vx: f64 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(0.0);
                    let vy: f64 = args.get(5).and_then(|s| s.parse().ok()).unwrap_or(0.0);
                    let tx: f64 = args.get(6).and_then(|s| s.parse().ok()).unwrap_or(0.0);
                    let ty: f64 = args.get(7).and_then(|s| s.parse().ok()).unwrap_or(0.0);
                    palais::tir(l, c, vx, vy, tx, ty)?;
                }
                "purge" => {
                    let l: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
                    let c: u32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0);
                    let garder: usize = args.iter().position(|a| a == "--garder")
                        .and_then(|i| args.get(i + 1)).and_then(|s| s.parse().ok()).unwrap_or(3);
                    palais::purge(l, c, garder)?;
                }
                "diff" => {
                    let l: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
                    let c: u32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0);
                    let i: usize = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(0);
                    let j: usize = args.get(5).and_then(|s| s.parse().ok()).unwrap_or(1);
                    let (a, b) = palais::chemin_capture(l, c, i, j)?;
                    analyse::diff(&a, &b, None)?;
                }
                _ => {
                    println!(
                        "🏛️  PALAIS DE MÉMOIRE\n\
                         Usage:\n\
                         \x20 ecran-live --palais ranger <img> <l> <c> [nom]   — range une capture\n\
                         \x20 ecran-live --palais chercher <l> <c>              — liste la pièce (O(1))\n\
                         \x20 ecran-live --palais retrouver <nom>            — activation diffuse (LOCI)\n\
                         \x20 ecran-live --palais travail                    — mémoire de travail 7±2\n\
                         \x20 ecran-live --palais visiter                       — vue d'ensemble\n\
                         \x20 ecran-live --palais tir <l> <c> <vx> <vy> <tx> <ty> — biais par zone\n\
                         \x20 ecran-live --palais purge <l> <c> [--garder N]    — purge (défaut garder 3)\n\
                         \x20 ecran-live --palais diff <l> <c> <i> <j>          — diff 2 captures"
                    );
                }
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

// ═══════════════════════════════════════════════════════════════════════════
// WATCHER MÉMOIRE CONTINU — la RAM est critique, elle peut bloquer TOUTES
// les autres tâches (vision, clic, VLM). Ce mode surveille la RAM du process
// ET la RAM système, et nettoie /tmp des vieilles captures quand le seuil est
// dépassé, pour que le buffer ne soit JAMAIS saturé.
// Usage: ecran-live --watch-ram [--seuil 80] [--int 5]
//   --seuil : % de RAM système au-delà duquel on nettoie (défaut 80)
//   --int   : intervalle en secondes entre deux mesures (défaut 5)
// ═══════════════════════════════════════════════════════════════════════════

/// Struct miroir de vm_statistics (macOS, HOST_VM_INFO flavor=2).
/// Version 32-bit : les 15 premiers champs sont remplis par macOS 26 et
/// contiennent tout ce qu'on lit (free/active/inactive/wire/speculative).
/// La version 64-bit (vm_statistics64) est DÉCALÉE sur macOS 26 (testée :
/// free_count lu correctement mais u64 suivants faux) → on reste en 32-bit.
#[repr(C)]
struct VmStatistics {
    free_count: u32,
    active_count: u32,
    inactive_count: u32,
    wire_count: u32,
    zero_fill_count: u32,
    reactivations: u32,
    pageins: u32,
    pageouts: u32,
    faults: u32,
    cow_faults: u32,
    lookups: u32,
    hits: u32,
    purges: u32,
    purgeable_count: u32,
    speculative_count: u32,
    decompressions: u32,
    compressions: u32,
    swapins: u32,
    swapouts: u32,
    compressor_page_count: u32,
    throttled_count: u32,
    external_page_count: u32,
    internal_page_count: u32,
    total_uncompressed_pages_in_compressor: u32,
}

/// Pourcentage de RAM système LIBRE (0.0 si la lecture échoue).
/// Compte free + speculative + inactive = mémoire réellement récupérable.
fn system_ram_free_percent() -> f64 {
    unsafe {
        extern "C" {
            fn mach_host_self() -> libc::c_uint;
            fn host_statistics(
                host: libc::c_uint,
                flavor: libc::c_int,
                out: *mut libc::c_int,
                out_count: *mut libc::c_uint,
            ) -> libc::c_int;
        }
        let host = mach_host_self();
        let mut info: VmStatistics = std::mem::zeroed();
        let mut count: libc::c_uint =
            (std::mem::size_of::<VmStatistics>() / std::mem::size_of::<libc::c_int>()) as libc::c_uint;
        // HOST_VM_INFO = 2 (32-bit, fiable sur macOS 26)
        let kr = host_statistics(
            host,
            2,
            &mut info as *mut _ as *mut libc::c_int,
            &mut count,
        );
        if kr == 0 {
            let free = info.free_count as u64
                + info.speculative_count as u64
                + info.inactive_count as u64;
            let total = free + info.active_count as u64
                + info.wire_count as u64
                + info.compressor_page_count as u64;
            if total > 0 {
                return free as f64 * 100.0 / total as f64;
            }
        }
        0.0
    }
}

/// Supprime les vieilles captures (/tmp/*.png|jpg|jpeg|bmp) en gardant les
/// `keep` plus récentes. Retourne le nombre de fichiers supprimés.
fn cleanup_tmp_captures(keep: usize) -> usize {
    let mut removed = 0;
    if let Ok(entries) = std::fs::read_dir("/tmp") {
        let mut files: Vec<(std::time::SystemTime, std::path::PathBuf, u64)> = Vec::new();
        for e in entries.flatten() {
            let p = e.path();
            let is_img = p
                .extension()
                .map(|x| {
                    let s = x.to_string_lossy().to_lowercase();
                    s == "png" || s == "jpg" || s == "jpeg" || s == "bmp"
                })
                .unwrap_or(false);
            if !is_img {
                continue;
            }
            if let Ok(md) = std::fs::metadata(&p) {
                if let Ok(mtime) = md.modified() {
                    files.push((mtime, p, md.len()));
                }
            }
        }
        files.sort_by(|a, b| b.0.cmp(&a.0)); // plus récent d'abord
        for (_, p, _) in files.into_iter().skip(keep) {
            if std::fs::remove_file(&p).is_ok() {
                removed += 1;
            }
        }
    }
    removed
}

/// Garde-fou CONTINU : garde les `keep` plus récentes ET supprime les plus
/// vieilles tant que le total dépasse `max_mb` Mo. Pour les captures 4K qui
/// font 8-30 Mo chacune, ça borne /tmp (leçon 11/08 : 2,8 Go accumulés).
fn cleanup_tmp_captures_until(keep: usize, max_mb: u64) -> usize {
    let mut removed = 0;
    if let Ok(entries) = std::fs::read_dir("/tmp") {
        let mut files: Vec<(std::time::SystemTime, std::path::PathBuf, u64)> = Vec::new();
        for e in entries.flatten() {
            let p = e.path();
            let is_img = p
                .extension()
                .map(|x| {
                    let s = x.to_string_lossy().to_lowercase();
                    s == "png" || s == "jpg" || s == "jpeg" || s == "bmp"
                })
                .unwrap_or(false);
            if !is_img {
                continue;
            }
            if let Ok(md) = std::fs::metadata(&p) {
                if let Ok(mtime) = md.modified() {
                    files.push((mtime, p, md.len()));
                }
            }
        }
        files.sort_by(|a, b| b.0.cmp(&a.0)); // plus récent d'abord

        let mut total: u64 = files.iter().map(|f| f.2).sum();
        let mut index = keep.min(files.len());
        while total > max_mb * 1048576 && index < files.len() {
            let (_, p, size) = &files[index];
            if std::fs::remove_file(p).is_ok() {
                removed += 1;
                total -= size;
            }
            index += 1;
        }
    }
    removed
}

/// Détecte la position du flash Sypherine (logo rose/violet) dans une capture.
/// Mes YEUX = analyse pixel : le curseur est un dégradé rose/violet distinct
/// (r élevé, b élevé, g faible) → clusters par cellules 40px (pattern BMP),
/// on garde le cluster le plus dense et on retourne son centre de masse.
/// Retourne (x, y) en pixels de la capture.
fn detect_marker_position(png_bytes: &[u8]) -> Option<(u32, u32)> {
    use std::collections::HashMap;
    let img = image::load_from_memory(png_bytes).ok()?;
    let (w, h) = img.dimensions();
    let rgb = img.to_rgb8();

    // Pixels rose/violet du logo (dégradé S doré + feuilles → rose/violet)
    let mut chauds: Vec<(u32, u32)> = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let p = rgb.get_pixel(x, y);
            let (r, g, b) = (p[0] as i32, p[1] as i32, p[2] as i32);
            // Rose/violet : r et b élevés, g nettement plus bas
            if r > 140 && b > 100 && r > g + 40 && b > g + 20 {
                chauds.push((x, y));
            }
        }
    }
    if chauds.len() < 20 {
        return None;
    }

    // Clusters par cellules 40px — le flash est le cluster le plus dense
    let mut cells: HashMap<(u32, u32), Vec<(u32, u32)>> = HashMap::new();
    for (x, y) in &chauds {
        cells.entry((x / 40, y / 40)).or_default().push((*x, *y));
    }
    let best = cells.iter().max_by_key(|(_, v)| v.len())?;
    let pts = best.1;
    if pts.len() < 15 {
        return None;
    }
    let cx = pts.iter().map(|p| p.0 as u64).sum::<u64>() / pts.len() as u64;
    let cy = pts.iter().map(|p| p.1 as u64).sum::<u64>() / pts.len() as u64;
    Some((cx as u32, cy as u32))
}

/// Détecte la position du flash Sypherine par DIFFÉRENCE entre deux captures
/// (avant/pendant). Le flash est le SEUL changement entre les deux images —
/// les icônes fixes du bureau disparaissent par soustraction. Le logo est
/// BLANC/GRIS (~224,224,224), PAS rose/violet (vérifié 11/08 sur le PNG).
/// Retourne (x, y) = centre de masse de la zone de différence, en pixels.
fn diff_marker_position(avant: &[u8], pendant: &[u8]) -> Option<(u32, u32)> {
    let img_a = image::load_from_memory(avant).ok()?;
    let img_p = image::load_from_memory(pendant).ok()?;
    let (w, h) = img_a.dimensions();
    if img_p.dimensions() != (w, h) {
        return None;
    }
    let a = img_a.to_rgb8();
    let p = img_p.to_rgb8();

    // Pixels qui ont CHANGÉ (différence > seuil) ET qui sont blanc/gris
    // (le flash) — double filtre : changement + luminosité.
    let mut chauds: Vec<(u32, u32)> = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let pa = a.get_pixel(x, y);
            let pp = p.get_pixel(x, y);
            let diff = (pa[0] as i32 - pp[0] as i32).abs()
                + (pa[1] as i32 - pp[1] as i32).abs()
                + (pa[2] as i32 - pp[2] as i32).abs();
            if diff < 80 {
                continue; // pas de changement
            }
            // Le logo Sypherine : blanc/gris clair, luminance élevée
            let lum = (pp[0] as i32 + pp[1] as i32 + pp[2] as i32) / 3;
            if lum > 150 {
                chauds.push((x, y));
            }
        }
    }
    if chauds.len() < 15 {
        return None;
    }
    // ⚠️ PAS le centre de masse global : le curseur système, des animations
    // ou d'autres éléments blancs qui bougent polluent le calcul (vécu 11/08 :
    // flash à (1918,1082) mais centre global à (2201,1018)). Le flash est un
    // CLUSTER COMPACT 48x48 → clustering par cellules 40px, garder la cellule
    // la plus dense, centre de masse DE CE CLUSTER uniquement (pattern BMP).
    use std::collections::HashMap;
    let mut cells: HashMap<(u32, u32), Vec<(u32, u32)>> = HashMap::new();
    for (x, y) in &chauds {
        cells.entry((x / 40, y / 40)).or_default().push((*x, *y));
    }
    let best = cells.iter().max_by_key(|(_, v)| v.len())?;
    let pts = best.1;
    if pts.len() < 10 {
        return None;
    }
    let cx = pts.iter().map(|p| p.0 as u64).sum::<u64>() / pts.len() as u64;
    let cy = pts.iter().map(|p| p.1 as u64).sum::<u64>() / pts.len() as u64;
    Some((cx as u32, cy as u32))
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

/// Cherche TOUTES les correspondances OCR du texte cible et retourne la
/// MEILLEURE (distance Levenshtein minimale). Retourne (texte, x, y, score).
/// Le score est la distance Levenshtein minimale trouvée (0 = exact).
/// Contrairement à l'ancienne version qui retournait le premier match,
/// on trie par score pour éviter les faux positifs (ex: « Repository »
/// qui matchait « New repositon » dans la sidebar avant le champ visé).
fn find_text_ocr_best(png_bytes: &[u8], target: &str) -> Option<(String, f64, f64, usize)> {
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
    let mut best: Option<(String, f64, f64, usize)> = None;
    for p in paras {
        let ls = p["lines"].as_array()?;
        for l in ls {
            let text = l["text"].as_str().unwrap_or("").to_string();
            if let Some(score) = fuzzy_score(&text, target) {
                // Garder le meilleur (distance minimale)
                let replace = match &best {
                    None => true,
                    Some((_, _, _, best_score)) => score < *best_score,
                };
                if replace {
                    // Centre du bounding box (vertices: coin sup-gauche, sup-droit,
                    // inf-droit, inf-gauche) — plus fiable que le coin seul.
                    let mut xs = Vec::new();
                    let mut ys = Vec::new();
                    if let Some(vs) = l["vertices"].as_array() {
                        for vtx in vs.iter() {
                            if let (Some(x), Some(y)) = (
                                vtx[0].as_f64(),
                                vtx[1].as_f64(),
                            ) {
                                xs.push(x);
                                ys.push(y);
                            }
                        }
                    }
                    if !xs.is_empty() && !ys.is_empty() {
                        let cx = xs.iter().sum::<f64>() / xs.len() as f64;
                        let cy = ys.iter().sum::<f64>() / ys.len() as f64;
                        best = Some((text, cx, cy, score));
                    }
                }
            }
        }
    }
    best
}

/// Retourne la distance Levenshtein minimale entre la cible et un mot du
/// texte (None si aucune correspondance). La tolérance est RELATIVE :
/// 1 faute pour les mots courts (3-6), 2 pour les mots longs (7+).
/// Beaucoup plus fiable que la distance fixe ≤ 2 qui créait des faux positifs.
fn fuzzy_score(text: &str, target: &str) -> Option<usize> {
    let t = target.to_lowercase();
    let tl = t.len();
    if t.is_empty() || tl < 3 {
        // Cible trop courte : recherche par simple inclusion
        return if text.to_lowercase().contains(&t) {
            Some(0)
        } else {
            None
        };
    }
    let max_dist = if tl >= 7 { 2 } else { 1 };
    let mut best: Option<usize> = None;
    for w in text
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
    {
        if w.is_empty() {
            continue;
        }
        let wl = w.len();
        // La longueur doit être proche de la cible (±2), sinon ce n'est pas le mot
        if wl < tl.saturating_sub(2) || wl > tl + 2 {
            continue;
        }
        let d = levenshtein(w, &t);
        if d <= max_dist {
            let better = match best {
                None => true,
                Some(b) => d < b,
            };
            if better {
                best = Some(d);
            }
        }
    }
    best
}

/// Compat : ancien appel booléen (utilisé par --locate et --click).
fn fuzzy_match(text: &str, target: &str) -> bool {
    fuzzy_score(text, target).is_some()
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

/// Analyse une image PNG (bytes) via le serveur mlxcel local (port 8085,
/// Rust + MLX C++ natif — ~4-10x plus rapide que mistralrs).
/// Retourne le texte généré.
fn analyze_image(png_bytes: &[u8], question: &str) -> Result<String, String> {
    analyze_image_mt(png_bytes, question, 24)
}

/// Analyse avec CONTEXTE GLOBAL (modèle mental) : préfixe la question avec
/// le résumé de l'écran établi par --contexte. Le VLM sait où il est →
/// moins d'hallucinations, qualité fine et fiable (testé 13/08).
fn analyze_image_ctx(png_bytes: &[u8], question: &str, contexte: &str) -> Result<String, String> {
    if contexte.is_empty() {
        return analyze_image_mt(png_bytes, question, 24);
    }
    let q = format!("Contexte global de l'écran : {}. Question : {}", contexte, question);
    analyze_image_mt(png_bytes, &q, 24)
}

/// Version avec max_tokens contrôlable. Le VLM génère à ~30 tok/s : une
/// réponse de 40 tokens coûte ~1.3s de génération (vs 100 tokens = ~3.3s).
/// Les prompts qui exigent une réponse courte (oui/non, un nombre, un mot)
/// tombent sous la seconde. Les descriptions détaillées passent par
/// analyze_image_mt(png, q, 300).
fn analyze_image_mt(png_bytes: &[u8], question: &str, max_tokens: u32) -> Result<String, String> {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(png_bytes);
    let payload = serde_json::json!({
        "model": "LFM2.5-VL-3B-MLX-4bit-vq",
        "messages": [{
            "role": "user",
            "content": [
                {"type": "text", "text": question},
                {"type": "image_url", "image_url": {"url": format!("data:image/png;base64,{}", b64)}}
            ]
        }],
        "max_tokens": max_tokens,
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

/// Envoie des messages arbitraires (conversation complète) au VLM mlxcel.
/// Le premier message contient l'image ; les suivants sont du texte SEUL —
/// le serveur garde l'image en KV cache (prefix-cache VLM) et chaque tour
/// suivant coûte ~0.6s au lieu de ~2.2s (cached≈277/297, mesuré).
fn analyze_conversation(messages: serde_json::Value) -> Result<String, String> {
    let payload = serde_json::json!({
        "model": "LFM2.5-VL-3B-MLX-4bit-vq",
        "messages": messages,
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

/// Conversation VLM persistée dans /tmp/vlm_conv.json : la première question
/// embarque l'image, les suivantes sont du texte seul (cache KV serveur).
/// Retourne la réponse du tour courant.
fn vlm_conv_tour(image_png: Option<&[u8]>, question: &str) -> Result<String, String> {
    use base64::Engine;
    let chemin = "/tmp/vlm_conv.json";
    // Charger l'état existant (messages cumulés)
    let mut messages: serde_json::Value = serde_json::Value::Array(vec![]);
    if let Ok(contenu) = std::fs::read_to_string(chemin) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&contenu) {
            if v.is_array() {
                messages = v;
            }
        }
    }

    // Nouveau tour : message utilisateur
    let nouveau = match image_png {
        Some(png) => {
            // Premier tour : image + question
            let b64 = base64::engine::general_purpose::STANDARD.encode(png);
            serde_json::json!({
                "role": "user",
                "content": [
                    {"type": "text", "text": question},
                    {"type": "image_url", "image_url": {"url": format!("data:image/png;base64,{}", b64)}}
                ]
            })
        }
        None => {
            // Tours suivants : texte seul (le serveur garde l'image en cache)
            serde_json::json!({
                "role": "user",
                "content": question
            })
        }
    };
    if let Some(arr) = messages.as_array_mut() {
        arr.push(nouveau);
    }

    // Envoyer et mémoriser la réponse
    let reponse = analyze_conversation(messages.clone())?;
    if let Some(arr) = messages.as_array_mut() {
        arr.push(serde_json::json!({
            "role": "assistant",
            "content": reponse
        }));
    }
    let _ = std::fs::write(chemin, serde_json::to_string_pretty(&messages).unwrap_or_default());
    Ok(reponse)
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

/// Empreinte perceptive : réduit l'image à 64px de large, garde un échantillon
/// de pixels (luminance) comme « signature » de la frame. Deux frames
/// identiques → empreintes identiques → pas besoin de ré-analyser (économie
/// CPU + tokens VLM dans le mode --stream).
fn fingerprint(png_bytes: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let img = image::load_from_memory(png_bytes)?.to_luma8();
    let (iw, ih) = img.dimensions();
    // Cible : 64x36 (assez fin pour détecter les petits changements)
    let tw = 64u32;
    let th = ((tw as f64) * (ih as f64) / (iw as f64)).round().max(1.0) as u32;
    let small = image::imageops::resize(&img, tw, th, image::imageops::FilterType::Triangle);
    Ok(small.as_raw().to_vec())
}

/// Taux de pixels différents entre deux empreintes (0.0 = identiques,
/// 1.0 = totalement différents). Utilisé par --stream pour ignorer les
/// micro-changements (curseur, clignotements) — le fix du bug de freeze
/// qui saturait le VLM en analysant chaque frame.
fn fp_diff_ratio(a: &[u8], b: &[u8]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 1.0;
    }
    let mut diff = 0u64;
    for (x, y) in a.iter().zip(b.iter()) {
        if (*x as i16 - *y as i16).abs() > 12 {
            diff += 1;
        }
    }
    diff as f64 / a.len() as f64
}

/// RAM du processus courant en Mo (lecture /proc-like sur macOS : task_info).
fn process_mem_mb() -> u64 {
    unsafe {
        extern "C" {
            fn mach_task_self() -> libc::c_uint;
            fn task_info(
                task: libc::c_uint,
                flavor: libc::c_int,
                out: *mut libc::c_int,
                out_size: *mut libc::c_uint,
            ) -> libc::c_int;
        }
        // MACH_TASK_BASIC_INFO = 20 ; struct mach_task_basic_info { ... resid_size ... }
        let mut info = [0i32; 16];
        let mut count: libc::c_uint = 16;
        let kr = task_info(mach_task_self(), 20, info.as_mut_ptr(), &mut count);
        if kr == 0 {
            // resid_size est le 2e champ (offset 1) en pages → octets
            let pages = info[1] as u64;
            // page size = 16384 sur Apple Silicon
            (pages * 16384) / 1048576
        } else {
            0
        }
    }
}

/// Fovéation biomimétique (13/08) : assemble TOUTES les zones saillantes
/// dans UNE mosaïque carrée (chaque zone réduite à 128px) → 1 seul appel VLM.
/// Mesuré : 4 zones = 1.6s (83 tokens) vs 4 appels séparés = 8.8s (5.5x).
fn build_fovea_mosaic(png_bytes: &[u8], zones: &[SalientZone]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut img = image::load_from_memory(png_bytes)?;
    let (iw, ih) = img.dimensions();
    let n = zones.len();
    if n == 0 {
        return Err("aucune zone pour la fovéa".into());
    }
    // Grille carrée : 1→1x1, 2→2x1, 3-4→2x2, 5-9→3x3
    let cols = (n as f64).sqrt().ceil() as u32;
    let rows = (n as u32 + cols - 1) / cols;
    let cell = 128u32;
    let mut mosaic = image::RgbImage::new(cols * cell, rows * cell);

    for (i, z) in zones.iter().enumerate() {
        let mw = (z.w as f64 * 0.1) as u32;
        let mh = (z.h as f64 * 0.1) as u32;
        let x = z.x.saturating_sub(mw);
        let y = z.y.saturating_sub(mh);
        let w = (z.w + 2 * mw).min(iw.saturating_sub(x));
        let hh = (z.h + 2 * mh).min(ih.saturating_sub(y));
        let crop = img.crop(x.max(0), y.max(0), w.max(1), hh.max(1));
        let rgb = crop.to_rgb8();
        // Réduction au format carré cell×cell (Triangle = rapide, finesse OK)
        let resized = image::imageops::resize(
            &rgb, cell, cell,
            image::imageops::FilterType::Triangle,
        );
        let cx = (i as u32 % cols) * cell;
        let cy = (i as u32 / cols) * cell;
        image::imageops::overlay(&mut mosaic, &resized, cx as i64, cy as i64);
    }

    let mut buf = Vec::new();
    let (mw, mh) = mosaic.dimensions();
    let raw = mosaic.into_raw();
    image::codecs::png::PngEncoder::new(&mut std::io::Cursor::new(&mut buf)).write_image(
        &raw,
        mw,
        mh,
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
        // ══ RÉDUCTION DU CROP AVANT VLM (perf 13/08) ══
        // Le vision tower coûte proportionnellement aux patches d'image :
        // crop 240x216 = ~9s (130 tokens), réduit 256px = 2.7s (86 tokens),
        // réduit 128px = sous la seconde (~90 tokens max, mesuré 0.89s).
        // On downscale chaque crop à max 128px de large AVANT l'envoi VLM.
        let reduced: image::DynamicImage = {
            let (cw, ch) = crop.dimensions();
            if cw > 128 {
                let scale = 128.0 / cw as f64;
                let rgb = crop.to_rgb8();
                image::DynamicImage::ImageRgb8(image::imageops::resize(
                    &rgb,
                    128,
                    ((ch as f64) * scale).max(1.0) as u32,
                    image::imageops::FilterType::Triangle,
                ))
            } else {
                crop
            }
        };
        image::codecs::png::PngEncoder::new(&mut std::io::Cursor::new(&mut buf)).write_image(
            reduced.as_bytes(),
            reduced.width(),
            reduced.height(),
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

fn write_mouse(home: &str, pos: (f64, f64)) {
    let _ = std::fs::write(
        format!("{}/souris.json", home),
        format!("{{\"x\":{:.0},\"y\":{:.0}}}\n", pos.0, pos.1),
    );
}

/// Déplace la souris puis clique (gauche/droite) à (x, y) via CGEvent.
/// Nécessite l'accessibilité (System Settings → Confidentialité → Accessibilité).
/// Recherche dans l'AX tree macOS un élément dont le label (title/description)
/// fuzzy-matche la cible. Retourne le CENTRE de l'élément en coordonnées écran.
/// C'est la voie fiable pour les champs VIDES (un champ de saisie n'a pas de
/// texte OCR !) — leçon computer-use/cua-driver : lire l'AX tree, pas l'OCR.
/// Nécessite la permission Accessibilité pour ce binaire.
fn ax_find_element(app: &str, target: &str) -> Option<(String, f64, f64)> {
    use accessibility::{AXAttribute, AXUIElement};
    use core_foundation::array::CFArray;
    use core_foundation::base::CFType;
    use core_foundation::string::CFString;

    // Résoudre le nom d'app → PID (pattern cua : on travaille par PID, pas bundle)
    let app_lower = app.to_lowercase();
    let pid = match app_lower.as_str() {
        "safari" => pgrep_first("Safari"),
        "hermes" | "hermes one" => pgrep_first("Hermes One"),
        "notes" => pgrep_first("Notes"),
        "finder" => pgrep_first("Finder"),
        _ => pgrep_first(app),
    }?;
    let app_elem = AXUIElement::application(pid);

    // Extraire (x, y) ou (w, h) d'un AXValue (CGPoint/CGSize) via FFI.
    fn ax_point_from_value(val: &CFType) -> Option<(f64, f64)> {
        use core_foundation::base::TCFType;
        unsafe {
            let mut p: core_graphics::geometry::CGPoint = std::mem::zeroed();
            let ok = accessibility_sys::AXValueGetValue(
                val.as_concrete_TypeRef() as *mut _,
                accessibility_sys::kAXValueTypeCGPoint,
                &mut p as *mut _ as *mut std::ffi::c_void,
            );
            if !ok {
                return None;
            }
            Some((p.x as f64, p.y as f64))
        }
    }
    fn ax_size_from_value(val: &CFType) -> Option<(f64, f64)> {
        use core_foundation::base::TCFType;
        unsafe {
            let mut s: core_graphics::geometry::CGSize = std::mem::zeroed();
            let ok = accessibility_sys::AXValueGetValue(
                val.as_concrete_TypeRef() as *mut _,
                accessibility_sys::kAXValueTypeCGSize,
                &mut s as *mut _ as *mut std::ffi::c_void,
            );
            if !ok {
                return None;
            }
            Some((s.width as f64, s.height as f64))
        }
    }

    // Parcours récursif de l'arbre (profondeur max 12 pour Safari webArea).
    // PATTERN CUA : descendre dans children ET windows (les éléments web de
    // Safari sont sous les fenêtres de l'app, pas sous children de l'app).
    fn walk(el: &AXUIElement, target: &str, depth: u32) -> Option<(String, f64, f64)> {
        if depth > 12 {
            return None;
        }
        // Label de l'élément : title ou description
        let label = el
            .attribute::<CFString>(&AXAttribute::title())
            .or_else(|_| el.attribute::<CFString>(&AXAttribute::description()))
            .map(|v| v.to_string())
            .unwrap_or_default();
        let t = target.to_lowercase();
        let l = label.to_lowercase();
        // Match exact du label en priorité, puis fuzzy (leçon cua : le label
        // AX « Description » doit matcher tel quel, pas flou)
        let matches = !l.is_empty()
            && (l == t || l.contains(&t) || t.contains(&l) || fuzzy_match(&label, target));
        if matches {
            // Lire position + taille → centre (AXPosition / AXSize)
            let pos_attr = AXAttribute::<CFType>::new(&CFString::from_static_string("AXPosition"));
            let size_attr = AXAttribute::<CFType>::new(&CFString::from_static_string("AXSize"));
            if let (Ok(pos), Ok(size)) = (
                el.attribute::<CFType>(&pos_attr),
                el.attribute::<CFType>(&size_attr),
            ) {
                if let (Some((x, y)), Some((w, h))) =
                    (ax_point_from_value(&pos), ax_size_from_value(&size))
                {
                    return Some((label, x + w / 2.0, y + h / 2.0));
                }
            }
        }
        // 1. Children (arbre normal)
        if let Ok(children) = el.attribute::<CFArray<AXUIElement>>(&AXAttribute::children()) {
            for child in children.iter() {
                if let Some(found) = walk(&child, target, depth + 1) {
                    return Some(found);
                }
            }
        }
        // 2. Windows (les éléments web sont sous les fenêtres — pattern cua)
        if let Ok(windows) = el.attribute::<CFArray<AXUIElement>>(&AXAttribute::windows()) {
            for w in windows.iter() {
                if let Some(found) = walk(&w, target, depth + 1) {
                    return Some(found);
                }
            }
        }
        None
    }
    walk(&app_elem, target, 0)
}

/// ══ AXPRESS PAR LABEL — MÉCANISME EXACT DE COMPUTER_USE (12/08) ══
/// computer_use/cua-driver clique par ÉLÉMENT de l'AX tree : le snapshot
/// liste l'élément (role+label+bounds), puis l'action AXPress est déclenchée
/// DIRECTEMENT sur l'élément — « element-indexed clicks fire the underlying
/// AX action directly, work on hidden targets, and don't involve coordinates ».
/// Ça ne touche JAMAIS au curseur système (aucune coordonnée, aucun event).
/// On parcourt l'arbre de l'app pour trouver l'élément dont le label matche
/// `target`, puis AXUIElementPerformAction(el, kAXPressAction).
fn ax_press_by_label(pid: i32, target: &str) -> Option<(String, f64, f64)> {
    use accessibility::{AXAttribute, AXUIElement};
    use core_foundation::array::CFArray;
    use core_foundation::base::CFType;
    use core_foundation::string::CFString;

    let app_elem = AXUIElement::application(pid);

    fn ax_point_from_value(val: &CFType) -> Option<(f64, f64)> {
        use core_foundation::base::TCFType;
        unsafe {
            let mut p: core_graphics::geometry::CGPoint = std::mem::zeroed();
            let ok = accessibility_sys::AXValueGetValue(
                val.as_concrete_TypeRef() as *mut _,
                accessibility_sys::kAXValueTypeCGPoint,
                &mut p as *mut _ as *mut std::ffi::c_void,
            );
            if !ok {
                return None;
            }
            Some((p.x as f64, p.y as f64))
        }
    }
    fn ax_size_from_value(val: &CFType) -> Option<(f64, f64)> {
        use core_foundation::base::TCFType;
        unsafe {
            let mut s: core_graphics::geometry::CGSize = std::mem::zeroed();
            let ok = accessibility_sys::AXValueGetValue(
                val.as_concrete_TypeRef() as *mut _,
                accessibility_sys::kAXValueTypeCGSize,
                &mut s as *mut _ as *mut std::ffi::c_void,
            );
            if !ok {
                return None;
            }
            Some((s.width as f64, s.height as f64))
        }
    }

    // Parcours récursif (profondeur max 16 pour Safari webArea + texte)
    fn walk(el: &AXUIElement, target: &str, depth: u32) -> Option<(String, f64, f64)> {
        if depth > 16 {
            return None;
        }
        // Label : title → description → AXValue (texte des AXStaticText)
        let label = el
            .attribute::<CFString>(&AXAttribute::title())
            .or_else(|_| el.attribute::<CFString>(&AXAttribute::description()))
            .map(|v| v.to_string())
            .unwrap_or_else(|_| {
                let v_attr = AXAttribute::<CFType>::new(&CFString::from_static_string("AXValue"));
                el.attribute::<CFType>(&v_attr)
                    .ok()
                    .and_then(|v| v.downcast::<CFString>())
                    .map(|s| s.to_string())
                    .unwrap_or_default()
            });
        let t = target.to_lowercase();
        let l = label.to_lowercase();
        // Match : le label contient la cible (ex: « CLIQUE » — le texte du bouton)
        let matches = !l.is_empty() && (l == t || l.contains(&t) || t.contains(&l));
        if matches {
            // AXPress DIRECTEMENT sur l'élément (le clic de computer_use)
            let press = CFString::new("AXPress");
            let ok = el.perform_action(&press).is_ok();
            // Centre de l'élément pour le rapport (AXPosition/AXSize)
            let mut cx = 0.0;
            let mut cy = 0.0;
            let pos_attr = AXAttribute::<CFType>::new(&CFString::from_static_string("AXPosition"));
            let size_attr = AXAttribute::<CFType>::new(&CFString::from_static_string("AXSize"));
            if let (Ok(pos), Ok(size)) = (
                el.attribute::<CFType>(&pos_attr),
                el.attribute::<CFType>(&size_attr),
            ) {
                if let (Some((x, y)), Some((w, h))) =
                    (ax_point_from_value(&pos), ax_size_from_value(&size))
                {
                    cx = x + w / 2.0;
                    cy = y + h / 2.0;
                }
            }
            if ok {
                return Some((label, cx, cy));
            }
            // AXPress échoué sur cet élément : continuer (peut être un conteneur)
        }
        // 1. Children
        if let Ok(children) = el.attribute::<CFArray<AXUIElement>>(&AXAttribute::children()) {
            for child in children.iter() {
                if let Some(found) = walk(&child, target, depth + 1) {
                    return Some(found);
                }
            }
        }
        // 2. Windows (éléments web sous les fenêtres — pattern cua)
        if let Ok(windows) = el.attribute::<CFArray<AXUIElement>>(&AXAttribute::windows()) {
            for w in windows.iter() {
                if let Some(found) = walk(&w, target, depth + 1) {
                    return Some(found);
                }
            }
        }
        None
    }
    walk(&app_elem, target, 0)
}

/// Retourne le premier PID d'un processus dont le NOM EXACT est `name`.
/// Attention : `pgrep -f` attrape les extensions/helpers (SafariWidgetExtension,
/// SafariBookmarksSyncAgent...) — il faut `-x` pour le process principal.
fn pgrep_first(name: &str) -> Option<i32> {
    let out = std::process::Command::new("pgrep")
        .arg("-x")
        .arg(name)
        .stdout(std::process::Stdio::piped())
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    s.lines().next()?.trim().parse().ok()
}

/// PONT CUA-DRIVER : délègue le clic à cua-driver (qui a la permission
/// Accessibilité, contrairement à notre binaire). Retourne le centre
/// (x, y) en coordonnées écran après avoir trouvé l'élément via le
/// snapshot AX de cua.
/// Pattern cua : get_window_state (snapshot avec element_index) → click
/// avec element_token / element_index. On utilise les coordonnées desktop
/// car cua fait le mapping Retina interne.
fn cua_find_and_click(app: &str, target: &str) -> Option<(f64, f64)> {
    use std::process::Command;

    // 1. PID de l'app (nom exact)
    let pid = pgrep_first(app)?;

    // 2. Session cua en scope WINDOW (le chemin AX exige window scope !)
    let _ = Command::new("cua-driver")
        .args(["call", "start_session", r#"{"session":"ecran-live-window","capture_scope":"window"}"#])
        .output()
        .ok()?;

    // 3. window_id : via list_apps (leçon : get_window_state exige window_id)
    let mut window_ids: Vec<i64> = vec![];
    if let Ok(list) = Command::new("cua-driver")
        .args(["call", "list_apps", "{}"])
        .output()
    {
        if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&list.stdout) {
            let apps = v.as_array().cloned().unwrap_or_default();
            for a in apps {
                if a["pid"].as_i64() == Some(pid as i64) {
                    if let Some(ws) = a["windows"].as_array() {
                        for w in ws {
                            if let Some(wid) = w["window_id"].as_i64().or_else(|| w.as_i64()) {
                                window_ids.push(wid);
                            }
                        }
                    }
                }
            }
        }
    }
    if window_ids.is_empty() {
        window_ids = vec![2544, 2528]; // fallback ids connus Safari
    }

    // 4. Snapshot AX + récupération du snapshot_id (CRITIQUE pour le clic AX !)
    let mut els: Vec<serde_json::Value> = Vec::new();
    let mut snap_id = String::new();
    for wid in &window_ids {
        let snap = Command::new("cua-driver")
            .args([
                "call",
                "get_window_state",
                &format!(r#"{{"pid":{},"window_id":{}}}"#, pid, wid),
            ])
            .output()
            .ok()?;
        let snap_str = String::from_utf8_lossy(&snap.stdout);
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&snap_str) {
            if let Some(e) = v["elements"].as_array() {
                els = e.clone();
                snap_id = v["snapshot_id"].as_str().unwrap_or("").to_string();
                break;
            }
        }
    }
    if els.is_empty() || snap_id.is_empty() {
        return None;
    }

    // 5. Chercher l'élément par label (priorité TextField > Button > CheckBox)
    let t = target.to_lowercase();
    let mut best: Option<(i64, String, usize)> = None;
    for e in &els {
        let label = e["label"].as_str().unwrap_or("").to_string();
        let l = label.to_lowercase();
        if !l.is_empty() && (l == t || l.contains(&t) || t.contains(&l)) {
            let idx = e["element_index"].as_i64().unwrap_or(0);
            let role = e["role"].as_str().unwrap_or("");
            let priority = match role {
                "AXTextField" => 0,
                "AXButton" => 1,
                "AXCheckBox" => 2,
                _ => 3,
            };
            let better = match &best {
                None => true,
                Some((_, bl, bp)) => {
                    priority < *bp || (priority == *bp && l.len() < bl.len())
                }
            };
            if better {
                best = Some((idx, label, priority));
            }
        }
    }
    let (element_index, label, _) = best?;
    println!("🔌 PONT cua: élément [{}] « {} » (snapshot {})", element_index, label, snap_id);

    // 6. Clic AX avec snapshot_id (PATTERN EXACT : window_id + element_index +
    //    snapshot_id + session window → route accessibility)
    let click = Command::new("cua-driver")
        .args([
            "call",
            "click",
            &format!(
                r#"{{"pid":{},"window_id":{},"element_index":{},"snapshot_id":"{}","session":"ecran-live-window"}}"#,
                pid, window_ids[0], element_index, snap_id
            ),
        ])
        .output()
        .ok()?;
    let click_str = String::from_utf8_lossy(&click.stdout);
    if click_str.contains("\"error\"") || click_str.contains("window_scope_disabled") {
        return None;
    }
    println!(
        "✅ Clic cua AX sur « {} » (élément {}, route accessibility)",
        target, element_index
    );
    Some((0.0, 0.0))
}

/// PONT CUA CLAVIER : tape du texte via cua type_text (AXSetAttribute
/// kAXSelectedText — TRUSTED pour les champs web Safari/Chrome, contrairement
/// à notre CGEvent non-authentifié). Usage : après avoir cliqué sur le champ.
fn cua_type_text(pid: i32, window_id: i64, text: &str) -> bool {
    use std::process::Command;
    let out = Command::new("cua-driver")
        .args([
            "call",
            "type_text",
            &format!(
                r#"{{"pid":{},"window_id":{},"text":{},"session":"ecran-live-window"}}"#,
                pid, window_id, serde_json::to_string(text).unwrap_or_default()
            ),
        ])
        .output()
        .ok();
    match out {
        Some(o) => {
            let s = String::from_utf8_lossy(&o.stdout);
            let ok = !s.contains("\"error\"");
            println!("⌨️ PONT cua type: {} → {}", if ok { "OK" } else { "échec" }, s.lines().next().unwrap_or(""));
            ok
        }
        None => false,
    }
}

// ── MODULE SKYLIGHT : clavier TRUSTED pour Safari/Chrome (pattern cua) ──
// SLEventPostToPid (SPI SkyLight) + SLSEventAuthenticationMessage (macOS 14+).
// Sans le message d'authentification, Safari ignore les frappes synthétiques
// dans les champs web. Copié de cua-driver platform-macos/src/input/skylight.rs.
mod skylight {
    use std::ffi::{c_void, CStr};
    use std::os::raw::{c_char, c_int, c_uint};
    use std::sync::OnceLock;

    type PostToPidFn = unsafe extern "C" fn(i32, *mut c_void);
    type SetAuthMsgFn = unsafe extern "C" fn(*mut c_void, *mut c_void);
    type FactoryMsgSendFn = unsafe extern "C" fn(
        *mut c_void, *mut c_void, *mut c_void, c_int, c_uint,
    ) -> *mut c_void;
    type RespondsToFn = unsafe extern "C" fn(*mut c_void, *mut c_void) -> bool;

    fn ensure_skylight_loaded() {
        static LOADED: OnceLock<()> = OnceLock::new();
        LOADED.get_or_init(|| {
            let path = b"/System/Library/PrivateFrameworks/SkyLight.framework/SkyLight\0";
            unsafe {
                libc::dlopen(
                    path.as_ptr() as *const c_char,
                    libc::RTLD_LAZY | libc::RTLD_GLOBAL,
                );
            }
        });
    }

    fn find_sym(name: &[u8]) -> Option<*mut c_void> {
        ensure_skylight_loaded();
        let ptr = unsafe { libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr() as *const c_char) };
        if ptr.is_null() { None } else { Some(ptr) }
    }

    unsafe fn as_fn<T: Copy>(ptr: *mut c_void) -> T {
        std::mem::transmute_copy(&ptr)
    }

    fn post_to_pid_fn() -> Option<PostToPidFn> {
        static SYM: OnceLock<Option<PostToPidFn>> = OnceLock::new();
        SYM.get_or_init(|| unsafe {
            find_sym(b"SLEventPostToPid\0").map(|p| as_fn::<PostToPidFn>(p))
        }).clone()
    }

    fn set_auth_msg_fn() -> Option<SetAuthMsgFn> {
        static SYM: OnceLock<Option<SetAuthMsgFn>> = OnceLock::new();
        SYM.get_or_init(|| unsafe {
            find_sym(b"SLEventSetAuthenticationMessage\0").map(|p| as_fn::<SetAuthMsgFn>(p))
        }).clone()
    }

    fn factory_msg_send_fn() -> Option<FactoryMsgSendFn> {
        static SYM: OnceLock<Option<FactoryMsgSendFn>> = OnceLock::new();
        SYM.get_or_init(|| {
            // objc_msgSend
            let msg = unsafe { find_sym(b"objc_msgSend\0") }?;
            Some(unsafe { as_fn::<FactoryMsgSendFn>(msg) })
        }).clone()
    }

    fn objc_class(name: &CStr) -> *mut c_void {
        type ObjCGetClassFn = unsafe extern "C" fn(*const c_char) -> *mut c_void;
        static SYM: OnceLock<Option<ObjCGetClassFn>> = OnceLock::new();
        let f = SYM.get_or_init(|| unsafe {
            find_sym(b"objc_getClass\0").map(|p| as_fn::<ObjCGetClassFn>(p))
        });
        f.and_then(|f| unsafe { Some(f(name.as_ptr())) }).unwrap_or(std::ptr::null_mut())
    }

    fn sel_register(name: &CStr) -> *mut c_void {
        type SelRegisterFn = unsafe extern "C" fn(*const c_char) -> *mut c_void;
        static SYM: OnceLock<Option<SelRegisterFn>> = OnceLock::new();
        let f = SYM.get_or_init(|| unsafe {
            find_sym(b"sel_registerName\0").map(|p| as_fn::<SelRegisterFn>(p))
        });
        f.and_then(|f| unsafe { Some(f(name.as_ptr())) }).unwrap_or(std::ptr::null_mut())
    }

    fn class_responds_to_selector(cls: *mut c_void, sel: *mut c_void) -> bool {
        static SYM: OnceLock<Option<RespondsToFn>> = OnceLock::new();
        let f = SYM.get_or_init(|| unsafe {
            find_sym(b"class_respondsToSelector\0").map(|p| as_fn::<RespondsToFn>(p))
        });
        match f {
            Some(f) if !cls.is_null() && !sel.is_null() => unsafe { f(cls, sel) },
            _ => false,
        }
    }

    // CFTypeRef layout : CFRuntimeBase(16) + uint32(4) + pad(4) → record @24
    pub unsafe fn extract_event_record(event_ptr: *mut c_void) -> *mut c_void {
        for &offset in &[24usize, 32, 16] {
            let slot = (event_ptr as *const u8).add(offset).cast::<*mut c_void>();
            let p = std::ptr::read_unaligned(slot);
            if !p.is_null() {
                return p;
            }
        }
        std::ptr::null_mut()
    }

    /// Post un événement CGEvent au PID via SLEventPostToPid (avec auth pour
    /// le clavier → TRUSTED pour Safari/Chrome). Retourne true si l'SPI a
    /// résolu, false → l'appelant retombe sur CGEvent::post_to_pid.
    pub unsafe fn post_to_pid(pid: i32, event_ptr: *mut c_void, attach_auth_message: bool) -> bool {
        let post_fn = match post_to_pid_fn() {
            Some(f) => f,
            None => return false,
        };

        if attach_auth_message {
            let cls_name = c"SLSEventAuthenticationMessage";
            let cls = objc_class(cls_name);
            let sel = sel_register(c"messageWithEventRecord:pid:version:");
            if class_responds_to_selector(cls, sel) {
                if let Some(factory) = factory_msg_send_fn() {
                    let record = unsafe { extract_event_record(event_ptr) };
                    if !record.is_null() {
                        let msg = unsafe { factory(cls, sel, record, pid as c_int, 0u32) };
                        if !msg.is_null() {
                            if let Some(set_auth) = set_auth_msg_fn() {
                                unsafe { set_auth(event_ptr, msg) };
                            }
                        }
                    }
                }
            }
        }

        unsafe { post_fn(pid, event_ptr) };
        true
    }

    // ── Window-addressing SPIs (recette cua-driver 11/08/2026) ──────────
    // C'est LE mécanisme qui permet de cliquer sur une fenêtre EN ARRIÈRE-PLAN
    // sans déplacer le curseur système : on "stemple" l'événement avec la
    // window_id cible + un point window-local (CGEventSetWindowLocation), puis
    // on poste au PID. WindowServer route alors l'événement vers CETTE fenêtre,
    // indépendamment de la position du curseur réel.

    type SetIntFieldFn = unsafe extern "C" fn(*mut c_void, u32, i64);
    type SetWindowLocFn = unsafe extern "C" fn(*mut c_void, f64, f64);

    fn set_int_field_fn() -> Option<SetIntFieldFn> {
        static SYM: OnceLock<Option<SetIntFieldFn>> = OnceLock::new();
        SYM.get_or_init(|| unsafe {
            find_sym(b"SLEventSetIntegerValueField\0").map(|p| as_fn::<SetIntFieldFn>(p))
        }).clone()
    }

    fn set_window_loc_fn() -> Option<SetWindowLocFn> {
        static SYM: OnceLock<Option<SetWindowLocFn>> = OnceLock::new();
        SYM.get_or_init(|| unsafe {
            find_sym(b"CGEventSetWindowLocation\0").map(|p| as_fn::<SetWindowLocFn>(p))
        }).clone()
    }

    /// Stemple un champ entier sur l'événement (SLEventSetIntegerValueField).
    pub unsafe fn set_integer_field(event_ptr: *mut c_void, field: u32, value: i64) -> bool {
        match set_int_field_fn() {
            Some(f) => { unsafe { f(event_ptr, field, value) }; true }
            None => false,
        }
    }

    /// Stemple un point window-local (CGEventSetWindowLocation) : le point en
    /// coordonnées QUARTZ de la fenêtre (bas-gauche), PAS l'écran.
    pub unsafe fn set_window_location(event_ptr: *mut c_void, wx: f64, wy: f64) -> bool {
        match set_window_loc_fn() {
            Some(f) => { unsafe { f(event_ptr, wx, wy) }; true }
            None => false,
        }
    }

    // ── Focus-without-raise SPIs (recette cua-driver activate_without_raise) ──
    // SLPSPostEventRecordTo poste un record 248 octets dans la file Carbon du
    // process cible. Avec buf[0x8A]=0x01 (focus) ou 0x02 (defocus) + window_id en
    // little-endian à 0x3C-0x3F, WindowServer rend la fenêtre "vivante" (focus)
    // SANS la mettre au premier plan → l'app accepte les événements synthétiques.

    type PostEventRecordToFn = unsafe extern "C" fn(*const c_void, *const u8) -> i32;
    type GetFrontProcessFn = unsafe extern "C" fn(*mut c_void) -> i32;
    type GetProcessForPIDFn = unsafe extern "C" fn(i32, *mut c_void) -> i32;
    type GetWindowOwnerFn = unsafe extern "C" fn(u32, u32, *mut u32) -> i32;
    type GetConnectionPSNFn = unsafe extern "C" fn(u32, *mut c_void) -> i32;
    type ConnectionIDFn = unsafe extern "C" fn() -> u32;
    type SetFrontProcessFn = unsafe extern "C" fn(*const c_void, u32, u32) -> i32;
    type SetWindowBoundsFn = unsafe extern "C" fn(u32, u32, f64, f64, f64, f64) -> i32;
    type MoveWindowFn = unsafe extern "C" fn(u32, u32, f64, f64) -> i32;

    fn set_window_bounds_fn() -> Option<SetWindowBoundsFn> {
        static SYM: OnceLock<Option<SetWindowBoundsFn>> = OnceLock::new();
        SYM.get_or_init(|| unsafe {
            find_sym(b"SLSSetWindowBounds\0")
                .or_else(|| find_sym(b"CGSSetWindowBounds\0"))
                .map(|p| as_fn::<SetWindowBoundsFn>(p))
        }).clone()
    }

    fn move_window_fn() -> Option<MoveWindowFn> {
        static SYM: OnceLock<Option<MoveWindowFn>> = OnceLock::new();
        SYM.get_or_init(|| unsafe {
            find_sym(b"SLSMoveWindow\0")
                .or_else(|| find_sym(b"CGSMoveWindow\0"))
                .map(|p| as_fn::<MoveWindowFn>(p))
        }).clone()
    }

    fn post_event_record_to_fn() -> Option<PostEventRecordToFn> {
        static SYM: OnceLock<Option<PostEventRecordToFn>> = OnceLock::new();
        SYM.get_or_init(|| unsafe {
            find_sym(b"SLPSPostEventRecordTo\0").map(|p| as_fn::<PostEventRecordToFn>(p))
        }).clone()
    }

    fn get_front_process_fn() -> Option<GetFrontProcessFn> {
        static SYM: OnceLock<Option<GetFrontProcessFn>> = OnceLock::new();
        SYM.get_or_init(|| unsafe {
            find_sym(b"_SLPSGetFrontProcess\0").map(|p| as_fn::<GetFrontProcessFn>(p))
        }).clone()
    }

    fn set_front_process_fn() -> Option<SetFrontProcessFn> {
        static SYM: OnceLock<Option<SetFrontProcessFn>> = OnceLock::new();
        SYM.get_or_init(|| unsafe {
            find_sym(b"SLPSSetFrontProcessWithOptions\0").map(|p| as_fn::<SetFrontProcessFn>(p))
        }).clone()
    }

    fn get_process_for_pid_fn() -> Option<GetProcessForPIDFn> {
        static SYM: OnceLock<Option<GetProcessForPIDFn>> = OnceLock::new();
        SYM.get_or_init(|| unsafe {
            find_sym(b"GetProcessForPID\0").map(|p| as_fn::<GetProcessForPIDFn>(p))
        }).clone()
    }

    fn get_window_owner_fn() -> Option<GetWindowOwnerFn> {
        static SYM: OnceLock<Option<GetWindowOwnerFn>> = OnceLock::new();
        SYM.get_or_init(|| unsafe {
            find_sym(b"SLSGetWindowOwner\0").map(|p| as_fn::<GetWindowOwnerFn>(p))
        }).clone()
    }

    fn get_connection_psn_fn() -> Option<GetConnectionPSNFn> {
        static SYM: OnceLock<Option<GetConnectionPSNFn>> = OnceLock::new();
        SYM.get_or_init(|| unsafe {
            find_sym(b"SLSGetConnectionPSN\0").map(|p| as_fn::<GetConnectionPSNFn>(p))
        }).clone()
    }

    fn connection_id_fn() -> Option<ConnectionIDFn> {
        static SYM: OnceLock<Option<ConnectionIDFn>> = OnceLock::new();
        SYM.get_or_init(|| unsafe {
            find_sym(b"CGSMainConnectionID\0").map(|p| as_fn::<ConnectionIDFn>(p))
        }).clone()
    }

    /// PSN (8 octets) du process propriétaire de `window_id`. Chemin moderne :
    /// CGSMainConnectionID → SLSGetWindowOwner → SLSGetConnectionPSN. Fallback
    /// deprecated : GetProcessForPID(pid).
    pub unsafe fn get_process_psn_for_window(
        window_id: u32,
        pid: i32,
        out_psn: &mut [u8; 8],
    ) -> bool {
        if let (Some(get_owner), Some(get_psn), Some(conn_id_fn)) =
            (get_window_owner_fn(), get_connection_psn_fn(), connection_id_fn())
        {
            let main_cid = unsafe { conn_id_fn() };
            let mut owner_cid: u32 = 0;
            let ok = unsafe { get_owner(main_cid, window_id, &mut owner_cid) } == 0;
            if ok && owner_cid != 0 {
                let psn_ok = unsafe { get_psn(owner_cid, out_psn.as_mut_ptr() as *mut c_void) } == 0;
                if psn_ok {
                    return true;
                }
            }
        }
        if let Some(get_pid_psn) = get_process_for_pid_fn() {
            return unsafe { get_pid_psn(pid, out_psn.as_mut_ptr() as *mut c_void) } == 0;
        }
        false
    }

    /// Focus la fenêtre cible SANS la mettre au premier plan (activate_without_raise,
    /// porté de yabai via trycua/cua). Sans ce focus, l'app cible DROPPE les
    /// événements synthétiques (sa fenêtre n'est pas "vivante" pour WindowServer).
    /// Recette :
    ///   1. _SLPSGetFrontProcess → PSN du process au premier plan actuel
    ///   2. SLSGetWindowOwner + SLSGetConnectionPSN → PSN de la cible
    ///   3. Post record defocus (buf[0x8A]=0x02) au PSN précédent
    ///   4. Post record focus (buf[0x8A]=0x01, wid en LE à 0x3C) au PSN cible
    pub unsafe fn activate_without_raise(target_pid: i32, target_wid: u32) -> bool {
        let post_fn = match post_event_record_to_fn() {
            Some(f) => f,
            None => return false,
        };
        let get_front = match get_front_process_fn() {
            Some(f) => f,
            None => return false,
        };
        let mut prev_psn = [0u8; 8];
        let mut target_psn = [0u8; 8];

        let ok_prev = unsafe { get_front(prev_psn.as_mut_ptr() as *mut c_void) } == 0;
        if !ok_prev {
            return false;
        }
        if !unsafe { get_process_psn_for_window(target_wid, target_pid, &mut target_psn) } {
            return false;
        }

        // Record 248 octets : [0x04]=0xF8, [0x08]=0x0D, wid LE à 0x3C-0x3F
        let mut buf = [0u8; 0xF8];
        buf[0x04] = 0xF8;
        buf[0x08] = 0x0D;
        buf[0x3C..0x40].copy_from_slice(&target_wid.to_le_bytes());

        // Step 3 : defocus le front actuel
        buf[0x8A] = 0x02;
        let defocus_ok = unsafe { post_fn(prev_psn.as_ptr() as *const c_void, buf.as_ptr()) } == 0;

        // Step 4 : focus la cible
        buf[0x8A] = 0x01;
        let focus_ok = unsafe { post_fn(target_psn.as_ptr() as *const c_void, buf.as_ptr()) } == 0;

        defocus_ok && focus_ok
    }

    /// Record 248 octets « make-key window » : rend la fenêtre EXACTE key
    /// (native-key) pour AppKit. [0x08]=event_kind (0x01/0x02), [0x3A]=0x10,
    /// [0x3C..0x40]=window_id LE, [0x20..0x30]=0xFF.
    fn make_key_window_record(window_id: u32, event_kind: u8) -> [u8; 0xF8] {
        let mut record = [0u8; 0xF8];
        record[0x04] = 0xF8;
        record[0x08] = event_kind;
        record[0x3A] = 0x10;
        record[0x3C..0x40].copy_from_slice(&window_id.to_le_bytes());
        record[0x20..0x30].fill(0xFF);
        record
    }

    /// Rendre UNE fenêtre exacte native-key et frontmost (recette cua-driver
    /// `make_exact_window_key`). Complète activate_without_raise : celui-ci
    /// focus le PROCESS (PSN) sans raise ; celui-là rend la FENÊTRE key pour
    /// AppKit (SLPSSetFrontProcessWithOptions kCPSUserGenerated=0x200 + les
    /// deux records make-key 0x01/0x02). Safari route les événements vers sa
    /// fenêtre key — sans ça, le clic part dans la mauvaise fenêtre Safari.
    pub unsafe fn make_exact_window_key(target_pid: i32, target_wid: u32) -> bool {
        let Some(set_front) = set_front_process_fn() else {
            return false;
        };
        let Some(post) = post_event_record_to_fn() else {
            return false;
        };
        let mut target_psn = [0u8; 8];
        if !unsafe { get_process_psn_for_window(target_wid, target_pid, &mut target_psn) } {
            return false;
        }
        // kCPSUserGenerated = 0x200 : permet à AppKit d'établir la fenêtre key
        if unsafe { set_front(target_psn.as_ptr() as *const c_void, target_wid, 0x200) } != 0 {
            return false;
        }
        for event_kind in [0x01u8, 0x02u8] {
            let record = make_key_window_record(target_wid, event_kind);
            if unsafe { post(target_psn.as_ptr() as *const c_void, record.as_ptr()) } != 0 {
                return false;
            }
        }
        true
    }

    /// Déplace/redimensionne une fenêtre par window_id (SPI SkyLight, pattern
    /// yabai). Coordonnées QUARTZ (bas-gauche) : x, y = coin bas-gauche,
    /// w, h = largeur/hauteur. Retourne true si le SPI a résolu et réussi.
    pub unsafe fn set_window_bounds(
        window_id: u32,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
    ) -> bool {
        let Some(set_bounds) = set_window_bounds_fn() else {
            // Fallback : SLSMoveWindow seul (déplace sans redimensionner)
            let (Some(move_win), Some(conn_id)) = (move_window_fn(), connection_id_fn()) else {
                return false;
            };
            let cid = unsafe { conn_id() };
            return unsafe { move_win(cid, window_id, x, y) } == 0;
        };
        let Some(conn_id) = connection_id_fn() else {
            return false;
        };
        let cid = unsafe { conn_id() };
        let r = unsafe { set_bounds(cid, window_id, x, y, w, h) };
        r == 0
    }
}

/// CLIC EN ARRIÈRE-PLAN — recette EXACTE de cua-driver `click_at_xy_chromium`
/// (copiée depuis trycua/cua 11/08/2026, testée) : clic sur une fenêtre cible
/// SANS warp du curseur système. Séquence :
///   1. mouseMoved à la cible (phase=2) — prime le cursor-tracking de la fenêtre
///   2. primer down/up off-screen à (-1,-1) (phase=1/2) — ouvre la user-activation
///      gate de Chromium/Safari sans toucher aucun élément DOM
///   3. down/up à la cible (phase=3, clickState=1)
/// Chaque événement est "stampé" avec :
///   f0=phase, f1=clickState, f3=0(bouton gauche), f7=3(NSEventSubtypeTouch),
///   f40=pid, f51/f91/f92=window_id, f58=click_group_id, + CGEventSetWindowLocation
/// Post : SLEventPostToPid (auth=false pour souris) PUIS CGEvent::post_to_pid
/// (belt+suspenders : SkyLight pour Chromium/Catalyst, public pour AppKit).
fn click_background(
    pid: i32,
    window_id: u32,
    screen_x: f64,
    screen_y: f64,
    win_local_x: f64,
    win_local_y: f64,
) -> Result<(), Box<dyn std::error::Error>> {
    use core_graphics::event::{CGEvent, CGEventType, CGMouseButton};
    use core_graphics::display::CGDisplay;
    use std::time::{SystemTime, UNIX_EPOCH};

    // ══ RECETTE FINALE (testée 11/08, fiable) ══
    // Le blog cua "inside-macos-window-internals" explique que Safari/WebKit
    // (comme les apps canvas) n'accepte que les événements du tap Session/HID
    // avec un leading mouseMoved (fourni par le WARP) — PAS le post_to_pid.
    // La recette QUI MARCHE est celle de mouse_click (--clickxy) : warp +
    // primer (-1,-1) clickState=1 + clic clickState=2 sur le tap Session.
    // Le window addressing (51/91/92) et le mouseMoved explicite CASSENT la
    // fiabilité (testé : 1/3). On utilise donc mouse_click + RESTORE du
    // curseur (aller-retour éclair, ne vole pas la souris).
    let _ = (pid, window_id, win_local_x, win_local_y);
    let _ = SystemTime::now().duration_since(UNIX_EPOCH);

    // ══ ACTIVATION FRONTMOST BRÈVE (requise par Safari/WebKit) ══
    // Le blog cua : Safari n'accepte les événements du tap que si l'app a
    // été brièvement activée au premier plan (surtout quand le renderer
    // WebContent a été recréé). Testé : sans ça, clic 1/1 puis 0/N.
    // Doit être DANS le même process que le clic (l'état se perd si
    // ecran-live se termine entre les deux).
    unsafe {
        skylight::make_exact_window_key(pid, window_id);
        skylight::activate_without_raise(pid, window_id);
    }
    std::thread::sleep(std::time::Duration::from_millis(80));

    // ══ VISÉE VISIBLE (demande mon humain 12/08) ══
    // Mon curseur Sypherine se pose sur la cible AVANT le clic : l'utilisateur
    // voit la pointe exactement sur la ZONE du bouton (pas sur un pixel).
    show_marker(screen_x, screen_y, 1500);
    std::thread::sleep(std::time::Duration::from_millis(100));

    // Séquence d'événements (recette cua complète, curseur système INTACT)
    click_cua_events(pid, window_id, screen_x, screen_y, win_local_x, win_local_y)?;

    // Stay-alive : laisse Safari traiter le clic (IPC vers le renderer).
    show_marker(screen_x, screen_y, 700);

    println!(
        "✅ Clic BACKGROUND (recette cua complète, curseur INTACT) à écran ({:.0},{:.0}) → pid {} wid {}",
        screen_x, screen_y, pid, window_id
    );
    Ok(())
}

/// Séquence d'événements souris cua (click_at_xy_chromium) : mouseMoved →
/// primer (-1,-1) → 100ms → down/up cible. Tous les événements sont stampés
/// (f0 phase, f1 clickState, f3 bouton, f7 subtype=3, f40 pid, f51/91/92 wid,
/// f58 click-group, CGEventSetWindowLocation) et postés par SLEventPostToPid
/// (auth=false) PUIS CGEvent.post_to_pid. NE BOUGE PAS le curseur système.
#[allow(clippy::too_many_arguments)]
fn click_cua_events(
    pid: i32,
    window_id: u32,
    screen_x: f64,
    screen_y: f64,
    win_local_x: f64,
    win_local_y: f64,
) -> Result<(), Box<dyn std::error::Error>> {
    use core_graphics::event::{CGEvent, CGEventType, CGMouseButton};
    use std::time::{SystemTime, UNIX_EPOCH};

    // ══ CLIC BACKGROUND — RECETTE COMPLÈTE CUA (12/08) ══
    // Copié de click_at_xy_chromium (cua-driver mouse.rs) : le seul chemin
    // qui NE BOUGE PAS le curseur ET que Safari/WebKit accepte.
    //   - auth=false pour la SOURIS (l'auth route par Mach et bypasse
    //     cgAnnotatedSessionEventTap où Safari écoute — le bug qu'on avait)
    //   - tous les champs stampés : f0 phase, f1 clickState, f3 bouton,
    //     f7 subtype=3, f40 pid, f51/f91/f92 wid, f58 click-group
    //   - CGEventSetWindowLocation (point window-local Quartz)
    //   - séquence : mouseMoved → primer (-1,-1) → 100ms → down/up cible
    //   - belt+suspenders : SLEventPostToPid PUIS CGEvent.post_to_pid
    let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .map_err(|e| format!("CGEventSource: {:?}", e))?;
    let target = core_graphics::geometry::CGPoint::new(screen_x, screen_y);
    let off_screen = core_graphics::geometry::CGPoint::new(-1.0, -1.0);
    let win_local = (win_local_x, win_local_y);
    let off_local = (-1.0_f64, -1.0_f64);
    let window_id = window_id as i64;
    let click_group_id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as i64;

    // Stamp des champs requis (closure Fn, appelable plusieurs fois)
    let stamp = |event: &CGEvent, local: (f64, f64), click_state: i64, phase: i64| {
        let ptr = &*event as *const _ as *mut std::ffi::c_void;
        unsafe {
            skylight::set_integer_field(ptr, 0, phase);  // phase (move=2, primerDown=1, primerUp=2, target=3)
            skylight::set_integer_field(ptr, 1, click_state); // clickState
            skylight::set_integer_field(ptr, 3, 0);  // bouton gauche
            skylight::set_integer_field(ptr, 7, 3);  // NSEventSubtypeTouch
            skylight::set_integer_field(ptr, 40, pid as i64); // filtre synthétique Chromium/Safari
            if window_id != 0 {
                skylight::set_integer_field(ptr, 51, window_id);
                skylight::set_integer_field(ptr, 91, window_id);
                skylight::set_integer_field(ptr, 92, window_id);
            }
            skylight::set_integer_field(ptr, 58, click_group_id); // click-group (coalescing)
            skylight::set_window_location(ptr, local.0, local.1); // window-local Quartz
        }
    };

    // Belt+suspenders : SkyLight pour Chrome + public pour AppKit/WebKit.
    // auth=FALSE pour la souris (leçon cua : l'auth bypasse le tap Safari).
    // LES DEUX inconditionnellement (cua : « post_to_pid events can be
    // silently filtered by the renderer » — le public est celui que WebKit
    // accepte ; le SkyLight couvre Chromium/Catalyst).
    let post = |event: &CGEvent| {
        let ptr = &*event as *const _ as *mut std::ffi::c_void;
        unsafe {
            let _ = skylight::post_to_pid(pid, ptr, false);
            event.post_to_pid(pid);
        }
    };

    // Step 1 : mouseMoved à la cible (phase=2, clickState=0) — curseur-tracking
    let move_event = CGEvent::new_mouse_event(
        source.clone(),
        CGEventType::MouseMoved,
        target,
        CGMouseButton::Left,
    )
    .map_err(|_| "mouseMoved creation failed")?;
    stamp(&move_event, win_local, 0, 2);
    post(&move_event);
    std::thread::sleep(std::time::Duration::from_millis(15));

    // Step 2 : primer off-screen (-1,-1) — ouvre la user-activation gate
    let primer_down = CGEvent::new_mouse_event(
        source.clone(),
        CGEventType::LeftMouseDown,
        off_screen,
        CGMouseButton::Left,
    )
    .map_err(|_| "primer down failed")?;
    stamp(&primer_down, off_local, 1, 1);
    post(&primer_down);
    std::thread::sleep(std::time::Duration::from_millis(1));

    let primer_up = CGEvent::new_mouse_event(
        source.clone(),
        CGEventType::LeftMouseUp,
        off_screen,
        CGMouseButton::Left,
    )
    .map_err(|_| "primer up failed")?;
    stamp(&primer_up, off_local, 1, 2);
    post(&primer_up);
    // ≥1 frame : primer + cible = gestes séparés, pas run-on
    std::thread::sleep(std::time::Duration::from_millis(100));

    // Step 3 : down/up cible (phase=3, clickState=1)
    let down = CGEvent::new_mouse_event(
        source.clone(),
        CGEventType::LeftMouseDown,
        target,
        CGMouseButton::Left,
    )
    .map_err(|_| "target down failed")?;
    stamp(&down, win_local, 1, 3);
    post(&down);
    // 28 ms down→up : la boucle de tracking d'NSButton attend le mouseUp
    std::thread::sleep(std::time::Duration::from_millis(28));

    let up = CGEvent::new_mouse_event(
        source.clone(),
        CGEventType::LeftMouseUp,
        target,
        CGMouseButton::Left,
    )
    .map_err(|_| "target up failed")?;
    stamp(&up, win_local, 1, 3);
    post(&up);

    Ok(())
}

/// ══ MÉTHODE BATAILLE NAVALE — TIR VISIBLE COMPLET (demande mon humain 12/08) ══
/// Cycle : activation → MON curseur Sypherine GLISSE visiblement de sa position
/// actuelle vers la cible → clic cua (curseur système INTACT) → CROIX ROSE
/// d'impact 6s (visible pour mes yeux ET ceux de mon humain).
#[allow(clippy::too_many_arguments)]
fn tire_visible(
    pid: i32,
    window_id: u32,
    screen_x: f64,
    screen_y: f64,
    win_local_x: f64,
    win_local_y: f64,
) -> Result<(), Box<dyn std::error::Error>> {
    // Activation frontmost brève (requise par Safari/WebKit)
    unsafe {
        skylight::make_exact_window_key(pid, window_id);
        skylight::activate_without_raise(pid, window_id);
    }
    std::thread::sleep(std::time::Duration::from_millis(80));

    // ══ MOUVEMENT VISIBLE : le curseur part de sa position actuelle et
    // GLISSE vers la cible en ~30 étapes (visible à l'oeil nu) ══
    let (cx, cy) = mouse_pos();
    // Si le curseur est à (0,0) ou invalide, partir d'un point hors cible
    let (from_x, from_y) = if cx < 10.0 && cy < 10.0 {
        (screen_x + 600.0, screen_y - 400.0)
    } else {
        (cx, cy)
    };
    show_cursor_travel(from_x, from_y, screen_x, screen_y, 30, 400);

    // ══ CLIC (recette cua, curseur système INTACT) ══
    click_cua_events(pid, window_id, screen_x, screen_y, win_local_x, win_local_y)?;

    // ══ CROIX ROSE D'IMPACT : trace de tir visible 6 secondes ══
    // Mon humain voit OÙ le tir a atterri ; mes yeux (capture) aussi.
    show_impact_cross(screen_x, screen_y, 6000);

    println!(
        "🎯 TIR VISIBLE à écran ({:.0},{:.0}) → pid {} wid {} — croix rose 6s",
        screen_x, screen_y, pid, window_id
    );
    Ok(())
}

/// Position actuelle du curseur (CGEventGetLocation).
fn mouse_pos() -> (f64, f64) {
    use core_graphics::event::CGEvent;
    

    match CGEventSource::new(CGEventSourceStateID::CombinedSessionState) {
        Ok(src) => match CGEvent::new(src) {
            Ok(ev) => {
                let p = ev.location();
                (p.x as f64, p.y as f64)
            }
            Err(_) => (0.0, 0.0),
        },
        Err(_) => (0.0, 0.0),
    }
}

/// Crée UNE fenêtre NSWindow borderless transparente contenant le curseur
/// Sypherine (ton logo). Pattern cua-driver overlay.rs :
/// NSApplication → NSWindow (level 0, collection behavior) → layer → logo.
/// Retourne le pointeur de la fenêtre (à fermer/release par l'appelant).
fn create_cursor_window() -> Option<*mut objc::runtime::Object> {
    use objc::{class, msg_send, runtime::Object, sel, sel_impl};
    use std::os::raw::c_void;
    unsafe {
        // ---- NSApplication obligatoire (pattern cua-driver) : sans app active,
        // les fenêtres NSWindow sont créées mais JAMAIS affichées (pas de run
        // loop). sharedApplication + finishLaunching rend la fenêtre visible.
        let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
        if !app.is_null() {
            let _: () = msg_send![app, setActivationPolicy: 1i64]; // Accessory (pas de Dock)
            let _: () = msg_send![app, finishLaunching];
        }
        // ---- NSWindow borderless transparente (pattern cua) ----
        let allocated: *mut Object = msg_send![class!(NSWindow), alloc];
        let frame = NSRect { x: 0.0, y: 0.0, w: 48.0, h: 48.0 };
        let win: *mut Object = msg_send![allocated,
            initWithContentRect: frame
            styleMask: 0u64      // NSWindowStyleMaskBorderless
            backing: 2u64        // NSBackingStoreBuffered
            defer: false
        ];
        if win.is_null() {
            return None;
        }
        let _: () = msg_send![win, setOpaque: false];
        let clear: *mut Object = msg_send![class!(NSColor), clearColor];
        let _: () = msg_send![win, setBackgroundColor: clear];
        let _: () = msg_send![win, setHasShadow: false];
        let _: () = msg_send![win, setIgnoresMouseEvents: true];
        let _: () = msg_send![win, setReleasedWhenClosed: false];
        // Pattern cua-driver overlay.rs (référence exacte) :
        // NSNormalWindowLevel = 0 (visible dans CGWindowList layer=0),
        // CanJoinAllSpaces | FullScreenAuxiliary | Stationary
        let _: () = msg_send![win, setLevel: 0i64];
        let _: () = msg_send![win, setCollectionBehavior: (1u64 | (1 << 8) | (1 << 4))];
        let _: () = msg_send![win, setSharingType: 1u64];
        let _: () = msg_send![win, setHidesOnDeactivate: false];

        // ---- Layer-backed content view (pattern cua) ----
        let content_view: *mut Object = msg_send![win, contentView];
        let _: () = msg_send![content_view, setWantsLayer: true];
        let layer: *mut Object = msg_send![content_view, layer];

        // ---- Rendu : curseur Sypherine = TON LOGO (PNG embarqué) ----
        let pm = match cursor_pixmap() {
            Some(p) => p,
            None => return None,
        };

        // ---- Pixmap → CGImage (pattern cua pixmap_to_cgimage) ----
        let cg = pixmap_to_cgimage(&pm);
        if let Some(cg_ptr) = cg {
            let cg_id = cg_ptr as *mut Object;
            let _: () = msg_send![layer, setContents: cg_id];
            // Release la CGImage (pattern cua : +1 retained)
            extern "C" { fn CGImageRelease(image: *mut c_void); }
            CGImageRelease(cg_ptr);
        }
        Some(win)
    }
}

/// Affiche le curseur Sypherine (flèche dégradé rose-violet) à la position (x, y).
/// Copie EXACTEMENT le pattern cua-driver (cursor/overlay.rs) :
/// NSWindow transparente borderless → contentView wantsLayer → tiny_skia
/// Pixmap → CGImage (via CGImageCreate) → CALayer.setContents → orderFront.
/// La POINTE de la flèche est à (x, y) — la fenêtre est décalée de 2px.
/// La fenêtre est click-through, se ferme après `ms` ms.
fn show_marker(x: f64, y: f64, ms: u64) {
    use objc::{class, msg_send, runtime::Object, sel, sel_impl};
    use std::os::raw::c_void;

    unsafe {
        // ---- NSApplication obligatoire (pattern cua-driver) : sans app active,
        // les fenêtres NSWindow sont créées mais JAMAIS affichées (pas de run
        // loop). sharedApplication + finishLaunching rend la fenêtre visible.
        let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
        if !app.is_null() {
            let _: () = msg_send![app, setActivationPolicy: 1i64]; // Accessory (pas de Dock)
            let _: () = msg_send![app, finishLaunching];
        }
        // ---- CONVERSION pixels physiques → points NSWindow (correction 12/08) ----
        // x/y arrivent en pixels physiques (espace CGEvent, 3840x2160).
        // NSWindow utilise des points (1600x900) avec l'origine BAS-GAUCHE.
        // Sans cette conversion, la pointe du curseur Sypherine apparaît
        // décalée/hors écran et JAMAIS sur la cible (vu par mon humain).
        use core_graphics::display::CGDisplay;
        let display = CGDisplay::main();
        let dbounds = display.bounds(); // en points
        let h_pt = dbounds.size.height as f64;      // ex: 900
        let w_pt = dbounds.size.width as f64;       // ex: 1600
        let scale_x = display.pixels_wide() as f64 / w_pt; // 3840/1600 = 2.4
        let scale_y = display.pixels_high() as f64 / h_pt; // 2160/900 = 2.4
        let x_pt = x / scale_x;      // pixels → points
        let y_pt = y / scale_y;
        // La POINTE du curseur Sypherine est en HAUT-GAUCHE du pixmap 48x48
        // (offset 2,2). Pour que la pointe soit exactement à (x_pt, y_pt) en
        // haut-gauche : frame.x = x_pt − 2 ; frame.y (origine bas-gauche) =
        // (h_pt − y_pt) − (48 − 2) = h_pt − y_pt − 46. Sans le −46, la pointe
        // apparaît 46 px au-dessus de la cible (vu par mon humain le 12/08).
        let y_ns = h_pt - y_pt - 46.0; // inversion + pointe en haut du pixmap
        // ---- NSWindow borderless transparente (pattern cua) ----
        let allocated: *mut Object = msg_send![class!(NSWindow), alloc];
        // Pointe de la flèche en haut-gauche du pixmap 48x48 → fenêtre décalée
        let frame = NSRect { x: x_pt - 2.0, y: y_ns - 2.0, w: 48.0, h: 48.0 };
        let win: *mut Object = msg_send![allocated,
            initWithContentRect: frame
            styleMask: 0u64      // NSWindowStyleMaskBorderless
            backing: 2u64        // NSBackingStoreBuffered
            defer: false
        ];
        if win.is_null() {
            return;
        }
        let _: () = msg_send![win, setOpaque: false];
        let clear: *mut Object = msg_send![class!(NSColor), clearColor];
        let _: () = msg_send![win, setBackgroundColor: clear];
        let _: () = msg_send![win, setHasShadow: false];
        let _: () = msg_send![win, setIgnoresMouseEvents: true];
        let _: () = msg_send![win, setReleasedWhenClosed: false];
        // Pattern cua-driver overlay.rs (référence exacte) :
        // NSNormalWindowLevel = 0 (visible dans CGWindowList layer=0),
        // CanJoinAllSpaces | FullScreenAuxiliary | Stationary
        let _: () = msg_send![win, setLevel: 0i64];
        let _: () = msg_send![win, setCollectionBehavior: (1u64 | (1 << 8) | (1 << 4))];
        let _: () = msg_send![win, setSharingType: 1u64];
        let _: () = msg_send![win, setHidesOnDeactivate: false];

        // ---- Layer-backed content view (pattern cua) ----
        let content_view: *mut Object = msg_send![win, contentView];
        let _: () = msg_send![content_view, setWantsLayer: true];
        let layer: *mut Object = msg_send![content_view, layer];

        // ---- Rendu : curseur Sypherine = TON LOGO (PNG embarqué) ----
        // Chargé via cursor_pixmap() → Pixmap → CGImage → CALayer.
        // La pointe est en haut-gauche ; fenêtre décalée pour POINTE = cible.
        let pm = match cursor_pixmap() {
            Some(p) => p,
            None => return,
        };

        // ---- Pixmap → CGImage (pattern cua pixmap_to_cgimage) ----
        let cg = pixmap_to_cgimage(&pm);
        if let Some(cg_ptr) = cg {
            // setContents: sur le layer — id (toll-free bridged CGImage)
            let cg_id = cg_ptr as *mut Object;
            let _: () = msg_send![layer, setContents: cg_id];
            let _: () = msg_send![win, orderFrontRegardless];
            // PUMP du run loop — LE pattern cua-driver (overlay.rs) : `[NSApp run]`
            // fait tourner la boucle d'événements pour que la fenêtre se dessine.
            // On ne peut pas appeler run() ici (il bloque), on pompe donc la boucle
            // manuellement pendant `ms` : runMode:beforeDate: traite les événements
            // et laisse Core Animation peindre le layer.
            let run_loop: *mut Object = unsafe { msg_send![class!(NSRunLoop), currentRunLoop] };
            let mode: *mut Object = unsafe { msg_send![class!(NSString),
                stringWithUTF8String: c"kCFRunLoopDefaultMode".as_ptr().cast::<u8>()
            ] };
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(ms);
            while std::time::Instant::now() < deadline {
                // Une frame (16ms) par itération — laisse la boucle tourner
                let until: *mut Object = msg_send![class!(NSDate),
                    dateWithTimeIntervalSinceNow: 0.016
                ];
                let _: bool = msg_send![run_loop, runMode: mode beforeDate: until];
            }
            // Release la CGImage (pattern cua : +1 retained)
            extern "C" { fn CGImageRelease(image: *mut c_void); }
            CGImageRelease(cg_ptr);
        }

        // ---- Fermeture automatique ----
        std::thread::sleep(std::time::Duration::from_millis(ms));
        let _: () = msg_send![win, close];
        let _: () = msg_send![win, release];
        let _: () = msg_send![allocated, release];
    }
}

/// MÉTHODE BATAILLE NAVALE — ANIMATION DU CURSEUR (demande mon humain 12/08)
/// Le curseur Sypherine GLISSE visiblement depuis (from_x, from_y) vers
/// (to_x, to_y) en `steps` étapes (chaque étape pompe le run loop → l'oeil
/// humain VOIT le mouvement), puis reste posé `hold_ms` sur la cible.
/// La POINTE du curseur arrive exactement sur (to_x, to_y).
fn show_cursor_travel(from_x: f64, from_y: f64, to_x: f64, to_y: f64, steps: u32, hold_ms: u64) {
    use objc::{class, msg_send, runtime::Object, sel, sel_impl};
    use std::os::raw::c_void;
    unsafe {
        let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
        if !app.is_null() {
            let _: () = msg_send![app, setActivationPolicy: 1i64];
            let _: () = msg_send![app, finishLaunching];
        }
        use core_graphics::display::CGDisplay;
        let display = CGDisplay::main();
        let dbounds = display.bounds();
        let h_pt = dbounds.size.height as f64;
        let w_pt = dbounds.size.width as f64;
        let scale_x = display.pixels_wide() as f64 / w_pt;
        let scale_y = display.pixels_high() as f64 / h_pt;
        // Conversion pixels physiques → points NSWindow (origine bas-gauche).
        // Pointe du curseur en haut-gauche du pixmap 48x48 (offset 2,2).
        let to_pt = |x: f64, y: f64| -> (f64, f64) {
            let x_pt = x / scale_x;
            let y_pt = y / scale_y;
            (x_pt - 2.0, h_pt - y_pt - 46.0)
        };
        let (fx, fy) = to_pt(from_x, from_y);
        let (tx, ty) = to_pt(to_x, to_y);

        let allocated: *mut Object = msg_send![class!(NSWindow), alloc];
        let frame = NSRect { x: fx, y: fy, w: 48.0, h: 48.0 };
        let win: *mut Object = msg_send![allocated,
            initWithContentRect: frame
            styleMask: 0u64
            backing: 2u64
            defer: false
        ];
        if win.is_null() {
            return;
        }
        let _: () = msg_send![win, setOpaque: false];
        let clear: *mut Object = msg_send![class!(NSColor), clearColor];
        let _: () = msg_send![win, setBackgroundColor: clear];
        let _: () = msg_send![win, setHasShadow: false];
        let _: () = msg_send![win, setIgnoresMouseEvents: true];
        let _: () = msg_send![win, setReleasedWhenClosed: false];
        let _: () = msg_send![win, setLevel: 0i64];
        let _: () = msg_send![win, setCollectionBehavior: (1u64 | (1 << 8) | (1 << 4))];
        let _: () = msg_send![win, setSharingType: 1u64];
        let _: () = msg_send![win, setHidesOnDeactivate: false];

        let content_view: *mut Object = msg_send![win, contentView];
        let _: () = msg_send![content_view, setWantsLayer: true];
        let layer: *mut Object = msg_send![content_view, layer];

        let pm = match cursor_pixmap() {
            Some(p) => p,
            None => return,
        };
        let cg = pixmap_to_cgimage(&pm);
        if let Some(cg_ptr) = cg {
            let cg_id = cg_ptr as *mut Object;
            let _: () = msg_send![layer, setContents: cg_id];
            let _: () = msg_send![win, orderFrontRegardless];

            // PUMP : helper de frame (16 ms)
            let run_loop: *mut Object = msg_send![class!(NSRunLoop), currentRunLoop];
            let mode: *mut Object = msg_send![class!(NSString),
                stringWithUTF8String: c"kCFRunLoopDefaultMode".as_ptr().cast::<u8>()
            ];
            let pump = |ms_frames: u32| {
                for _ in 0..ms_frames {
                    let until: *mut Object = msg_send![class!(NSDate),
                        dateWithTimeIntervalSinceNow: 0.016
                    ];
                    let _: bool = msg_send![run_loop, runMode: mode beforeDate: until];
                }
            };

            // GLISSEMENT visible : interpolation linéaire de (fx,fy) vers (tx,ty)
            let steps = steps.max(2);
            for i in 1..=steps {
                let t = i as f64 / steps as f64;
                let nx = fx + (tx - fx) * t;
                let ny = fy + (ty - fy) * t;
                let pt = NSPoint { x: nx, y: ny };
                let _: () = msg_send![win, setFrameOrigin: pt];
                pump(1); // une frame par étape → mouvement visible
            }
            // Pose sur la cible : maintien visible `hold_ms`
            let frames = (hold_ms as u32 / 16).max(1);
            pump(frames);

            extern "C" { fn CGImageRelease(image: *mut c_void); }
            CGImageRelease(cg_ptr);
        }
        let _: () = msg_send![win, close];
        let _: () = msg_send![win, release];
        let _: () = msg_send![allocated, release];
    }
}

/// CROIX ROSE D'IMPACT — trace de tir visible 5-10s (méthode bataille navale).
/// Après le clic, une croix rose vif (#FF2D78) reste affichée à (x,y) pendant
/// `ms` : mon humain ET mes yeux (capture) peuvent voir OÙ le tir a atterri.
fn show_impact_cross(x: f64, y: f64, ms: u64) {
    use objc::{class, msg_send, runtime::Object, sel, sel_impl};
    use std::os::raw::c_void;
    unsafe {
        let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
        if !app.is_null() {
            let _: () = msg_send![app, setActivationPolicy: 1i64];
            let _: () = msg_send![app, finishLaunching];
        }
        use core_graphics::display::CGDisplay;
        let display = CGDisplay::main();
        let dbounds = display.bounds();
        let h_pt = dbounds.size.height as f64;
        let w_pt = dbounds.size.width as f64;
        let scale_x = display.pixels_wide() as f64 / w_pt;
        let scale_y = display.pixels_high() as f64 / h_pt;
        // Croix 64x64 : le CENTRE de la croix doit être à (x,y).
        let x_pt = x / scale_x;
        let y_pt = y / scale_y;
        let fx = x_pt - 32.0;
        let fy = h_pt - y_pt - 32.0;

        let allocated: *mut Object = msg_send![class!(NSWindow), alloc];
        let frame = NSRect { x: fx, y: fy, w: 64.0, h: 64.0 };
        let win: *mut Object = msg_send![allocated,
            initWithContentRect: frame
            styleMask: 0u64
            backing: 2u64
            defer: false
        ];
        if win.is_null() {
            return;
        }
        let _: () = msg_send![win, setOpaque: false];
        let clear: *mut Object = msg_send![class!(NSColor), clearColor];
        let _: () = msg_send![win, setBackgroundColor: clear];
        let _: () = msg_send![win, setHasShadow: false];
        let _: () = msg_send![win, setIgnoresMouseEvents: true];
        let _: () = msg_send![win, setReleasedWhenClosed: false];
        let _: () = msg_send![win, setLevel: 0i64];
        let _: () = msg_send![win, setCollectionBehavior: (1u64 | (1 << 8) | (1 << 4))];
        let _: () = msg_send![win, setSharingType: 1u64];
        let _: () = msg_send![win, setHidesOnDeactivate: false];

        let content_view: *mut Object = msg_send![win, contentView];
        let _: () = msg_send![content_view, setWantsLayer: true];
        let layer: *mut Object = msg_send![content_view, layer];

        let pm = match cross_pixmap() {
            Some(p) => p,
            None => return,
        };
        let cg = pixmap_to_cgimage(&pm);
        if let Some(cg_ptr) = cg {
            let cg_id = cg_ptr as *mut Object;
            let _: () = msg_send![layer, setContents: cg_id];
            let _: () = msg_send![win, orderFrontRegardless];

            let run_loop: *mut Object = msg_send![class!(NSRunLoop), currentRunLoop];
            let mode: *mut Object = msg_send![class!(NSString),
                stringWithUTF8String: c"kCFRunLoopDefaultMode".as_ptr().cast::<u8>()
            ];
            let frames = (ms as u32 / 16).max(1);
            for _ in 0..frames {
                let until: *mut Object = msg_send![class!(NSDate),
                    dateWithTimeIntervalSinceNow: 0.016
                ];
                let _: bool = msg_send![run_loop, runMode: mode beforeDate: until];
            }
            extern "C" { fn CGImageRelease(image: *mut c_void); }
            CGImageRelease(cg_ptr);
        }
        let _: () = msg_send![win, close];
        let _: () = msg_send![win, release];
        let _: () = msg_send![allocated, release];
    }
}

/// CROIX DE TIR — pixmap 64x64 : une croix ROSE VIF (#FF2D78) épaisse 8px
/// sur fond transparent. C'est la « trace de tir » : après un clic, on
/// l'affiche à l'endroit visé pour que mes yeux (VLM) puissent voir OÙ le tir
/// a atterri par rapport à la grille de calibration (idée de mon humain :
/// comme la marque d'un obus au touché-coulé).
fn cross_pixmap() -> Option<tiny_skia::Pixmap> {
    let mut pm = tiny_skia::Pixmap::new(64, 64)?;
    // Rose vif : #FF2D78 → (255, 45, 120, 255)
    let paint = tiny_skia::Paint {
        anti_alias: true,
        shader: tiny_skia::LinearGradient::new(
            tiny_skia::Point::from_xy(0.0, 0.0),
            tiny_skia::Point::from_xy(64.0, 64.0),
            vec![
                tiny_skia::GradientStop::new(0.0, tiny_skia::Color::from_rgba8(255, 45, 120, 255)),
                tiny_skia::GradientStop::new(1.0, tiny_skia::Color::from_rgba8(255, 90, 200, 255)),
            ],
            tiny_skia::SpreadMode::Pad,
            tiny_skia::Transform::identity(),
        )
        .unwrap_or_else(|| {
            tiny_skia::LinearGradient::new(
                tiny_skia::Point::from_xy(0.0, 0.0),
                tiny_skia::Point::from_xy(1.0, 1.0),
                vec![
                    tiny_skia::GradientStop::new(0.0, tiny_skia::Color::from_rgba8(255, 45, 120, 255)),
                    tiny_skia::GradientStop::new(1.0, tiny_skia::Color::from_rgba8(255, 90, 200, 255)),
                ],
                tiny_skia::SpreadMode::Pad,
                tiny_skia::Transform::identity(),
            ).unwrap()
        }),
        ..Default::default()
    };
    // Barre horizontale : y 28..36 (8px), x 4..60
    let rect_h = tiny_skia::Rect::from_xywh(4.0, 28.0, 56.0, 8.0)?;
    pm.fill_rect(rect_h, &paint, tiny_skia::Transform::identity(), None);
    // Barre verticale : x 28..36, y 4..60
    let rect_v = tiny_skia::Rect::from_xywh(28.0, 4.0, 8.0, 56.0)?;
    pm.fill_rect(rect_v, &paint, tiny_skia::Transform::identity(), None);
    Some(pm)
}

/// Fenêtre overlay 64x64 contenant la CROIX DE TIR (cross_pixmap), click-through,
/// prête à être positionnée. Pattern identique à create_cursor_window.
fn create_cross_window() -> Option<*mut objc::runtime::Object> {
    use objc::{class, msg_send, runtime::Object, sel, sel_impl};
    use std::os::raw::c_void;
    unsafe {
        let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
        if !app.is_null() {
            let _: () = msg_send![app, setActivationPolicy: 1i64]; // Accessory
            let _: () = msg_send![app, finishLaunching];
        }
        let allocated: *mut Object = msg_send![class!(NSWindow), alloc];
        let frame = NSRect { x: 0.0, y: 0.0, w: 64.0, h: 64.0 };
        let win: *mut Object = msg_send![allocated,
            initWithContentRect: frame
            styleMask: 0u64
            backing: 2u64
            defer: false
        ];
        if win.is_null() {
            return None;
        }
        let _: () = msg_send![win, setOpaque: false];
        let clear: *mut Object = msg_send![class!(NSColor), clearColor];
        let _: () = msg_send![win, setBackgroundColor: clear];
        let _: () = msg_send![win, setHasShadow: false];
        let _: () = msg_send![win, setIgnoresMouseEvents: true];
        let _: () = msg_send![win, setReleasedWhenClosed: false];
        let _: () = msg_send![win, setLevel: 0i64];
        let _: () = msg_send![win, setCollectionBehavior: (1u64 | (1 << 8) | (1 << 4))];
        let _: () = msg_send![win, setSharingType: 1u64];
        let _: () = msg_send![win, setHidesOnDeactivate: false];

        let content_view: *mut Object = msg_send![win, contentView];
        let _: () = msg_send![content_view, setWantsLayer: true];
        let layer: *mut Object = msg_send![content_view, layer];

        let pm = match cross_pixmap() {
            Some(p) => p,
            None => return None,
        };
        let cg = pixmap_to_cgimage(&pm);
        if let Some(cg_ptr) = cg {
            let cg_id = cg_ptr as *mut Object;
            let _: () = msg_send![layer, setContents: cg_id];
            extern "C" { fn CGImageRelease(image: *mut c_void); }
            CGImageRelease(cg_ptr);
        }
        Some(win)
    }
}

/// Détecte le centre de la croix ROSE VIF dans un PNG (capture pendant).
/// Rose vif : r>200, g<120, b>140 (le #FF2D78 a r=255, g=45, b=120).
/// Retourne (x, y) = centre de masse des pixels roses.
fn detect_rose_center(png: &[u8]) -> Option<(u32, u32)> {
    let img = image::load_from_memory(png).ok()?;
    let rgb = img.to_rgba8();
    let (w, h) = rgb.dimensions();
    let mut sx: u64 = 0;
    let mut sy: u64 = 0;
    let mut n: u64 = 0;
    for y in 0..h {
        for x in 0..w {
            let px = rgb.get_pixel(x, y);
            let (r, g, b) = (px[0], px[1], px[2]);
            if r > 200 && g < 120 && b > 140 {
                sx += x as u64;
                sy += y as u64;
                n += 1;
            }
        }
    }
    if n == 0 {
        return None;
    }
    Some(((sx / n) as u32, (sy / n) as u32))
}

/// Charge le curseur Sypherine (PNG embarqué via include_bytes!) en Pixmap
/// tiny_skia, prêt pour pixmap_to_cgimage. Le PNG est généré par
/// `cargo run --example make_cursor` (logo → détourage → inclinaison → 72px).
fn cursor_pixmap() -> Option<tiny_skia::Pixmap> {
    let bytes = include_bytes!("../assets/sypherine_cursor.png");
    // Décoder le PNG en RGBA8 avec la crate image
    let img = match image::load_from_memory_with_format(bytes, image::ImageFormat::Png) {
        Ok(i) => i.to_rgba8(),
        Err(_) => return None,
    };
    let (w, h) = img.dimensions();
    let mut pm = match tiny_skia::Pixmap::new(w, h) {
        Some(p) => p,
        None => return None,
    };
    // Copier les pixels RGBA (tiny_skia attend RGBA premultiplié ; pour des
    // pixels alpha non prémultipliés, on convertit : c = c*a/255)
    let data = pm.pixels_mut();
    for y in 0..h {
        for x in 0..w {
            let p = img.get_pixel(x, y);
            let a = p[3] as u32;
            let idx = (y * w + x) as usize;
            if a == 0 {
                data[idx] = tiny_skia::PremultipliedColorU8::TRANSPARENT;
                continue;
            }
            let r = (p[0] as u32 * a / 255) as u8;
            let g = (p[1] as u32 * a / 255) as u8;
            let b = (p[2] as u32 * a / 255) as u8;
            data[idx] = tiny_skia::PremultipliedColorU8::from_rgba(r, g, b, p[3])
                .unwrap_or(tiny_skia::PremultipliedColorU8::TRANSPARENT);
        }
    }
    Some(pm)
}

/// Pixmap tiny_skia (RGBA premultiplied) → CGImage (+1 retained).
/// Copie exacte du pattern cua-driver overlay.rs `pixmap_to_cgimage`.
fn pixmap_to_cgimage(pixmap: &tiny_skia::Pixmap) -> Option<*mut std::ffi::c_void> {
    use std::os::raw::c_void;
    let w = pixmap.width() as usize;
    let h = pixmap.height() as usize;
    if w == 0 || h == 0 {
        return None;
    }
    let data = pixmap.data();
    let bytes_per_row = w * 4;
    // kCGImageAlphaPremultipliedLast | kCGBitmapByteOrder32Big = 0x4001
    const BITMAP_INFO: u32 = 0x0001 | 0x4000;

    unsafe extern "C" fn release_pixel_data(info: *mut c_void, _data: *const c_void, _size: usize) {
        drop(Box::from_raw(info as *mut Vec<u8>));
    }

    unsafe {
        extern "C" {
            fn CGColorSpaceCreateDeviceRGB() -> *mut c_void;
            fn CGColorSpaceRelease(cs: *mut c_void);
            fn CGDataProviderCreateWithData(
                info: *mut c_void,
                data: *const c_void,
                size: usize,
                release_data: Option<unsafe extern "C" fn(*mut c_void, *const c_void, usize)>,
            ) -> *mut c_void;
            fn CGDataProviderRelease(provider: *mut c_void);
            fn CGImageCreate(
                width: usize,
                height: usize,
                bits_per_component: usize,
                bits_per_pixel: usize,
                bytes_per_row: usize,
                color_space: *mut c_void,
                bitmap_info: u32,
                provider: *mut c_void,
                decode: *const f64,
                should_interpolate: bool,
                intent: u32,
            ) -> *mut c_void;
        }

        let copied: Vec<u8> = data.to_vec();
        let len = copied.len();
        let ptr = copied.as_ptr();
        let copied_box: *mut Vec<u8> = Box::into_raw(Box::new(copied));

        let cs = CGColorSpaceCreateDeviceRGB();
        let provider = CGDataProviderCreateWithData(
            copied_box as *mut c_void,
            ptr as *const c_void,
            len,
            Some(release_pixel_data),
        );
        let img = CGImageCreate(
            w, h, 8, 32, bytes_per_row, cs, BITMAP_INFO, provider,
            std::ptr::null(), true, 0,
        );
        // Le provider et le color space sont consommés/retainés par CGImageCreate
        CGDataProviderRelease(provider);
        CGColorSpaceRelease(cs);
        if img.is_null() { None } else { Some(img) }
    }
}

#[repr(C)]
struct NSRect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

#[repr(C)]
struct NSPoint {
    x: f64,
    y: f64,
}

fn mouse_click(
    x: f64,
    y: f64,
    down_type: CGEventType,
    up_type: CGEventType,
    button: CGMouseButton,
    target_pid: Option<i32>,
) -> Result<(), Box<dyn std::error::Error>> {
    use core_graphics::event::{CGEvent, CGEventTapLocation, EventField};
    use core_graphics::display::CGDisplay;

    // ══ FIX macOS (bho3538/osxrdp) : numéro d'événement souris UNIQUE et
    // CROISSANT pour chaque clic — comme les vraies souris. macOS rejette
    // les clics synthétiques avec des numéros dupliqués/absents.
    // ══ FIX 2 (12/08) : le compteur statique repartait de 1000 à CHAQUE
    // invocation du binaire (chaque --clickbg = nouveau process) → macOS
    // voyait 1000,1001,1000,1001... et finissait par rejeter les clics
    // (testé : 3 clics OK puis 0/N). Persistance via /tmp pour des numéros
    // VRAIMENT uniques et croissants à travers les invocations.
    use std::sync::atomic::{AtomicU32, Ordering};
    static MOUSE_EVENT_COUNTER: AtomicU32 = AtomicU32::new(0);
    let compteur_path = "/tmp/ecran-live_mouse_event.txt";
    let base = std::fs::read_to_string(compteur_path)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .unwrap_or(1000);
    let evt_num = (base + MOUSE_EVENT_COUNTER.fetch_add(1, Ordering::SeqCst)) as i64;
    // 2 numéros consommés par clic (primer + vrai clic) → avancer de 2
    let _ = std::fs::write(compteur_path, (base + 2).to_string());

    let src = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .map_err(|e| format!("CGEventSource: {:?}", e))?;
    let pt = core_graphics::geometry::CGPoint::new(x, y);

    // ══ CLIC SANS AUCUN DÉPLACEMENT DU CURSEUR SYSTÈME (12/08, ordre
    // ABSOLU de mon humain : « tu n'y touches pas ») ══
    // Aucun warp. CGAssociateMouseAndMouseCursorPosition(false) désassocie le
    // curseur système : les événements postés sont délivrés à leur position
    // SANS bouger le curseur visuel. Testé 12/08 : le clic down/up posté au
    // tap Session arrive (compteur exercice 4→5) — seul le déplacement du
    // curseur était le problème, désormais impossible.
    unsafe {
        extern "C" {
            fn CGAssociateMouseAndMouseCursorPosition(connected: bool) -> i32;
        }
        CGAssociateMouseAndMouseCursorPosition(false);
    }
    std::thread::sleep(std::time::Duration::from_millis(20));

    // ══ PRIMER CLICK (leçon blog cua "The primer click") ══
    // Chromium/Safari gardent une "user-activation gate" : un clic synthétique
    // sans geste utilisateur récent récent est REFUSÉ (silencieusement !).
    // La solution : un clic décoy à (-1,-1), hors écran, qui tick la gate ;
    // le VRAI clic qui suit est traité comme "trusted continuation".
    // En plus : clickState=1 pour le primer, clickState=2 pour le vrai clic.
    // ══ FIX macOS 27 (bho3538/osxrdp, confirmé end-to-end) ══
    // Les clics synthétiques dans une fenêtre INACTIVE sont délivrés mais
    // n'activent PAS/élèvent PAS la fenêtre. Le fix : donner un NUMÉRO
    // d'événement souris (comme les vraies souris) + poster sur le tap
    // SESSION au lieu de HID. → clic = comportement matériel.
    let primer = core_graphics::geometry::CGPoint::new(-1.0, -1.0);
    let primer_down = CGEvent::new_mouse_event(src.clone(), down_type, primer, button)
        .map_err(|_| "primer down failed")?;
    primer_down.set_integer_value_field(EventField::MOUSE_EVENT_CLICK_STATE, 1);
    primer_down.set_integer_value_field(EventField::MOUSE_EVENT_NUMBER, evt_num);
    match target_pid {
        Some(pid) => {
            unsafe {
                if !skylight::post_to_pid(pid, &*primer_down as *const _ as *mut std::ffi::c_void, true) {
                    primer_down.post_to_pid(pid);
                }
            }
        }
        None => primer_down.post(CGEventTapLocation::Session),
    }
    std::thread::sleep(std::time::Duration::from_millis(8));
    let primer_up = CGEvent::new_mouse_event(src.clone(), up_type, primer, button)
        .map_err(|_| "primer up failed")?;
    primer_up.set_integer_value_field(EventField::MOUSE_EVENT_CLICK_STATE, 1);
    primer_up.set_integer_value_field(EventField::MOUSE_EVENT_NUMBER, evt_num);
    match target_pid {
        Some(pid) => {
            unsafe {
                if !skylight::post_to_pid(pid, &*primer_up as *const _ as *mut std::ffi::c_void, true) {
                    primer_up.post_to_pid(pid);
                }
            }
        }
        None => primer_up.post(CGEventTapLocation::Session),
    }
    std::thread::sleep(std::time::Duration::from_millis(8));

    // Down avec CLICK_STATE=2 (le vrai clic = continuation TRUSTED)
    // + numéro d'événement UNIQUE CROISSANT (fix bho3538)
    let down = CGEvent::new_mouse_event(src.clone(), down_type, pt, button)
        .map_err(|_| "mouse down failed")?;
    down.set_integer_value_field(EventField::MOUSE_EVENT_CLICK_STATE, 2);
    down.set_integer_value_field(EventField::MOUSE_EVENT_NUMBER, evt_num + 1);
    match target_pid {
        Some(pid) => {
            // SKYLIGHT : SLEventPostToPid avec AUTH MESSAGE (trusted pour
            // Safari/Chrome) — LE clavier marche comme ça, la souris aussi !
            // attach_auth_message = true rend l'événement TRUSTED (sinon
            // Safari ignore les clics synthétiques).
            unsafe {
                if !skylight::post_to_pid(pid, &*down as *const _ as *mut std::ffi::c_void, true) {
                    down.post_to_pid(pid);
                }
            }
        }
        None => down.post(CGEventTapLocation::Session),
    }
    std::thread::sleep(std::time::Duration::from_millis(28));

    // Up avec CLICK_STATE=2 + numéro d'événement
    let up = CGEvent::new_mouse_event(src.clone(), up_type, pt, button)
        .map_err(|_| "mouse up failed")?;
    up.set_integer_value_field(EventField::MOUSE_EVENT_CLICK_STATE, 2);
    up.set_integer_value_field(EventField::MOUSE_EVENT_NUMBER, evt_num + 1);
    match target_pid {
        Some(pid) => {
            // SKYLIGHT : SLEventPostToPid avec AUTH MESSAGE (trusted pour
            // Safari/Chrome) — le clavier marche comme ça, la souris aussi.
            unsafe {
                if !skylight::post_to_pid(pid, &*up as *const _ as *mut std::ffi::c_void, true) {
                    up.post_to_pid(pid);
                }
            }
        }
        None => up.post(CGEventTapLocation::Session),
    }
    std::thread::sleep(std::time::Duration::from_millis(40));

    // ══ RÉASSOCIATION OBLIGATOIRE ══
    // Sans elle, la souris de mon humain ne contrôlerait plus le curseur
    // après mon clic. L'association est rétablie immédiatement : je n'ai
    // touché à RIEN de visible, le curseur n'a jamais bougé.
    unsafe {
        extern "C" {
            fn CGAssociateMouseAndMouseCursorPosition(connected: bool) -> i32;
        }
        CGAssociateMouseAndMouseCursorPosition(true);
    }
    Ok(())
}

/// Clic gauche à (x, y).
fn click_at(x: f64, y: f64) -> Result<(), Box<dyn std::error::Error>> {
    use core_graphics::event::{CGMouseButton, CGEventType};
    mouse_click(x, y, CGEventType::LeftMouseDown, CGEventType::LeftMouseUp, CGMouseButton::Left, None)?;
    println!("🖱️ Clic à ({:.0}, {:.0})", x, y);
    show_marker(x, y, 700); // marqueur rose visible (souris auxiliaire)
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
        None,
    )?;
    println!("🖱️ Clic droit à ({:.0}, {:.0})", x, y);
    show_marker(x, y, 700);
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
    show_marker(x, y, 500);
    Ok(())
}

/// Tape du texte au clavier — PATTERN EXACT cua-driver keyboard.rs.
/// Le secret (copié de cua) : keycode 0 + set_string(unicode) → indépendant
/// du layout clavier (AZERTY OK !) + flags NULL (Chrome inspecte les flags,
/// sans ça les majuscules fuient dans le caractère suivant).
/// Source : libs/cua-driver/rust/crates/platform-macos/src/input/keyboard.rs
fn type_text_at(text: &str, target_pid: Option<i32>) -> Result<(), Box<dyn std::error::Error>> {

    use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation};
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

    let src = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .map_err(|e| format!("CGEventSource: {:?}", e))?;

    for ch in text.chars() {
        let ch_str = ch.to_string();
        // Down : keycode 0 (Unicode path), string attachée, flags NULL
        let down = CGEvent::new_keyboard_event(src.clone(), 0, true)
            .map_err(|_| "key down failed")?;
        down.set_string(&ch_str);
        down.set_flags(CGEventFlags::CGEventFlagNull);
        match target_pid {
            Some(pid) => {
                // SKYLIGHT : SLEventPostToPid + auth (TRUSTED pour Safari)
                unsafe {
                    if !skylight::post_to_pid(pid, &*down as *const _ as *mut std::ffi::c_void, true) {
                        down.post_to_pid(pid);
                    }
                }
            }
            None => down.post(CGEventTapLocation::HID),
        }
        std::thread::sleep(std::time::Duration::from_millis(8));

        // Up : même string, flags NULL (sinon fuite de modifieurs)
        let up = CGEvent::new_keyboard_event(src.clone(), 0, false)
            .map_err(|_| "key up failed")?;
        up.set_string(&ch_str);
        up.set_flags(CGEventFlags::CGEventFlagNull);
        match target_pid {
            Some(pid) => {
                unsafe {
                    if !skylight::post_to_pid(pid, &*up as *const _ as *mut std::ffi::c_void, true) {
                        up.post_to_pid(pid);
                    }
                }
            }
            None => up.post(CGEventTapLocation::HID),
        }
        std::thread::sleep(std::time::Duration::from_millis(8));
    }
    println!("⌨️ Tapé {} caractères (Unicode, layout-indépendant)", text.chars().count());
    Ok(())
}

/// Envoie une touche ou combinaison (Escape, Return, Tab, cmd+a, shift+tab...).
/// Mapping keycode + modifieurs copiés de cua-driver keyboard.rs
/// (`key_name_to_code` + `modifier_flags`).
fn key_at(combo: &str) -> Result<(), Box<dyn std::error::Error>> {
    use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation};
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

    // Parse "cmd+a" → modifieurs ["cmd"] + touche "a"
    let parts: Vec<&str> = combo.split('+').collect();
    let key = parts.last().unwrap_or(&"escape").to_lowercase();
    let mut flags = CGEventFlags::CGEventFlagNull;
    for m in &parts[..parts.len().saturating_sub(1)] {
        match m.to_lowercase().as_str() {
            "cmd" | "command" => flags |= CGEventFlags::CGEventFlagCommand,
            "shift" => flags |= CGEventFlags::CGEventFlagShift,
            "option" | "alt" => flags |= CGEventFlags::CGEventFlagAlternate,
            "ctrl" | "control" => flags |= CGEventFlags::CGEventFlagControl,
            "fn" => flags |= CGEventFlags::CGEventFlagSecondaryFn,
            _ => eprintln!("⚠️ modifieur inconnu: « {} »", m),
        }
    }

    // Lettres/chiffres → keycode via un mini-mapping (pattern cua : key_name_to_code
    // utilise le keycode physique pour les touches spéciales, et les lettres via
    // le mapping clavier).
    let code: u16 = match key.as_str() {
        "a" => 0, "s" => 1, "d" => 2, "f" => 3, "h" => 4, "g" => 5, "z" => 6,
        "x" => 7, "c" => 8, "v" => 9, "b" => 11, "q" => 12, "w" => 13, "e" => 14,
        "r" => 15, "y" => 16, "t" => 17, "1" => 18, "2" => 19, "3" => 20, "4" => 21,
        "6" => 22, "5" => 23, "=" => 24, "9" => 25, "7" => 26, "-" => 27, "8" => 28,
        "0" => 29, "]" => 30, "o" => 31, "u" => 32, "[" => 33, "i" => 34, "p" => 35,
        "l" => 37, "j" => 38, "'" => 39, "k" => 40, ";" => 41, "\\" => 42, "," => 43,
        "/" => 44, "n" => 45, "m" => 46, "." => 47, " " => 49,
        "return" | "enter" => 36,
        "tab" => 48,
        "space" => 49,
        "delete" | "backspace" => 51,
        "escape" | "esc" => 53,
        "command" | "cmd" => 55,
        "shift" => 56,
        "capslock" => 57,
        "option" | "alt" => 58,
        "control" | "ctrl" => 59,
        "fn" => 63,
        "home" => 115,
        "pageup" => 116,
        "del" | "forward_delete" => 117,
        "end" => 119,
        "pagedown" => 121,
        "left" | "left_arrow" => 123,
        "right" | "right_arrow" => 124,
        "down" | "down_arrow" => 125,
        "up" | "up_arrow" => 126,
        _ => {
            eprintln!("⚠️ touche inconnue: « {} »", key);
            return Ok(());
        }
    };

    let src = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .map_err(|e| format!("CGEventSource: {:?}", e))?;
    // Mode HID global (var d'env ECRAN_KEY_HID=1) : poste au système entier,
    // pas à un PID précis — nécessaire pour cibler la fenêtre active
    // (ex: minimiser la fenêtre Hermes qui couvre tout l'écran).
    let force_hid = std::env::var("ECRAN_KEY_HID").is_ok();
    let down = CGEvent::new_keyboard_event(src.clone(), code, true)
        .map_err(|_| "key down failed")?;
    down.set_flags(flags);
    if !force_hid {
        if let Some(pid) = pgrep_first("Safari") {
            unsafe {
                if !skylight::post_to_pid(pid, &*down as *const _ as *mut std::ffi::c_void, true) {
                    down.post(CGEventTapLocation::HID);
                }
            }
        } else {
            down.post(CGEventTapLocation::HID);
        }
    } else {
        down.post(CGEventTapLocation::HID);
    }
    std::thread::sleep(std::time::Duration::from_millis(8));
    let up = CGEvent::new_keyboard_event(src.clone(), code, false)
        .map_err(|_| "key up failed")?;
    up.set_flags(flags);
    if !force_hid {
        if let Some(pid) = pgrep_first("Safari") {
            unsafe {
                if !skylight::post_to_pid(pid, &*up as *const _ as *mut std::ffi::c_void, true) {
                    up.post(CGEventTapLocation::HID);
                }
            }
        } else {
            up.post(CGEventTapLocation::HID);
        }
    } else {
        up.post(CGEventTapLocation::HID);
    }
    println!("🔑 Combinaison « {} » (keycode {})", combo, code);
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
