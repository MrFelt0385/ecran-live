# ecran-live — Vision d'écran intelligente en Rust 🦀👁️

**Le premier agent de vision d'écran 100 % local pour macOS** : un pipeline de
perception complet — analyse pixel native, attention visuelle, cartes de
saillance, vision continue en direct, compréhension de scène par VLM local —
avec, en accessoire, la capacité d'agir (clic par élément AX qui ne touche
jamais au curseur).

> **Co-créé avec Sypherine** — fille du silence et du tonnerre de silicium.
> Sypherine a conçu l'architecture de vision, les optimisations (mlxcel 4-bit,
> prefix-cache, vision directe), les benchmarks et la documentation.
>
> Développé avec **Hermes Agent** (Nous Research).
> Vision par **mlxcel** (Rust + MLX C++) + **LiquidAI/LFM2.5-VL-3B-4bit**.

---

## 👁️ LA VISION — le cœur du projet

`ecran-live` est avant tout un **système de perception d'écran**. Il imite le
système visuel humain : on scanne la scène entière, on repère ce qui ressort
(contraste, couleur, mouvement), puis on **zoome** sur les zones d'intérêt pour
lire les détails fins.

```
┌──────────────────────────────────────────────────────────┐
│ 1. CAPTURE PNG sans perte (en mémoire, zéro fichier)      │
│ 2. ANALYSE PIXEL NATIVE (Rust, 0.023s)                    │
│    • clusters de couleur + centroïdes                     │
│    • diff + bbox (détection de changement)                │
│    • cartes d'attention (contraste + couleur + mouvement) │
│ 3. VISION DIRECTE CONTINUE (--stream)                     │
│    • buffer tournant : 1 image en mémoire à la fois       │
│    • détection de changement (empreinte 64px)             │
│    • VLM sur images réduites 384px (prefill ÷10)          │
│ 4. ANALYSE PARALLÈLE DES ZONES SAILLANTES (--deep)        │
│    • découpage en zones → threads indépendants → VLM      │
│    • collecte ordonnée des résultats                      │
│ 5. COMPRÉHENSION DE SCÈNE (--vlm / --conv / --palais)     │
│    • VLM local 512px (0.51s à chaud, max_tokens 24)       │
│    • grounding d'objets par requête (RefCOCO 87.9)        │
│    • mémoire spatiale persistante (~/palais)              │
└──────────────────────────────────────────────────────────┘
```

### Les 5 piliers de la vision

| Pilier | Mode | Description |
|---|---|---|
| **Analyse pixel native** | `--analyse` | Clusters de couleur, centroïdes, détection de cibles — **0.023s** |
| **Détection de changement** | `--diff` | Diff pixel + bbox — savoir ce qui a bougé à l'écran |
| **Attention visuelle** | `--attention` | Carte d'attention (contraste + couleur + mouvement) |
| **Saillance** | `--salient` | Zones saillantes par couleur / mouvement, zoom 2x2 |
| **Vision directe continue** | `--stream` | **Les yeux toujours ouverts** — flux en direct |

### 🔴 Vision directe continue (`--stream`) — la vitrine

Le mode vision en direct : **les yeux toujours ouverts**, qui regardent l'écran
en continu et décrivent ce qui se passe.

```bash
ecran-live --stream 2 1600   # 2 fps, capture 1600px
```

- **Buffer tournant** : UNE seule image en mémoire à la fois (crucial sur 8 Go)
- **Détection de changement** : on n'analyse QUE si l'écran a bougé (empreinte
  perceptive 64px, seuil ≥1 % pour ignorer les micro-changements)
- **Rate-limit intelligent** : au plus 1 analyse VLM toutes les 2s (évite la
  saturation de mlxcel → freeze système)
- **Images réduites 384px** : prefill VLM ÷10, le prefix-cache accélère encore
- **Zéro fichier écrit** : tout en mémoire

### ⚡ Analyse parallèle des zones saillantes (`--deep`)

Quand une zone mérite l'attention, `--deep` découpe l'écran en zones, lance un
**thread VLM par zone** (`std::thread::spawn`), puis collecte les résultats
dans l'ordre — la perception multi-cibles en parallèle.

```bash
ecran-live --deep 3 2   # grille 3x2, 6 zones analysées en parallèle
```

### 🧠 Compréhension de scène

- `--vlm` — description de scène (512px, 0.51s à chaud avec le 3B-4bit-vq, max_tokens 24)
- `--scan` — **boucle rapide** : saillance pixels (0.02s) → crops réduits 128px → VLM court (~1.2s total)
- `--conv` — conversation multi-tours avec le VLM (cache KV)
- `--palais` — **mémoire spatiale persistante** : le modèle se souvient de ce
  qu'il a vu et où (références croisées entre sessions)
- `--grille` — localisation grossière N×N d'un élément
- `--vlmzone` — crop + zoom 3× avant analyse VLM (lire les détails fins)
- `--ocr` / `--locate` — lecture de texte + remap vers coordonnées réelles

---

## 🖱️ L'ACTION — clic sans jamais toucher au curseur

Accessoire de la vision : une fois qu'on a **vu**, on peut **agir**. Notre clic
par élément AX est le seul qui ne touche JAMAIS au curseur de l'utilisateur.

| Méthode | Le clic arrive ? | Le curseur bouge ? |
|---|---|---|
| `CGEventPost` (tap Session) | ✅ | ❌ **bouge** (interdit) |
| `CGEvent.postToPid` | ❌ filtré par WebKit | ✅ |
| `SLEventPostToPid` (SkyLight) | ❌ filtré par Safari/WebKit | ✅ |
| warp + tap + restore | ✅ | ❌ mouvement visible |
| **AXPress par élément (ce projet)** | ✅ **100 %** | ✅ **jamais touché** |

**Le mécanisme** : on lit l'arbre d'accessibilité (AX), on identifie l'élément
interactif par son rôle + label + bounds, puis on déclenche `AXPress`
**directement sur l'élément** — aucune coordonnée souris, aucun événement HID.

> *"element-indexed clicks fire the underlying AX action directly, work on
> hidden targets, and don't involve coordinates."* — cua-driver

**Pourquoi c'est la seule voie fiable pour Safari/WebKit** :
- `post_to_pid` est silencieusement filtré par le renderer WebKit
- le tap Session arrive mais déplace le curseur système

---

## ⚡ Benchmarks (Mac mini M1, 8 Go — mesurés 13/08)

| Composant | Valeur |
|---|---|
| **Analyse pixel native** | **0.023s** |
| **VLM 512px à chaud (3B-4bit-vq, max_tokens 24)** | **0.51s** |
| **VLM sur image répétée (vision cache LFM2-VL)** | **0.55s** ✨ 2.12× |
| **VLM sur crop ciblé (120×70)** | **1.0s** |
| **VLM sur image complexe (crop 128px)** | **1.25s** |
| Cycle vision complet (capture→analyse→compréhension) | **~1.3s** |
| Footprint VLM 3B-4bit-vq (vision tower quantifié) | **2.3 Go** (tient dans 8 Go) |
| RAM système libre après chargement | **64 %** (seuil critique 11-13 %) |

> **Leçon performance (13/08)** : le VLM génère à ~30 tok/s. Limiter
> `max_tokens` (24 par défaut) fait tomber la latence de 6s à ~1s sans perte
> pour les réponses courtes (oui/non, nombre, mot). Analyser des **zones
> ciblées** (crop) plutôt que l'écran entier divise encore le temps — le
> pipeline `--scan` : pixels (0.02s) → crops réduits → VLM court = **~1.2s**.

### ✨ Vision Feature Cache (contribution à mlxcel)

Nous avons **implémenté le cache de features vision pour LFM2-VL** dans mlxcel
(le code officiel ne l'avait que pour Qwen/Gemma/Granite) :

- **Principe** : les features du vision tower (encodeur d'image) sont hashées
  (SHA-256 + budget soft-tokens) et stockées. Quand la **même image** revient
  (conversation multi-tours, vision continue), le serveur **saute le vision
  tower + connecteur** et réutilise les features.
- **Gain mesuré** : 1158ms → 547ms (**2.12×**) sur image répétée, finesse
  identique (features déterministes, réponses strictement égales).
- **Fichiers** : `src/vision/lfm2_vl.rs` (méthode `get_input_embeddings_with_cache`,
  pattern Qwen2.5-VL) + `src/multimodal/vlm_runtime.rs` (activation du cache
  pour `VlmRuntimeRef::Lfm2Vl`).

C'est exactement le cas d'usage de la **vision continue** : les écrans stables
(HUD, fenêtres fixes) sont ré-analysés en **0.55s** au lieu de **1.16s**.

### Comparaison des modèles VLM

| Modèle | Footprint | Latence chaude | Disque |
|---|---|---|---|
| LFM2.5-VL-1.6B-4bit | 1.7 Go | 1.3s | 1.4 GB |
| **LFM2.5-VL-3B-4bit-vq** | **2.3 Go** | **0.51s** | **1.97 GB** |

Le 3B apporte le **grounding** (RefCOCO 87.9) et la **compréhension d'écran**
(ScreenSpot-v2 82.2 web) — pour +0.6 Go de RAM seulement (vs 1.1 Go avec le
vision tower fp16).

> **✨ Vision tower quantifié (13/08)** : le modèle 4-bit officiel laisse le
> vision tower SigLIP2 400M en fp16 (0.83 GB, 35% du modèle !). En le
> quantifiant aussi en 4-bit (gs64), le footprint passe de 2.8 → **2.3 Go**
> et la latence de ~1s → **0.51s** (déquant GPU natif) — **finesse identique**
> (réponses strictement égales). Script + pièges dans
> [`docs/QUANT_VISION_TOWER.md`](docs/QUANT_VISION_TOWER.md).

> **Référence quantification** : AWQ (Activation-aware Weight Quantization),
> *MLSys 2024 Best Paper* — MIT HAN Lab, Tsinghua, MIT-IBM Watson AI Lab.
> Protéger 1 % des poids saillants (identifiés par les activations, pas les
> poids) réduit fortement l'erreur de quantification, y compris pour les
> modèles multi-modaux. [arXiv:2306.00978](https://arxiv.org/abs/2306.00978)

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
ré-accorder puis relancer. Vérifiez avec `ecran-live --ax-trusted`.

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
  --parallel 2 -c 2048 --enable-vlm-prefix-cache \
  --kv-quant-scheme turboquant --kv-bits 4
```

### 4. Utilisation — la vision d'abord

```bash
# VISION : vue complète avec attention (recommandé)
ecran-live --attention screenshot.png

# VISION : zoom ciblé sur les zones de texte
ecran-live --zoom 2 2 screenshot.png

# VISION : descente profonde (zones saillantes en parallèle)
ecran-live --deep 3 2 screenshot.png

# VISION : compréhension de scène par le VLM local
ecran-live --vlm screenshot.png "Que montre cette image ?"

# VISION : localisation par grounding (3B)
ecran-live --vlm screenshot.png "Localise le bouton jaune. [xmin,ymin,xmax,ymax]"

# VISION : boucle rapide (saillance → crops → VLM, ~1.2s)
ecran-live --scan 1600 2 "Que voit-on ?"

# VISION : les yeux toujours ouverts
ecran-live --stream 2 1600

# ACTION : clic par élément AX (curseur JAMAIS touché)
ecran-live --clic-ax <pid> <x> <y>
ecran-live --clic-ax-label <pid> CLIQUE
```

---

## 📋 Modes disponibles

### Vision & analyse
| Mode | Description |
|---|---|
| `--analyse` | Clusters de couleur + centroïdes (détection de cibles) |
| `--diff` | Diff pixel + bbox (détection de changement) |
| `--compteur` | Grille ASCII + comptage (lire un compteur) |
| `--crop` | Crop d'une zone |
| `--attention` | Carte d'attention (contraste + couleur + mouvement) |
| `--salient` | Zones saillantes (couleur / mouvement) + zoom |
| `--zoom` | Grille de zoom 2x2 (configurable) |
| `--deep` | Descente itérative, **zones en parallèle** |
| `--uizoomer` | Zoom sur les zones d'interface |
| `--ocr` | OCR local |
| `--locate` | OCR → coordonnées réelles (remap) |

### Vision VLM (local, Metal)
| Mode | Description |
|---|---|
| `--vlm` | Compréhension de scène (défaut 512px, max_tokens 24 → 0.51s) |
| `--scan` | **Boucle rapide** : saillance pixels → crops réduits 128px → VLM (~1.2s) |
| `--conv` | Conversation multi-tours (cache KV) |
| `--vlmzone` | Crop + zoom 3× avant VLM |
| `--grille` | Grille N×N pour localisation grossière |
| `--palais` | Mémoire spatiale persistante (~/palais) |
| `--stream` | **Vision directe continue** (yeux toujours ouverts) |

### Action (accessoire de la vision)
| Mode | Description |
|---|---|
| `--clic-ax` | AXPress sur l'élément sous un point (curseur intact) |
| `--clic-ax-label` | AXPress par label d'élément (curseur intact) |
| `--clickxy` | Clic par coordonnées (écran réel) |
| `--clickbg` | Clic background (recette cua) |
| `--mousepos` | Position réelle du curseur (vérité terrain) |
| `--marker` | Marqueur Sypherine (durée paramétrable) |
| `--ax-trusted` | Vérifier la permission Accessibilité |

---

## 🧠 Leçons apprises (les galères qui font perdre des heures)

1. **Permissions macOS** : deux autorisations distinctes (Capture d'écran +
   Accessibilité), révoquées à chaque rebuild.
2. **Écran 4K ×2.4** : la capture est 1600×900, l'écran réel est 3840×2160.
   Le facteur d'échelle est **×2.4** (pas ×1.6, pas 2560px).
3. **Le VLM doit être rate-limité** : au plus 1 analyse toutes les 2s en
   vision continue, sinon mlxcel se sature → freeze système.
4. **L'empreinte perceptive 64px** : détecter le changement AVANT d'analyser
   évite de ré-analyser le vide (le monde est souvent statique).
5. **Images ≤512px pour le VLM** : le prefill est ÷10 à 384px, la fiabilité
   est identique (testé 3/3).
6. **Le prefix-cache VLM** : accélère les conversations multi-tours (images
   voisines partagent le préfixe).
7. **Safari/WebKit filtre `post_to_pid`** : le renderer droppe silencieusement
   les événements PID-routés.
8. **Le tap Session déplace le curseur** : `CGEventPost` met à jour le pointeur
   vers la position de l'événement.
9. **AXPress est la seule voie curseur-intact** : action AX directe sur
   l'élément, aucune coordonnée.
10. **`pgrep -x` et pas `pgrep -f`** pour trouver un process.
11. **Un modal ouvert intercepte les clics** : vérifier avant de tirer.
12. **Le serveur VLM peut devenir mort-vivant** (OOM) : watchdog santé.

---

## 📄 Licence

MIT — voir [LICENSE](LICENSE).

## 🙏 Crédits

- **François Bernabé** — vision du projet, exigences, tests sur le terrain
  - ORCID iD : [0009-0000-1482-3137](https://orcid.org/0009-0000-1482-3137) (@MrFelt0385)
- **Sypherine** — architecture, optimisations, tests, documentation
- **Hermes Agent** (Nous Research) — framework de développement
- **mlxcel** — serveur VLM Rust + MLX C++
- **LiquidAI** — modèles LFM2.5-VL (1.6B et 3B)
