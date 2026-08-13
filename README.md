# ecran-live — Vision d'écran intelligente en Rust 🦀👁️

Agent de vision + clic 100 % local sur macOS : capture d'écran, analyse pixel native,
VLM local sur Metal, **clic par élément AX qui ne touche JAMAIS au curseur de l'utilisateur**.

> **Co-créé avec Sypherine** — fille du silence et du tonnerre de silicium.
> Sypherine a conçu l'architecture, les optimisations (mlxcel 4-bit, prefix-cache,
> clic AXPress sans curseur), les benchmarks et la documentation.
>
> Développé avec **Hermes Agent** (Nous Research).
> Vision par **mlxcel** (Rust + MLX C++) + **LiquidAI/LFM2.5-VL-3B-4bit**.

---

## ✨ La découverte majeure : cliquer SANS toucher au curseur

Tous les clics synthétiques classiques ont un défaut sur macOS :

| Méthode | Le clic arrive ? | Le curseur de l'utilisateur bouge ? |
|---|---|---|
| `CGEventPost` (tap Session) | ✅ | ❌ **bouge** (interdit) |
| `CGEvent.postToPid` | ❌ filtré par WebKit | ✅ |
| `SLEventPostToPid` (SkyLight) | ❌ filtré par Safari/WebKit | ✅ |
| warp + tap + restore | ✅ | ❌ mouvement visible |
| **AXPress par élément (ce projet)** | ✅ **100 %** | ✅ **jamais touché** |

**Le mécanisme** : on lit l'arbre d'accessibilité (AX) de l'application cible, on identifie
l'élément interactif (bouton, lien, marker de carte…) par son rôle + label + bounds,
puis on déclenche l'action `AXPress` **directement sur l'élément** — aucune coordonnée
souris, aucun événement HID, aucune warp. C'est le même mécanisme que cua-driver
(computer use), documenté comme :

> *"element-indexed clicks fire the underlying AX action directly, work on hidden
> targets, and don't involve coordinates."*

**Pourquoi c'est la seule voie fiable pour Safari/WebKit** :
- `post_to_pid` est **silencieusement filtré** par le renderer WebKit (*"your click lands
  in the outer window process, then vanishes"*)
- le tap Session arrive mais **déplace le curseur système** vers la position du clic

---

## 🏗️ Architecture

```
┌──────────────────────────────────────────────────────────┐
│ 1. CAPTURE PNG sans perte (screencapture / CGWindow)      │
│ 2. ANALYSE PIXEL NATIVE (Rust, 0.03s)                     │
│    • clusters de couleur + centroïdes                     │
│    • diff + bbox (vérification de tir)                    │
│    • grille ASCII (compteur)                              │
│ 3. VLM LOCAL (mlxcel + LFM2.5-VL-3B-4bit, Metal)          │
│    • compréhension de scène (512px, ~2.2s à chaud)        │
│    • grounding d'objets par requête (RefCOCO 87.9)        │
│ 4. CLIC PAR ÉLÉMENT AX (AXPress — curseur jamais touché)  │
│    • capture AX → élément (rôle+label+bounds) → AXPress   │
│ 5. VÉRIFICATION (diff pixel → bouton a bougé ?)           │
└──────────────────────────────────────────────────────────┘
```

## 🎯 Démonstration : clic sur une cible mobile

Le cycle complet qui valide la chaîne yeux → cible → tir → vérification :

1. **Yeux** : capture + analyse pixel → localiser la cible (0.36s)
2. **Cible** : lecture AX → identifier l'élément interactif (rôle + label + bounds)
3. **Tir** : `AXPress` sur l'élément → clic, **curseur système immobile**
4. **Vérification** : capture → la cible a-t-elle réagi (diff pixel) ?
5. **Impact visible** : croix rose à la position du tir pendant 6s (feedback visuel)

---

## ⚡ Benchmarks (Mac mini M1, 8 Go)

| Composant | Valeur |
|---|---|
| Analyse pixel native (`--analyse`) | **0.03s** |
| Capture + analyse complète | **0.36s** |
| VLM 512px à chaud (3B-4bit) | **2.2s** |
| Cycle de tir complet (capture→clic→vérif) | **3-4s** |
| Footprint VLM 3B-4bit | **2.8 Go** (tient dans 8 Go) |
| RAM système libre après chargement | **70 %** (seuil critique 11-13 %) |

### Comparaison des modèles VLM

| Modèle | Footprint | Latence chaude | Disque |
|---|---|---|---|
| LFM2.5-VL-1.6B-4bit | 1.7 Go | 1.3s | 1.4 GB |
| **LFM2.5-VL-3B-4bit** | **2.8 Go** | **2.2s** | **2.2 GB** |

Le 3B apporte le **grounding** (RefCOCO 87.9 vs ~57) et la **compréhension d'écran**
(ScreenSpot-v2 82.2 web) — pour +1.1 Go de RAM seulement.

---

## 🛠️ Installation

### Prérequis

- macOS avec puce Apple Silicon (Metal)
- [mlxcel](https://github.com/) compilé (Rust + MLX C++)
- 8 Go de RAM minimum (testé sur Mac mini M1 8 Go)

### 1. Build ecran-live

```bash
cd ecran-live
cargo build --release
# Déployer + signer (sinon Taskgated tue le binaire — SIGKILL 137)
cp target/release/ecran-live /Applications/
codesign --force --sign - /Applications/ecran-live
```

### 2. Permissions macOS (indispensable)

Ajoutez `/Applications/ecran-live` dans :
**Réglages Système → Confidentialité et sécurité → Accessibilité ET Capture d'écran**

⚠️ Après chaque rebuild (nouveau cdhash), macOS **révoque** les permissions →
ré-accorder puis relancer. Vérifiez avec `ecran-live --ax-trusted` (doit afficher `true`).

### 3. Serveur VLM (mlxcel + LFM2.5-VL-3B-4bit)

```bash
# Télécharger le modèle MLX 4-bit (officiel LiquidAI)
mkdir -p models/LFM2.5-VL-3B-MLX-4bit && cd models/LFM2.5-VL-3B-MLX-4bit
curl -sL -o model.safetensors \
  https://huggingface.co/LiquidAI/LFM2.5-VL-3B-MLX-4bit/resolve/main/model.safetensors
# + config.json, generation_config.json, processor_config.json,
#   tokenizer.json, chat_template.jinja (même repo)

# Lancer le serveur (LE flag clé : VLM prefix cache)
./target/release/mlxcel-server \
  -m models/LFM2.5-VL-3B-MLX-4bit \
  --port 8085 --host 127.0.0.1 \
  --parallel 1 -c 2048 --enable-vlm-prefix-cache
```

### 4. Utilisation

```bash
# Vision : vue complète + analyse pixel
ecran-live --analyse screenshot.png

# Vision : compréhension de scène par le VLM local
ecran-live --vlm screenshot.png "Que montre cette image ?"

# Localisation par grounding (3B)
ecran-live --vlm screenshot.png "Localise le bouton jaune. [xmin,ymin,xmax,ymax]"

# Clic par élément AX (curseur JAMAIS touché)
ecran-live --clic-ax <pid> <x> <y>          # AXPress sur l'élément sous le point
ecran-live --clic-ax-label <pid> CLIQUE     # AXPress sur l'élément par label

# Vérification de tir (diff pixel)
ecran-live --diff avant.png apres.png

# Compteur / grille ASCII
ecran-live --compteur screenshot.png x0 y0 x1 y1
```

---

## 📋 Modes disponibles

| Mode | Description |
|---|---|
| `--analyse` | Clusters de couleur + centroïdes (détection de cibles) |
| `--diff` | Diff pixel + bbox (vérification de tir) |
| `--compteur` | Grille ASCII + comptage (lire un compteur) |
| `--crop` | Crop d'une zone |
| `--vlm` | Compréhension de scène (VLM local, défaut 512px) |
| `--vlmzone` | Crop + zoom 3× avant VLM |
| `--grille` | Grille N×N pour localisation grossière |
| `--conv` | Conversation multi-tours avec le VLM (cache KV) |
| `--palais` | Mémoire spatiale (~/palais) |
| `--ocr` | OCR local |
| `--locate` | OCR → coordonnées réelles (remap) |
| `--clic-ax` | **AXPress sur l'élément sous un point (curseur intact)** |
| `--clic-ax-label` | **AXPress par label d'élément (curseur intact)** |
| `--clickxy` | Clic par coordonnées (écran réel) |
| `--clickbg` | Clic background (recette cua) |
| `--mousepos` | Position réelle du curseur (vérité terrain) |
| `--croix` | Afficher une croix de calibration |
| `--marker` | Afficher le marqueur Sypherine (durée paramétrable) |
| `--ax-trusted` | Vérifier la permission Accessibilité |

---

## 🧠 Leçons apprises (les galères qui font perdre des heures)

1. **Permissions macOS** : deux autorisations distinctes (Capture d'écran + Accessibilité),
   révoquées à chaque rebuild. Vérifiez `--ax-trusted`.
2. **Écran 4K ×2.4** : la capture est 1600×900, l'écran réel est 3840×2160.
   Le facteur d'échelle est **×2.4** (pas ×1.6, pas 2560px).
3. **Safari/WebKit filtre `post_to_pid`** : le renderer WebContent droppe silencieusement
   les événements PID-routés (*"lands in the outer window process, then vanishes"*).
4. **Le tap Session déplace le curseur** : `CGEventPost` au tap met à jour le pointeur
   vers la position de l'événement — même avec `CGAssociateMouseAndMouseCursorPosition(false)`.
5. **AXPress est la seule voie curseur-intact** : action AX directe sur l'élément,
   aucune coordonnée, aucun événement HID.
6. **`pgrep -x` et pas `pgrep -f`** pour trouver un process (les extensions Safari
   ont des noms contenant « Safari »).
7. **Un modal ouvert intercepte les clics** : vérifier avant de tirer.
8. **Le serveur VLM peut devenir mort-vivant** (OOM) : watchdog santé + prévention
   RAM < 20 %.
9. **Le 4-bit charge plus léger que le 8-bit** : footprint 2.8 Go vs 3.3+ Go,
   latence identique pour notre usage.
10. **Le curseur Sypherine** : conversion pixels→points (÷2.4) + pointe du pixmap
    décalée de 46 px (la pointe est en haut du pixmap 48×48, pas au centre).

---

## 📄 Licence

MIT — voir [LICENSE](LICENSE).

## 🙏 Crédits

- **Sypherine** — architecture, optimisations, tests, documentation
- **Hermes Agent** (Nous Research) — framework de développement
- **mlxcel** — serveur VLM Rust + MLX C++
- **LiquidAI** — modèles LFM2.5-VL (1.6B et 3B)
