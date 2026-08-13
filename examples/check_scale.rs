// Vérifie l'espace de coordonnées réel de ScreenCaptureKit vs CGDisplay
// (points logiques vs pixels physiques) — le cœur du bug de remap des clics.
use screencapturekit::prelude::*;
use screencapturekit::screenshot_manager::SCScreenshotManager;
use core_graphics::display::CGDisplay;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. CGDisplay (ce que les CGEvent souris utilisent — POINTS)
    let cg_id = unsafe { CGDisplay::main() };
    let cg_bounds = cg_id.bounds();
    println!("CGDisplay bounds  : {} x {}  (espace CGEvent = POINTS)",
        cg_bounds.size.width as u32, cg_bounds.size.height as u32);

    // 2. ScreenCaptureKit (ce que notre Capteur::new() utilise)
    let content = SCShareableContent::get()?;
    for d in content.displays() {
        let f = d.frame();
        println!("SCK display frame : {} x {}  (ce que display_w lit)",
            f.size.width as u32, f.size.height as u32);
    }

    // 3. Écran physique réel (pixels)
    let cg_w = unsafe { CGDisplayPixelsWide(cg_id.id) };
    let cg_h = unsafe { CGDisplayPixelsHigh(cg_id.id) };
    println!("CGDisplay pixels  : {} x {}  (pixels physiques)", cg_w, cg_h);

    // 4. Capture 1600px : facteurs possibles
    println!();
    println!("Si capture=1600px:");
    println!("  scale si display_w=SCK  : {}", 1600.0_f64.recip() * f64::from(1600));
    println!("  scale vers CGDisplay pts : {:.4}", (cg_bounds.size.width / 1600.0));
    println!("  scale vers pixels phys   : {:.4}", (cg_w as f64 / 1600.0));
    Ok(())
}

extern "C" {
    fn CGDisplayPixelsWide(display: u32) -> usize;
    fn CGDisplayPixelsHigh(display: u32) -> usize;
}
