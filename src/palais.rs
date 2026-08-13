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
    /// Index inversé biomimétique (méthode LOCI / activation diffuse) :
    /// nom → liste de pièces qui contiennent ce nom. Le cerveau retrouve
    /// un souvenir par SES ASSOCIATIONS (le nom active les lieux), pas en
    /// scannant tout le palais. Un même nom peut vivre dans plusieurs pièces
    /// (blob-like : plusieurs chemins d'accès vers la même donnée).
    #[serde(default)]
    pub associations: BTreeMap<String, Vec<String>>,
    /// Mémoire de travail (Miller 1956 : 7±2 items) — les dernières pièces
    /// consultées, gardées en tête pour un accès instantané aux données
    /// récentes (hot cache, chemins d'accès courts).
    #[serde(default)]
    pub travail: Vec<String>,
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

    /// Mémoire de travail : marque une pièce comme récemment consultée
    /// (le cerveau garde 7±2 items actifs — les données chaudes sont
    /// retrouvées par le chemin le plus court).
    pub fn noter_travail(&mut self, nom_piece: &str) {
        if let Some(pos) = self.travail.iter().position(|p| p == nom_piece) {
            self.travail.remove(pos);
        }
        self.travail.push(nom_piece.to_string());
        const MAX_TRAVAIL: usize = 7; // Miller 1956 : 7±2
        if self.travail.len() > MAX_TRAVAIL {
            self.travail.remove(0);
        }
    }

    /// Lien associatif : nom → pièce (l'index inversé pour retrouver par
    /// sémantique, pas seulement par position spatiale).
    pub fn associer(&mut self, nom: &str, nom_piece: &str) {
        if nom.is_empty() || nom == "capture" {
            return; // nom par défaut inutile — pas d'association à créer
        }
        let pieces = self.associations.entry(nom.to_string()).or_default();
        if !pieces.contains(&nom_piece.to_string()) {
            pieces.push(nom_piece.to_string());
        }
    }

    /// Activation diffuse : à partir d'un nom, récupère TOUTES les pièces
    /// qui le contiennent (plusieurs chemins d'accès = blob-like) + les
    /// pièces récentes si aucune association exacte (proximité temporelle).
    pub fn activer(&self, nom: &str) -> Vec<String> {
        if let Some(pieces) = self.associations.get(nom) {
            return pieces.clone();
        }
        // Recherche partielle (préfixe/contient) — comme le cerveau qui
        // reconnaît un souvenir par fragments.
        let mut resultats: Vec<String> = Vec::new();
        for (cle, pieces) in &self.associations {
            if cle.contains(nom) || nom.contains(cle) {
                resultats.extend(pieces.iter().cloned());
            }
        }
        resultats
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
    // Méthode LOCI : on crée les associations (nom → pièce) pour retrouver
    // par activation diffuse, et on marque la pièce en mémoire de travail.
    index.associer(nom, &nom_piece);
    index.noter_travail(&nom_piece);
    index.sauver()?;

    println!(
        "🏛️  Palais : {} → ~/palais/{}/{} (ts={})",
        chemin_src, nom_piece, dest_name, ts
    );
    Ok(())
}

/// Chercher : liste les captures d'une pièce (O(1) via l'index).
/// Noté en mémoire de travail → l'accès suivant sera instantané.
pub fn chercher(l: u32, c: u32) -> Result<(), String> {
    let mut index = PalaisIndex::charger()?;
    let nom_piece = piece(l, c);
    index.noter_travail(&nom_piece);
    index.sauver()?;
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

/// Retrouver par ASSOCIATION (méthode LOCI / activation diffuse) : à partir
/// d'un nom, le cerveau active les pièces qui le contiennent — plusieurs
/// chemins d'accès (blob-like) au lieu d'une seule position spatiale.
/// Affiche les chemins les plus courts d'abord (mémoire de travail).
pub fn retrouver(nom: &str) -> Result<(), String> {
    let index = PalaisIndex::charger()?;
    let pieces = index.activer(nom);
    if pieces.is_empty() {
        println!("🏛️  Aucun souvenir pour « {} » — essayez un nom partiel (ex: --palais retrouver bouton)", nom);
        return Ok(());
    }
    println!("🏛️  ACTIVATION DIFFUSE — « {} » → {} chemin(s) d'accès :", nom, pieces.len());
    // Les pièces en mémoire de travail d'abord (chemin le plus court)
    let mut ordre: Vec<String> = pieces.clone();
    ordre.sort_by_key(|p| {
        let travail_pos = index.travail.iter().position(|t| t == p);
        match travail_pos {
            Some(pos) => 0usize.saturating_sub(pos), // récent = prioritaire
            None => usize::MAX,
        }
    });
    for (i, nom_piece) in ordre.iter().enumerate() {
        let data = index.cases.get(nom_piece);
        let (n, dx, dy) = match data {
            Some(d) => (d.captures.len(), d.dx, d.dy),
            None => (0, 0.0, 0.0),
        };
        let travail = if index.travail.contains(nom_piece) { " 🔥 mémoire de travail" } else { "" };
        println!("  [{}] {} — {} capture(s), biais dx={:.1} dy={:.1}{}", i, nom_piece, n, dx, dy, travail);
    }
    Ok(())
}

/// Accès public à l'index (pour le sous-commande `travail`)
pub fn charger_index() -> Result<PalaisIndex, String> {
    PalaisIndex::charger()
}

/// Affiche la mémoire de travail (Miller 1956 : 7±2 items récents)
pub fn afficher_travail(index: &PalaisIndex) -> Result<(), String> {
    if index.travail.is_empty() {
        println!("🏛️  Mémoire de travail vide — consultez des pièces (chercher) pour la remplir");
        return Ok(());
    }
    println!("🏛️  MÉMOIRE DE TRAVAIL (Miller 1956 : 7±2) — {} pièce(s) chaude(s) :", index.travail.len());
    for (i, nom_piece) in index.travail.iter().enumerate() {
        println!("  [{}] {} (récent {})", i, nom_piece, index.travail.len() - i);
    }
    Ok(())
}

/// SOMMEIL DU PALAIS (consolidation ADN, biomimétique) : pendant l'inactivité,
/// le cerveau rejoue les souvenirs pour les consolider en mémoire compacte
/// (hippocampe → cortex). Ici : chaque capture PNG (~1 Mo) est remplacée par
/// son empreinte perceptive (~2 Ko) + le nom → le palais devient ~500× plus
/// léger, chargement instantané, RAM quasi nulle.
pub fn sommeil() -> Result<(), String> {
    let r = racine()?;
    let mut index = PalaisIndex::charger()?;
    let mut compactes = 0u64;
    let mut octets_avant: u64 = 0;
    let mut octets_apres: u64 = 0;

    // Crée le dossier consolidé (l'ADN compacté)
    let consolide = r.join("consolide");
    std::fs::create_dir_all(&consolide).map_err(|e| format!("création consolide : {}", e))?;

    for (nom_piece, data) in index.cases.iter_mut() {
        let dir = r.join(nom_piece);
        let mut nouvelles_captures: Vec<CaptureEntry> = Vec::new();
        for cap in &data.captures {
            let src = dir.join(&cap.fichier);
            let taille = std::fs::metadata(&src).map(|m| m.len()).unwrap_or(0);
            octets_avant += taille;
            // Empreinte perceptive = la "séquence génétique" du souvenir
            let bytes = match std::fs::read(&src) {
                Ok(b) => b,
                Err(_) => continue,
            };
            let empr = match empreinte_from_bytes(&bytes) {
                Ok(e) => e,
                Err(_) => continue,
            };
            let dest = consolide.join(format!("{}_{}.fpt", nom_piece, &cap.ts));
            std::fs::write(&dest, &empr).map_err(|e| format!("écriture empreinte : {}", e))?;
            octets_apres += empr.len() as u64;
            nouvelles_captures.push(CaptureEntry {
                fichier: format!("{}_{}.fpt", nom_piece, &cap.ts),
                ts: cap.ts.clone(),
                nom: cap.nom.clone(),
            });
            compactes += 1;
        }
        data.captures = nouvelles_captures;
    }

    index.sauver()?;
    println!("💤  SOMMEIL DU PALAIS terminé — {} souvenir(s) consolidé(s) :", compactes);
    println!("    octets avant : {} ({:.1} Mo)", octets_avant, octets_avant as f64 / 1e6);
    println!("    octets après : {} ({:.1} Ko) — ratio {:.0}× plus léger", octets_apres, octets_apres as f64 / 1e3,
        if octets_apres > 0 { octets_avant as f64 / octets_apres as f64 } else { 0.0 });
    println!("    L'index pointe maintenant vers des empreintes (~2 Ko) au lieu des PNG.");
    Ok(())
}

/// Empreinte perceptive d'une image en mémoire (64x36 luminance) — réutilise
/// le même principe que fingerprint() mais sans dépendre de main.rs.
fn empreinte_from_bytes(png_bytes: &[u8]) -> Result<Vec<u8>, String> {
    let img = image::load_from_memory(png_bytes).map_err(|e| format!("image: {e}"))?;
    let luma = img.to_luma8();
    let (iw, ih) = luma.dimensions();
    // Réduit à 64x36
    let (rw, rh) = (64u32, 36u32);
    let mut empr = Vec::with_capacity((rw * rh) as usize);
    for y in 0..rh {
        for x in 0..rw {
            let sx = (x * iw / rw).min(iw - 1);
            let sy = (y * ih / rh).min(ih - 1);
            empr.push(luma.get_pixel(sx, sy)[0]);
        }
    }
    Ok(empr)
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
