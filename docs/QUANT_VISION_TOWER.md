# Quantification du vision tower LFM2-VL — −500 Mo RAM, 2× plus rapide (13/08)

## Découverte

Le modèle `LFM2.5-VL-3B-MLX-4bit` quantifie le LLM (600 poids, 334 scales)
mais **laisse le vision tower SigLIP2 400M ENTIÈREMENT en fp16**
(437 poids, 0 scales) — vérifié dans l'index safetensors.

Le vision tower = **0.83 GB sur 2.37 GB total (35%)** en fp16, alors qu'en
4-bit il ne pèserait que ~0.42 GB.

## Solution (script `/tmp/quantize_vision2.py`)

Quantifier UNIQUEMENT les projections des couches encoder du vision tower
(`self_attn.q_proj/k_proj/v_proj/out_proj` + `mlp.fc1/fc2`) en 4-bit
group_size 64 via `mx.quantize`. Le patch_embedding et la position_embedding
restent plain (formes spéciales non-matmul).

## Pièges découverts (important pour refaire)

1. **Les clés de quantification** : les poids vision finissent DÉJÀ par
   `.weight`. Le loader cherche `prefix.weight` + `prefix.scales` +
   `prefix.biases` — il faut **REMPlACER** `.weight` par `.scales`/`.biases`
   pour les sidecars, pas les ajouter (`q_proj.weight.scales` = CRASH).

2. **group_size doit être 64** (le défaut du loader via
   `text_args.group_size()` = `quantization.unwrap_or(64)`). Avec gs=32 le
   loader infère bits=2 et crashe :
   `quantized_matmul ... expanded quantized matrix (2304, 1152) ... bits=2`.

3. **Ne quantifier que les matmul standards** : quantifier le
   patch_embedding (forme P×P×C) cause un mismatch
   `(1,308,1152) vs (144,1152)`.

## Résultats mesurés (Mac mini M1 8 Go)

| Métrique | Tower fp16 | Tower 4-bit | Gain |
|---|---|---|---|
| Poids disque | 2.37 GB | 1.97 GB | −0.40 GB |
| **Footprint RAM** | 2.8 Go | **2.3 Go** | **−500 Mo** |
| **RAM libre** | 31% | **64%** | ×2 |
| **Vitesse à chaud** | ~1.0s | **0.51s** | **2×** |
| Finesse (lecture HUD) | ✓ | ✓ identique | conservée |

Le vision tower quantifié est PLUS RAPIDE que le fp16 : la déquantification
est native sur GPU Metal, et le coût mémoire réduit améliore le cache.

## Statut

- Modèle : `~/Projects/mlxcel/models/LFM2.5-VL-3B-MLX-4bit-vq` (1.97 GB)
- LaunchAgent `com.hermes.mlxcel-vision.plist` → pointe vers `-vq`
- Le modèle original `LFM2.5-VL-3B-MLX-4bit` (2.37 GB) reste disponible
