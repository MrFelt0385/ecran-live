// ═══════════════════════════════════════════════════════════════════════════
// palais.rs — PALAIS DE MÉMOIRE SPATIAL (méthode des loci, version virtuelle)
//
// Principe (idée de mon humain) : au lieu de ranger les captures par temps
// (cycles /tmp/analyse/cycle_*_<ts>) ou par sémantique, on les range par
// ESPACE — chaque « pièce » du palais = une case de grille écran (l,c).
// L'index spatial est déterministe (O(1)), zéro hallucination possible sur
// « où était X ? » : on lit l'index au lieu de redemander au VLM.
//
// Structure :
//   ~/palais/l<l>_c<c>/          — captures de la pièce (horodatées)
//   ~/palais/index.json          — index spatial { cases: { "l3_c2": {...} } }
//
// Biais par zone : chaque pièce accumule ses tirs (visé → touché) et apprend
// SON décalage — la géométrie du renderer Safari varie selon la zone écran.
// ═══════════════════════════════════════════════════════════════════════════

use serde_json::json;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Racine du palais (~/palais)
fn racine() -> Result<PathBuf, String> {
    let home = std::env::var("HOME").map_err(|_| "HOME introuvable".to_string())?;
    Ok(PathBuf::from(home).join("palais"))
}

/// Nom de la pièce pour une case (l,c) : l3_c2
pub fn piece(l: u32, c: u32) -> String {
    format!("l{}_c{}", l, c)
}

/// Structure d'une entrée de capture dans l'index
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct CaptureEntry {
    pub fichier: String,
    pub ts: String,
    pub nom: String,
}

/// Structure d'une pièce dans l'index
#[derive(serde::Serialize, serde::Deserialize, Clone, Default)]
pub struct PieceData {
    #[serde(default)]
    pub captures: Vec<CaptureEntry>,
    #[serde(default)]
    pub tirs: u32,
    #[serde(default)]
    pub dx: f64,
    #[serde(default)]
    pub dy: f64,
}

/// Index complet du palais
#[derive(serde::Serialize, serde::Deserialize, Default)]
pub struct PalaisIndex {
    #[serde(default)]
    pub cases: BTreeMap<String, PieceData>,
}

impl PalaisIndex {
    fn chemin(racine: &PathBuf) -> PathBuf {
        racine.join("index.json")
    }

    pub fn charger() -> Result<Self, String> {
        let r = racine()?;
        let p = Self::chemin(&r);
        if !p.exists() {
            return Ok(PalaisIndex::default());
        }
        let contenu = std::fs::read_to_string(&p)
            .map_err(|e| format!("lecture index palais : {}", e))?;
        serde_json::from_str(&contenu).map_err(|e| format!("index palais corrompu : {}", e))
    }

    pub fn sauver(&self) -> Result<(), String> {
        let r = racine()?;
        std::fs::create_dir_all(&r).map_err(|e| format!("création {} : {}", r.display(), e))?;
        let p = Self::chemin(&r);
        let contenu = serde_json::to_string_pretty(self)
            .map_err(|e| format!("encodage index : {}", e))?;
        std::fs::write(&p, contenu).map_err(|e| format!("écriture index : {}", e))?;
        Ok(())
    }
}

/// Ranger une capture dans la pièce (l,c) : copie le fichier dans
/// ~/palais/l<l>_c<c>/ avec un nom horodaté + met à jour l'index.
pub fn ranger(chemin_src: &str, l: u32, c: u32, nom: &str) -> Result<(), String> {
    let r = racine()?;
    let nom_piece = piece(l, c);
    let dir = r.join(&nom_piece);
    std::fs::create_dir_all(&dir).map_err(|e| format!("création pièce : {}", e))?;

    // Horodatage compact pour tri chronologique naturel
    let ts = horodate();
    let ext = PathBuf::from(chemin_src)
        .extension()
        .map(|e| e.to_string_lossy().to_string())
        .unwrap_or_else(|| "png".to_string());
    let dest_name = format!("{}_{}.{}", ts, nom, ext);
    let dest = dir.join(&dest_name);

    std::fs::copy(chemin_src, &dest)
        .map_err(|e| format!("copie {} → {} : {}", chemin_src, dest.display(), e))?;

    // Index
    let mut index = PalaisIndex::charger()?;
    let entree = CaptureEntry {
        fichier: dest_name.clone(),
        ts: ts.clone(),
        nom: nom.to_string(),
    };
    index
        .cases
        .entry(nom_piece.clone())
        .or_default()
        .captures
        .push(entree);
    index.sauver()?;

    println!(
        "🏛️  Palais : {} → ~/palais/{}/{} (ts={})",
        chemin_src, nom_piece, dest_name, ts
    );
    Ok(())
}

/// Chercher : liste les captures d'une pièce (O(1) via l'index).
pub fn chercher(l: u32, c: u32) -> Result<(), String> {
    let index = PalaisIndex::charger()?;
    let nom_piece = piece(l, c);
    match index.cases.get(&nom_piece) {
        Some(piece_data) => {
            println!("🏛️  Pièce {} — {} capture(s), {} tir(s), biais dx={:.1} dy={:.1}",
                     nom_piece, piece_data.captures.len(), piece_data.tirs, piece_data.dx, piece_data.dy);
            for (i, cap) in piece_data.captures.iter().enumerate() {
                println!("  [{}] {}  {}  {}", i, cap.ts, cap.nom, cap.fichier);
            }
            Ok(())
        }
        None => {
            println!("🏛️  Pièce {} — vide", nom_piece);
            Ok(())
        }
    }
}

/// Liste toutes les pièces non vides (vue d'ensemble du palais).
pub fn visiter() -> Result<(), String> {
    let index = PalaisIndex::charger()?;
    if index.cases.is_empty() {
        println!("🏛️  Palais vide — ranger des captures avec --palais ranger");
        return Ok(());
    }
    println!("🏛️  PALAIS DE MÉMOIRE — {} pièce(s) :", index.cases.len());
    for (nom, data) in &index.cases {
        println!(
            "  {} : {} capture(s), {} tir(s), biais dx={:.1} dy={:.1}",
            nom, data.captures.len(), data.tirs, data.dx, data.dy
        );
    }
    Ok(())
}

/// Enregistrer un tir (visé → touché) dans la pièce : moyenne glissante du
/// biais. Le décalage est appris PAR ZONE — chaque pièce a sa propre
/// géométrie (renderer Safari, conversion Y, etc.).
pub fn tir(l: u32, c: u32, vise_x: f64, vise_y: f64, touche_x: f64, touche_y: f64) -> Result<(), String> {
    let mut index = PalaisIndex::charger()?;
    let nom_piece = piece(l, c);
    let dx = touche_x - vise_x;
    let dy = touche_y - vise_y;
    // Copie de l'état courant (évite l'emprunt bloquant index ↔ data)
    let (n, old_dx, old_dy) = {
        let data = index.cases.entry(nom_piece.clone()).or_default();
        (data.tirs, data.dx, data.dy)
    };
    // Moyenne glissante (les tirs récents pèsent plus)
    let alpha = 0.3;
    let (new_n, new_dx, new_dy) = if n == 0 {
        (1, dx, dy)
    } else {
        (n + 1, (1.0 - alpha) * old_dx + alpha * dx, (1.0 - alpha) * old_dy + alpha * dy)
    };
    if let Some(data) = index.cases.get_mut(&nom_piece) {
        data.tirs = new_n;
        data.dx = new_dx;
        data.dy = new_dy;
    }
    index.sauver()?;
    println!(
        "🎯 Tir #{} en {} : visé ({:.0},{:.0}) → touché ({:.0},{:.0}) | décalage dx={:.1} dy={:.1} | biais cumulé dx={:.1} dy={:.1}",
        new_n, nom_piece, vise_x, vise_y, touche_x, touche_y, dx, dy, new_dx, new_dy
    );
    Ok(())
}

/// Purge : garde les N captures les plus récentes de la pièce, supprime les
/// autres (fichiers + index). --garder 0 = tout purger.
pub fn purge(l: u32, c: u32, garder: usize) -> Result<(), String> {
    let r = racine()?;
    let nom_piece = piece(l, c);
    let mut index = PalaisIndex::charger()?;
    let mut a_supprimer: Vec<String> = Vec::new();
    if let Some(data) = index.cases.get_mut(&nom_piece) {
        // Les captures sont triées par horodatage croissant (ordre d'ajout)
        let total = data.captures.len();
        if total > garder {
            let a_garder: Vec<CaptureEntry> = data.captures[total - garder..].to_vec();
            for cap in &data.captures[..total - garder] {
                a_supprimer.push(cap.fichier.clone());
            }
            data.captures = a_garder;
        }
    }
    index.sauver()?;

    // Supprimer les fichiers
    let dir = r.join(&nom_piece);
    let mut supprimes = 0;
    for fichier in &a_supprimer {
        let p = dir.join(fichier);
        if p.exists() {
            let _ = std::fs::remove_file(&p);
            supprimes += 1;
        }
    }
    println!(
        "🧹 Palais {} : {} fichier(s) purgé(s), {} restant(s)",
        nom_piece, supprimes, garder
    );
    Ok(())
}

/// Diff pixel entre deux captures de la même pièce (par index).
/// Retourne (chemin_a, chemin_b) résolus vers les fichiers.
pub fn chemin_capture(l: u32, c: u32, idx_a: usize, idx_b: usize) -> Result<(String, String), String> {
    let r = racine()?;
    let nom_piece = piece(l, c);
    let index = PalaisIndex::charger()?;
    let data = index
        .cases
        .get(&nom_piece)
        .ok_or_else(|| format!("pièce {} vide", nom_piece))?;
    let get = |i: usize| -> Result<String, String> {
        data.captures
            .get(i)
            .map(|cap| r.join(&nom_piece).join(&cap.fichier).to_string_lossy().to_string())
            .ok_or_else(|| format!("index {} hors bornes (max {})", i, data.captures.len()))
    };
    let a = get(idx_a)?;
    let b = get(idx_b)?;
    Ok((a, b))
}

/// Horodatage compact AAAAMMJJ_HHMMSS
fn horodate() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Format lisible approximatif (suffisant pour tri chronologique)
    format!("{}", now)
}

/// Nouvelle capture d'écran → rangée directement dans la pièce
/// (la pièce est connue AVANT la capture : on capture puis on range).
pub fn dernier_chemin() -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    format!("{}/ecran-live.png", home)
}
