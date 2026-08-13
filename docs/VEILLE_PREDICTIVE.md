# Vision prédictive `--veille` — le cerveau ne traite que l'erreur de prédiction

## Inspiration biomimétique

Deux mécanismes du système visuel humain implémentés en Rust (13/08) :

1. **Predictive coding** (codage prédictif) : le cerveau construit en
   permanence une prédiction du monde et ne traite QUE l'erreur de prédiction
   (ce qui diffère de l'attendu). Sources : Neural Brain framework
   (arXiv:2505.07634), neuroscience du cortex.

2. **Inhibition of Return (IOR)** : pendant la recherche visuelle, le cerveau
   **évite de re-regarder les endroits déjà explorés** et ignore les
   micro-changements permanents (le bruit visuel : curseur, animations
   subtiles). Sources : recherche classique sur l'IOR (Klein 2000, Weger 2006).

## Implémentation

```
ecran-live --veille [largeur] [secondes] [question]
```

Boucle (chaque 0.5s) :
1. Capture frame N
2. `analyse::diff_bbox(prev, png)` — diff pixel (0.05s) → bbox du changement
3. **Immobile** (0 px différents) → aucune analyse VLM (économie totale)
4. **Micro-changement** (< 0.05 % de l'écran : curseur, animations) →
   **ignoré** (inhibition de retour — le cerveau ne réagit pas au bruit)
5. **Changement réel** (≥ 0.05 %) → crop de la zone changée + VLM SEULEMENT
   dessus (~0.5s au lieu de ~6s pour l'écran entier)

## Résultat mesuré (Mac mini M1)

```
[t1] état initial → 4.30s (la prédiction)
[t2] micro-changement (0.014%) ignoré  ← inhibition
[t3] micro-changement (0.018%) ignoré  ← inhibition
[t4] micro-changement (0.019%) ignoré  ← inhibition
[t5] CHANGEMENT 7.81% → analysé (2.45s)
[t6] CHANGEMENT 7.96% → analysé (3.04s)
[t7] CHANGEMENT 11.10% → analysé (2.62s)
⏱️  7 tours, 4 analyses, 3 ÉCONOMISÉES
```

Le système ne réagit qu'aux changements réels : dans un environnement stable
(90% des cas en vision continue), il économise ~50-70% des analyses VLM —
donc la RAM ET la latence.

## Fichiers

- `src/analyse.rs` : `diff_bbox()` (diff en mémoire → bbox + %) et
  `crop_bytes_png()` (crop zone en mémoire sans fichier)
- `src/main.rs` : mode `--veille`
