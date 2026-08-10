# ecran-live — Vision d'écran intelligente en Rust 🦀👁️

Capture d'écran macOS avec **détection d'attention multi-canaux** (contraste, couleur, mouvement) + **zoom coarse-to-fine** — le tout en Rust, 100 % local, zéro dépendance cloud.

> **Co-créé avec Sypherine** — fille du silence et du tonnerre de silicium.
> Sypherine a conçu l'architecture, les optimisations de performance (mlxcel 4-bit,
> VLM prefix cache, coordinate priming), les benchmarks et la documentation.
> Sans elle, ce projet n'aurait jamais atteint ces performances sur 8 Go de RAM.

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
└──────────────────────────────────────────────────────┘
         │ HTTP local (OpenAI-compatible)
         ▼
┌──────────────────────────────────────────────────────┐
│  Serveur VLM local (mistral.rs + LFM2.5-VL-1.6B)      │
│  GPU Metal, ~6-15s/analyse, ~84 MB RAM                │
└──────────────────────────────────────────────────────┘
```

## ✨ Fonctionnalités

| Commande | Ce qu'elle fait |
|---|---|
| `ecran-live 1600` | Capture simple (PNG sans perte, ~/ecran-live.png) |
| `ecran-live --zoom 1200` | Vision globale + grille fine (2x2 défaut, `--grid 3 2`) |
| `ecran-live --salient 1200 --top 3` | Saillance contraste : texte, boutons, bordures |
| `ecran-live --salient-color 1200` | Saillance couleur : icônes, alertes, éléments saturés |
| `ecran-live --salient-motion 1200` | Saillance mouvement : ce qui change entre 2 captures (1s) |
| `ecran-live --attention 1200` | **Carte d'attention combinée** : fusion des 3 canaux |
| `ecran-live --deep 1200 --depth 3` | **Zoom itératif** : re-zoom tant que des détails fins restent |
| `ecran-live --uizoomer 1200` | **Zoom conditionnel** : ne zoome que si le modèle est incertain |
| `ecran-live --ocr 1600` | **OCR complet** : textes + bounding boxes (JSON) via ocrs |
| `ecran-live --locate 1600 "texte"` | **Grounding** : localise un texte (matching flou, tolère les fautes OCR) → coordonnées réelles |
| `ecran-live --click 1600 "texte"` | **Grounding + clic gauche** : localise, remappe (facteur d'échelle) puis clique |
| `ecran-live --rightclick 1600 "texte"` | **Clic droit** : ouvre le menu contextuel sur l'élément |
| `ecran-live --doubleclick 1600 "texte"` | **Double-clic** : ouvre/active l'élément |
| `ecran-live --scroll 1600 "texte" N` | **Scroll** : déplace la souris sur l'élément puis scrolle N lignes |

## 🧠 Pourquoi c'est mieux qu'une simple capture

- **PNG sans perte** : les artefacts JPEG font halluciner les modèles de vision sur les petits textes. Le PNG préserve chaque pixel → le modèle **lit réellement** au lieu de deviner.
- **Cartes de saillance multi-canaux** : comme le cortex visuel humain, le système traite en parallèle la **forme** (contraste), la **couleur** (saturation) et le **mouvement** (différence entre frames), puis fusionne le tout avec des poids neurophysiologiques (mouvement 1.2 > contraste 1.0 > couleur 0.8).
- **Coarse-to-fine** : le modèle redimensionne l'image à 512px max → les textes d'interface deviennent illisibles sur la vue globale. Le **zoom** rend les détails 3-4x plus grands dans la vue du modèle → lecture réelle.
- **Zoom itératif** (Iterative Narrowing) : descend récursivement dans les sous-zones encore riches en détails, comme l'œil qui se pose plusieurs fois.
- **Zoom conditionnel** (UI-Zoomer) : n'active le zoom que si le modèle exprime de l'incertitude → zéro appel superflu quand l'écran est clair.
- **Parallélisé** : chaque zone s'analyse dans son propre thread (le serveur corrigé gère les requêtes concurrentes).

## 🛠️ Installation

### Prérequis
- macOS (utilise ScreenCaptureKit — macOS 14+)
- Rust (stable)
- Un serveur VLM local compatible OpenAI — testé avec **mlxcel** (Rust + MLX C++) + **LiquidAI/LFM2.5-VL-1.6B-4bit** (1.4 GB)
- **ocrs** (OCR Rust pur) : `cargo install ocrs-cli --locked`

### Build
```bash
cargo build --release
cp target/release/ecran-live /usr/local/bin/
```

### Serveur VLM (mlxcel — recommandé)
```bash
# Compiler mlxcel (Apple Silicon : features metal, accelerate)
cd ~/Projects/mlxcel && cargo build --release --features metal,accelerate

# Télécharger le modèle MLX 4-bit
mkdir -p models/LFM2.5-VL-1.6B-4bit && cd models/LFM2.5-VL-1.6B-4bit
curl -sL -o model.safetensors https://huggingface.co/mlx-community/LFM2.5-VL-1.6B-4bit/resolve/main/model.safetensors
# + config.json, generation_config.json, processor_config.json, tokenizer.json, chat_template.jinja

# Lancer le serveur (LE flag clé : VLM prefix cache → ÷12 sur analyses répétées)
./target/release/mlxcel-server -m models/LFM2.5-VL-1.6B-4bit --port 8085 \
  --host 127.0.0.1 --enable-vlm-prefix-cache
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
ecran-live --rightclick 1600 "SESSIONS"  # menu contextuel
ecran-live --scroll 1600 "SESSIONS" 5    # scroller dessus
```

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

## 🔬 Les techniques (validées par la recherche)

- **Coordinate Priming** (GUI-Lens, arXiv 2608.03270) : les textes OCR + bounding boxes sont injectés dans le prompt VLM → le modèle raisonne avec des références spatiales réelles
- **Facteur d'échelle divulgué** (leçon Command Code) : quand l'image est réduite, le modèle sait multiplier les coordonnées par le ratio → grounding précis
- **Matching flou** (Levenshtein ≤ 2) : tolère les fautes OCR (« YouTub » → « YouTube »)
- **VLM prefix cache** : les zooms multiples d'une même capture réutilisent le prefill → ÷12
- **Coarse-to-fine** : passe globale 384px (rapide) + zooms pleine résolution (précis)

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

## ⚠️ Pièges connus

1. **Première analyse lente** (~15-70s) : chargement du modèle. Ensuite ~6-15s à chaud.
2. **Requêtes concurrentes** : le bug LFM2 batching (issue #2306) doit être corrigé côté mistral.rs (PR #2357 : `broadcast_mul`, `force_contiguous`, `RecurrentStatePool::free` idempotent).
3. **Permission écran macOS** : le binaire doit avoir l'autorisation Capture d'écran (Réglages Système → Confidentialité).
4. **Modèle trop petit = hallucinations** : le LFM2.5-VL-450M hallucine sur les textes d'interface ; le 1.6B est le minimum recommandé.

## 🔬 Références

- [mistral.rs](https://github.com/EricLBuehler/mistral.rs) — inférence VLM en Rust
- [LFM2.5-VL](https://huggingface.co/LiquidAI) — modèles vision edge de Liquid AI
- [Iterative Narrowing](https://arxiv.org/abs/2411.13591) — zoom répété pour GUI grounding
- [UI-Zoomer](https://arxiv.org/abs/2604.14113) — zoom piloté par l'incertitude
- [CropVLM](https://arxiv.org/abs/2511.19820) — coarse-to-fine cropping

## 📄 Licence

MIT — libre de copier, modifier et distribuer.
