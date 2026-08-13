# Test FastVLM sur mlxcel — rapport complet (13/08)

## Objectif

Vérifier si mlxcel peut charger **FastVLM d'Apple** (`apple/FastVLM-0.5B`) —
l'encodeur FastViT-HD est annoncé **8× plus petit et 20× plus rapide que
ViT-L/14, avec 16× moins de tokens visuels** (CVPR 2025). Ce serait le
remplaçant idéal du vision tower LFM2.5-VL pour l'analyse d'écran rapide.

## Résultats

| Test | Résultat |
|---|---|
| mlxcel reconnaît l'architecture FastVLM | ✅ (loader natif `vlm_fastvlm.rs`) |
| Chargement du modèle Apple original (1.4 GB bf16) | ✅ 1.04s, 1.16 Go résident |
| Génération texte seul | ✅ « Bonjour! Comment puis-je vous aider? » |
| Checkpoints InsightKeeper (`-MLX-4bit`, `-MLX-8bit`) | ❌ **incomplets** : 0 poids vision (conversion ratée) |
| Conversion maison (transform_key officielle mlx-vlm) | ❌ crash inférence vision |
| **Vision (image)** sur modèle Apple original | ⚠️ réponses dégradées (zéros) |

## Découvertes techniques

### 1. Les checkpoints InsightKeeper sont cassés
`InsightKeeper/FastVLM-0.5B-MLX-4bit` et `-8bit` contiennent **639 poids
language_model mais AUCUN poids vision** (vérifié via l'index safetensors).
Le vision tower est absent — les modèles chargent mais ne voient rien.

### 2. mlxcel attend le format Apple ORIGINAL (pas le format MLX)
Le loader `vlm_fastvlm.rs` a un chemin « genuine » dédié :
- Détecte le préfixe `model.vision_tower.vision_tower.model.`
- **Permute automatiquement les convolutions** `(O, I/G, kH, kW) → (O, kH, kW, I/G)`
  (channels-first → channels-last)
- Renomme `patch_embed.<n>` → `patch_embed.blocks.<n>`

**Conséquence** : ne PAS convertir le modèle avec mlx-vlm/transform_key — le
loader fait déjà la transformation. Une conversion maison produit un crash :
`Given groups=96 and weights of shape (96,1,3,3), expected to have 288 input
channels` (les convs ne sont plus permutées car le préfixe genuine a disparu).

### 3. Le processor pose problème
Le repo Apple n'a **pas** de `preprocessor_config.json` (le tokenizer est
Qwen BPE : `vocab.json` + `merges.txt`, pas `tokenizer.json`). Le processor
CLIP (crop 1024px) a été copié depuis le checkpoint InsightKeeper, mais la
vision reste dégradée — le processor exact de FastViT-HD n'est pas encore
trouvé.

## ✅ SOLUTION TROUVÉE (suite du test)

**Le modèle officiel `mlx-community/FastVLM-0.5B-bf16` fonctionne parfaitement
sur mlxcel !** La clé : utiliser le format mlx-community (avec
`chat_template.jinja` inclus), PAS une conversion maison.

| Test | Résultat |
|---|---|
| Vision simple (cercle jaune) | ✅ « Answer: yellow » |
| Écran complet (chaud) | **2.56s** |
| RAM | 1.16 Go résident |

**Pourquoi la conversion maison échouait** : le loader mlxcel a un chemin
« genuine » pour le format Apple original (permutation auto des convs), mais
le modèle Apple n'a pas de `chat_template.jinja` valide ni de
`image_token_index` → le sentinel `<image>` n'était jamais injecté dans le
prompt → vision dégénérée. Le format mlx-community embarque tout.

**Limite découverte** : le 0.5B est trop faible pour la lecture de texte fine
(compteurs HUD, UI) — notre usage nécessite le 3B. FastVLM est adapté au
**balayage rapide** (compréhension globale d'écran en 2.5s) avant le zoom 3B.

## Prochaine étape (fix)

1. Déterminer le processor exact de FastViT-HD (résolution native, resize,
   normalization) — source : `llava_qwen.py` d'Apple / code mlx-vlm
   `models/fastvlm/`
2. Tester la vision sur le modèle Apple original (chemin genuine du loader)
   avec le bon processor
3. Si OK → quantiser en 4-bit et mesurer le gain réel

## Fichiers utiles

- Loader mlxcel : `src/loading/vlm_fastvlm.rs`
- Modèle Apple : `~/Projects/mlxcel/models/FastVLM-0.5B-apple/` (1.4 GB)
- Script de conversion (NON recommandé, voir ci-dessus) : `/tmp/convert_fastvlm.py`
