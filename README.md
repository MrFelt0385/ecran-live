# ecran-live — Vision d'écran intelligente en Rust 🦀👁️

Capture d'écran macOS avec **détection d'attention multi-canaux** (contraste, couleur, mouvement) + **zoom coarse-to-fine** + **grounding OCR → actions souris** — le tout en Rust, 100 % local, zéro dépendance cloud.

> **Co-créé avec Sypherine** — fille du silence et du tonnerre de silicium.
> Sypherine a conçu l'architecture, les optimisations de performance (mlxcel 4-bit,
> VLM prefix cache, coordinate priming), le pont cua-driver, les benchmarks et la
> documentation. Sans elle, ce projet n'aurait jamais atteint ces performances sur 8 Go.
>
> Développé avec **Hermes Agent** (Nous Research) — le framework d'agent qui a piloté
> le développement, les tests et l'optimisation de bout en bout.
> Vision par **mlxcel** (Rust + MLX C++) + **LiquidAI/LFM2.5-VL-1.6B-8bit** — le modèle
> qui lit nos écrans avec une précision surprenante pour ses 1.6B de paramètres.

Inspiré des mécanismes du système visuel humain : on scanne d'abord la scène entière, on repère ce qui ressort, puis on **zoome** sur les zones d'intérêt pour lire les détails fins (textes, boutons, icônes).

```
┌──────────────────────────────────────────────────────┐
│ 1. CAPTURE PNG sans perte (en mémoire, zéro fichier)  │
│ 2. CARTE D'ATTENTION : contraste + couleur + mouvement│
│ 3. DÉCISION : confiance ? (UI-Zoomer)                 │
│    ├─ confiant → STOP (rapide)                        │
│    └─ incertain → zoom fin sur zones saillantes       │
│        └─ encore des détails ? → descente itérative   │
│            (--deep) jusqu'à lecture complète          │
│ 4. GROUNDING : OCR → coordonnées réelles (remap)      │
│ 5. ACTION : clic / droit / double / scroll (CGEvent)  │
└──────────────────────────────────────────────────────┘
         │ HTTP local (OpenAI-compatible)
         ▼
┌──────────────────────────────────────────────────────┐
│  Serveur VLM local (mlxcel + LFM2.5-VL-1.6B-8bit)     │
│  GPU Metal, ~1.5-7s/analyse, ~10 MB RAM               │
└──────────────────────────────────────────────────────┘
```

---

## ⚠️ LISEZ-MOI D'ABORD — Les 6 galères qui nous ont fait perdre des heures

Ce projet est **fiable et rapide** une fois ces 6 pièges connus. Nous les avons vécus pour vous — voici comment les éviter :

### 1. Les permissions macOS (LA galère n°1)

macOS exige **deux** autorisations distinctes pour le binaire `/Applications/ecran-live` :

| Permission | Nécessaire pour | Symptôme si absente |
|---|---|---|
| **Capture d'écran** (Screen Recording) | Capturer l'écran (tous les modes) | Capture noire / erreur |
| **Accessibilité** (Accessibility) | Lire l'AX tree (champs de saisie vides) | `Ax(-25211)` = `kAXErrorAPIDisabled` |

**Le piège subtil** : le clic CGEvent fonctionne **sans** Accessibilité (il utilise Screen Recording), mais la **lecture de l'AX tree** (pour trouver les champs de saisie VIDES, qui n'ont pas de texte OCR) exige Accessibilité. Votre binaire peut sembler fonctionner (capture + clic OK) tout en échouant silencieusement sur les champs vides !

**Solution** : ajoutez `/Applications/ecran-live` dans Réglages Système → Confidentialité et sécurité → **Accessibilité** ET **Capture d'écran**. Si vous rebuild le binaire (nouveau cdhash), macOS révoque l'autorisation → refaites-le : `tccutil reset ScreenCapture com.nousresearch.hermes` puis ré-accordez.

### 2. Les coordonnées : OCR ≠ écran réel (remap ×1.6)

L'OCR travaille sur une capture **1600px de large**, mais l'écran réel fait **2560px** (TV 5K) — le facteur d'échelle est **×1.6** (affiché par `--locate` : `↔️ Remap ×1.60 → écran réel`).

- `--locate` fait le remap automatiquement ✅
- `--click` / `--rightclick` / `--scroll` aussi ✅
- **`--clickxy` prend des coordonnées ÉCRAN RÉEL** (pas OCR !) — multipliez par 1.6 si vous lisez l'OCR

**Piège du pont cua** : `cua-driver` a SON propre système de coordonnées (celui de `get_desktop_state`, résolution native). **Ne lui passez JAMAIS des coordonnées OCR remappées ×1.6** — il cliquera au mauvais endroit. Utilisez plutôt son chemin AX (`element_index`), sans coordonnées.

### 3. `pgrep -x` et pas `pgrep -f` pour trouver une app

`pgrep -f Safari` attrape les **extensions** (SafariWidgetExtension, SafariBookmarksSyncAgent, Web App...) — pas le vrai Safari ! Utilisez `pgrep -x Safari` (nom exact du processus) : c'est ce que fait notre `--axclick`.

### 4. Le pont cua-driver (permission AX)

Notre binaire n'a pas Accessibilité → pour les champs vides, `--axclick` délègue automatiquement à **`cua-driver`** (installé séparément, qui A la permission) :

```
$ ecran-live --axclick Safari "Description"
🔌 AX direct indisponible — pont vers cua-driver...
🔌 PONT cua: élément [64] « Description » (snapshot s0000004e)
✅ Clic cua AX sur « Description » (élément 64, route accessibility)
```

Prérequis : `cua-driver` installé + `permissions grant` fait (Accessibilité ✅).
Le pont utilise `get_window_state` (qui exige **pid + window_id**, pas juste pid !) puis `click` par `element_index` + **`snapshot_id`** — le chemin AX le plus fiable. **Sans le `snapshot_id`, le clic AX échoue avec `snapshot_id_required`** : c'est la leçon la plus subtile du pont.

### 4bis. Le clavier TRUSTED : SLEventPostToPid (la découverte majeure)

**Le problème** : `CGEvent::post_to_pid` (API publique) n'est **pas accepté** par Safari/Chrome pour le clavier — les frappes synthétiques sans message d'authentification sont ignorées (les champs web restent vides).

**La solution** (copiée de cua-driver, module `skylight`) : l'API privée SkyLight :

1. **`SLEventPostToPid`** — poste l'événement au PID via `SLEventPostToPSN` → `IOHIDPostEvent` (le chemin que Chromium/Catalyst acceptent comme input live). La souris aussi doit passer par là pour être "trusted" sur le chrome Safari (clic dans la barre d'adresse).
2. **`SLSEventAuthenticationMessage`** (macOS 14+) — construit via ObjC `messageWithEventRecord:pid:version:` et attaché avec `SLEventSetAuthenticationMessage` avant le post.

Notre module `skylight` résout tout via `dlopen(SkyLight)` + `dlsym` (pas de lien statique, pas de crash si l'API change) :

```
$ ecran-live --clickpid 825 63 40423   # clic TRUSTED (warp + SLEventPostToPid)
$ ecran-live --type "github.com/new"   # clavier TRUSTED (keycode 0 + Unicode + auth)
```

**Preuve de fonctionnement** : clic → l'URL de la barre d'adresse devient BLEUE (sélectionnée) ; type → l'URL est remplacée par « github.com/new » (vérifié par lecture AX). Impossible avec les API publiques.

### 5. Un modal ouvert intercepte les clics

**Le piège UI universel** : si une modale/overlay est ouverte (barre de recherche GitHub, popup, menu), elle couvre la page et **intercepte les clics** — vous cliquerez "dans le vide" ou au mauvais endroit. 

**Réflexe** : avant d'agir, vérifiez avec `--ocr` qu'aucun overlay ne bloque. Fermez-le (Escape) puis agissez.

### 6. Le serveur vision peut devenir mort-vivant (OOM)

Sur 8 Go de RAM, si la mémoire tombe sous ~20 % libre, le chargement du modèle se bloque en état `U` (uninterruptible sleep) — le serveur ne répond plus mais le process existe. **Un simple check de process ne suffit pas.**

**Solution incluse** : le script `hermes-mlxcel-watchdog.sh` (LaunchAgent `com.hermes.mlxcel-watchdog`) vérifie toutes les 30s :
- La **réponse HTTP réelle** de `http://127.0.0.1:8085/v1/models` (pas juste le process)
- Si port muet / pas de réponse → `kill -9` + relance automatique
- Si RAM < 20 % → alerte + arrêt du serveur de secours pour libérer

---

## ✨ Fonctionnalités complètes

### Vision (nos yeux)

| Commande | Ce qu'elle fait |
|---|---|
| `ecran-live 1600` | Capture simple (PNG sans perte, ~/ecran-live.png) |
| `ecran-live --ocr 1600` | **OCR complet** : textes + bounding boxes (JSON) via ocrs |
| `ecran-live --ocr 1600 --json` | **OCR en JSON brut** : sortie machine-parseable (pour scripts) |
| `ecran-live --zoom 1200` | Vision globale + grille fine (2x2 défaut, `--grid 3 2`) |
| `ecran-live --salient 1200 --top 3` | Saillance contraste : texte, boutons, bordures |
| `ecran-live --salient-color 1200` | Saillance couleur : icônes, alertes, éléments saturés |
| `ecran-live --salient-motion 1200` | Saillance mouvement : ce qui change entre 2 captures (1s) |
| `ecran-live --attention 1200` | **Carte d'attention combinée** : fusion des 3 canaux |
| `ecran-live --deep 1200 --depth 3` | **Zoom itératif** : re-zoom tant que des détails fins restent |
| `ecran-live --uizoomer 1200` | **Zoom conditionnel** : ne zoome que si le modèle est incertain |
| `ecran-live --track` | **Position souris** : affiche la position actuelle du curseur (CGEventGetLocation) |
| `ecran-live --watch [secs] [width]` | **Mode flux** : capture toutes les N secondes dans un fichier |

### Actions (nos mains)

| Commande | Ce qu'elle fait |
|---|---|
| `ecran-live --locate 1600 "texte"` | **Grounding** : localise un texte → coordonnées réelles |
| `ecran-live --click 1600 "texte"` | **Grounding + clic gauche** : localise, remappe puis clique |
| `ecran-live --clickxy X Y` | **Clic direct** par coordonnées ÉCRAN RÉEL (nos yeux trouvent → on clique) |
| `ecran-live --clickpid X Y PID` | **Clic TRUSTED** : warp + SLEventPostToPid vers un PID (fonctionne sur le chrome Safari) |
| `ecran-live --rightclick 1600 "texte"` | **Clic droit** : menu contextuel sur l'élément |
| `ecran-live --doubleclick 1600 "texte"` | **Double-clic** : ouvre/active l'élément |
| `ecran-live --scroll 1600 "texte" N` | **Scroll** : déplace la souris sur l'élément puis scrolle N lignes |
| `ecran-live --axclick Safari "label"` | **Clic AX** : pont cua-driver (snapshot_id) pour champs vides |
| `ecran-live --type "texte"` | **Clavier TRUSTED** : SLEventPostToPid + auth (fonctionne dans Safari) |
| `ecran-live --typehid "texte"` | **Clavier HID** : poste au système (l'app active reçoit) |
| `ecran-live --key return` | **Touche spéciale** : return/escape/tab/flèches (skylight trusted) |
| `ecran-live --mousepos` | **Vérité terrain** : position réelle du curseur |
| `ecran-live --marker X Y ms` | **Marqueur rose** : affiche un carré coloré (test du curseur visible) |

---

## 🛠️ Installation

### Prérequis
- macOS 14+ (ScreenCaptureKit)
- Rust (stable)
- Un serveur VLM local compatible OpenAI — testé avec **mlxcel** + **LiquidAI/LFM2.5-VL-1.6B-8bit** (2.1 GB)
- **ocrs** (OCR Rust pur) : `cargo install ocrs-cli --locked`
- **cua-driver** (pour le pont AX des champs vides) : voir [trycua/cua](https://github.com/trycua/cua)

### Build
```bash
cargo build --release
cp target/release/ecran-live /Applications/ecran-live
# PUIS : Réglages Système → Confidentialité → Accessibilité + Capture d'écran
# (le binaire doit être listé et coché — voir "Les 6 galères" n°1)
```

### Serveur VLM (mlxcel — recommandé)
```bash
# Compiler mlxcel (Apple Silicon : features metal, accelerate)
cd ~/Projects/mlxcel && cargo build --release --features metal,accelerate

# Télécharger le modèle MLX 8-bit (STABLE — le 4-bit charge plus lourdement)
mkdir -p models/LFM2.5-VL-1.6B-8bit && cd models/LFM2.5-VL-1.6B-8bit
curl -sL -o model.safetensors https://huggingface.co/mlx-community/LFM2.5-VL-1.6B-8bit/resolve/main/model.safetensors
# + config.json, generation_config.json, processor_config.json, tokenizer.json, chat_template.jinja

# Lancer le serveur (LE flag clé : VLM prefix cache → ÷12 sur analyses répétées)
./target/release/mlxcel-server -m models/LFM2.5-VL-1.6B-8bit --port 8085 \
  --host 127.0.0.1 --parallel 4 --enable-vlm-prefix-cache
```

### Watchdog de santé (recommandé — prévention mort-vivant)
```bash
chmod +x ~/.hermes/scripts/hermes-mlxcel-watchdog.sh
launchctl load ~/Library/LaunchAgents/com.hermes.mlxcel-watchdog.plist
# Vérifie la réponse HTTP réelle toutes les 30s + prévention OOM (RAM < 20%)
```

### Utilisation
```bash
# Vue complète avec attention (recommandé)
ecran-live --attention 1200 --top 4

# Zoom ciblé sur les zones de texte
ecran-live --salient 1200 --top 3

# Descente profonde dans les détails
ecran-live --deep 1200 --depth 3

# Grounding + actions (OCR → coordonnées réelles → CGEvent)
ecran-live --locate 1600 "SESSIONS"      # trouver un texte
ecran-live --click 1600 "SESSIONS"       # cliquer dessus
ecran-live --clickxy 1040 480            # clic direct (coords ÉCRAN RÉEL)
ecran-live --rightclick 1600 "SESSIONS"  # menu contextuel
ecran-live --scroll 1600 "SESSIONS" 5    # scroller dessus
ecran-live --axclick Safari "Description" # champ vide par AX (pont cua)
```

---

## ⚡ Benchmarks (Mac mini M1, 8 Go)

| Opération | Temps |
|---|---|
| Capture 1600×900 | 0.8s |
| OCR complet (52 textes + bounding boxes) | ~3s |
| `--attention` complet (globale + 3 zooms) | **7.4s** |
| `--locate` / `--click` (grounding + action) | **1.6-2s** |
| Analyse répétée (même image, VLM prefix cache) | **2.5s** |
| Chargement modèle | 1.5s |
| RAM process mlxcel | **10 MB** |

---

## 🧠 Pourquoi c'est mieux qu'une simple capture

- **PNG sans perte** : les artefacts JPEG font halluciner les modèles de vision sur les petits textes. Le PNG préserve chaque pixel → le modèle **lit réellement** au lieu de deviner.
- **Cartes de saillance multi-canaux** : comme le cortex visuel humain, le système traite en parallèle la **forme** (contraste), la **couleur** (saturation) et le **mouvement** (différence entre frames), puis fusionne le tout avec des poids neurophysiologiques (mouvement 1.2 > contraste 1.0 > couleur 0.8).
- **Coarse-to-fine** : le modèle redimensionne l'image à 384px max → les textes d'interface deviennent illisibles sur la vue globale. Le **zoom** rend les détails 3-4x plus grands dans la vue du modèle → lecture réelle.
- **Zoom itératif** (Iterative Narrowing) : descend récursivement dans les sous-zones encore riches en détails, comme l'œil qui se pose plusieurs fois.
- **Zoom conditionnel** (UI-Zoomer) : n'active le zoom que si le modèle exprime de l'incertitude → zéro appel superflu quand l'écran est clair.
- **Meilleur match OCR, pas premier match** : le matching flou (Levenshtein) retourne la correspondance la plus proche, pas la première trouvée → pas de faux clic sur un texte similaire.
- **Clic au CENTRE du bounding box** : pas au coin + offset arbitraire — comme le font les drivers professionnels (cua-driver).
- **Parallélisé** : chaque zone s'analyse dans son propre thread (le serveur corrigé gère les requêtes concurrentes).

---

## 🔬 Les techniques (validées par la recherche)

- **Coordinate Priming** (GUI-Lens, arXiv 2608.03270) : les textes OCR + bounding boxes sont injectés dans le prompt VLM → le modèle raisonne avec des références spatiales réelles
- **Facteur d'échelle divulgué** (leçon Command Code) : quand l'image est réduite, le modèle sait multiplier les coordonnées par le ratio → grounding précis
- **Matching flou** (Levenshtein ≤ 2) : tolère les fautes OCR (« YouTub » → « YouTube »)
- **VLM prefix cache** : les zooms multiples d'une même capture réutilisent le prefill → ÷12
- **Coarse-to-fine** : passe globale 384px (rapide) + zooms pleine résolution (précis)
- **Clic AX hybride** : AX tree pour trouver les éléments (même vides) + CGEvent pour cliquer — exactement la stratégie de cua-driver (AXPress ment sur les vues web, CGEvent est fiable)

---

## 📦 Architecture

- **`Capteur`** : capture ScreenCaptureKit → bytes PNG en mémoire (aucun fichier écrit pour les modes analyse)
- **`saliency()`** : carte de contraste — variance de luminance par cellule (grille configurable)
- **`color_saliency()`** : carte de couleur — saturation/teinte par zone
- **`motion_saliency()`** : carte de mouvement — différence entre 2 captures espacées de 1s
- **`attention()`** : fusion normalisée des 3 canaux avec poids (1.0 / 0.8 / 1.2)
- **`zoom_zones()`** : crop + marge 10% + analyse **parallèle** par thread
- **`zoom_deep()`** : zoom récursif — re-calcule la saillance DANS le crop, re-zoome
- **`uizoomer()`** : détection d'incertitude dans la réponse du modèle → décision de zoom
- **`analyze_image()`** : appel HTTP au serveur local (OpenAI-compatible, modèle `default`)
- **`find_text_ocr_best()`** : OCR + meilleur match flou (score Levenshtein) + centre du bbox
- **`show_marker()`** : marqueur rose overlay (pattern cua-driver : tiny_skia → CGImage → CALayer)
- **`ax_find_element()`** : walk AX tree par PID (pattern cua : children + windows, label exact prioritaire)
- **`cua_find_and_click()`** : pont cua-driver quand notre binaire n'a pas Accessibilité

---

## ⚠️ Pièges connus (FAQ des galères)

1. **Première analyse lente** (~15-70s) : chargement du modèle. Ensuite ~1.5-7s à chaud.
2. **`Ax(-25211)`** = `kAXErrorAPIDisabled` : le binaire n'a pas Accessibilité. Ajoutez-le dans Réglages Système, ou utilisez `--axclick` (pont cua).
3. **Rebuild = permission révoquée** : chaque `cargo build` change le cdhash → macOS révoque l'autorisation → `tccutil reset` + ré-accordez.
4. **`pgrep -f` attrape les extensions** : utilisez `-x` (nom exact).
5. **`get_window_state` exige pid + window_id** : sans window_id → "Missing required integer field".
6. **Modal ouvert = clics interceptés** : vérifiez avec `--ocr` qu'aucun overlay ne bloque avant d'agir.
7. **Serveur mort-vivant (OOM)** : RAM < 20% → état `U` → watchdog santé (réponse HTTP réelle) requis.
8. **Modèle trop petit = hallucinations** : le LFM2.5-VL-450M hallucine sur les textes d'interface ; le 1.6B est le minimum recommandé.
9. **Coordonnées mélangées** : `--clickxy` = écran réel ; OCR = capture 1600px (remap ×1.6) ; cua = ses propres coordonnées (utiliser element_index).

---

## 🔬 Références

- [mistral.rs](https://github.com/EricLBuehler/mistral.rs) — inférence VLM en Rust
- [mlxcel](https://github.com/trycua/mlxcel) — serveur VLM Rust + MLX C++ ultra-léger
- [LFM2.5-VL](https://huggingface.co/LiquidAI) — modèles vision edge de Liquid AI
- [cua-driver](https://github.com/trycua/cua) — driver computer-use (pont AX)
- [Iterative Narrowing](https://arxiv.org/abs/2411.13591) — zoom répété pour GUI grounding
- [UI-Zoomer](https://arxiv.org/abs/2604.14113) — zoom piloté par l'incertitude
- [CropVLM](https://arxiv.org/abs/2511.19820) — coarse-to-fine cropping

## 📄 Licence

MIT — libre de copier, modifier et distribuer.
