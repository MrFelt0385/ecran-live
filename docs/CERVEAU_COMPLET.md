# Cerveau biomimétique complet — les 6 mécanismes (13/08)

## Le puzzle résolu : 6 mécanismes naturels → 1 système

Toutes les idées du puzzle (approche créative) ont été intégrées dans
`ecran-live --veille` + `--palais` :

| Mécanisme naturel | Implémentation | Gain mesuré |
|---|---|---|
| Predictive coding | Ne traite que l'erreur de prédiction (diff) | 50-70% analyses évitées |
| Inhibition de retour (IOR) | Micro-changements ignorés (< seuil) | bruit filtré |
| Habituation (Montaldi 2006) | Hippocampe : empreinte → réponse | 0 VLM sur scènes connues |
| Anticipation cyclique | Cortex : apprend les transitions A→B | **24/27 tours prédits ⚡** |
| Vocabulaire de deltas | Rétine : empreinte de changement → sens | delta connu = 0 VLM |
| Neurogenèse | Seuils auto-ajustés selon le contexte | adaptation continue |
| Vision entrelacée | Micro-saccades : diff 1 ligne/3, phase 0,1,2 | diff ÷3 |
| Sommeil du palais | Consolidation ADN : PNG → empreintes | **803× plus léger** |

## Test décisif (Mac mini M1, 27 tours)

```
[t1] état initial → analyse VLM (le monde est nouveau)
[t2..t26] ⚡ ANTICIPÉ (cycle connu, 0 capture, 0 VLM) × 24
[t27] micro-changement ignoré + familier (0 VLM)
⏱️  27 tours | 1 analyse VLM | 24 anticipations | seuil 0.05→0.08%
→ 96% d'économie
```

Le cortex a appris « l'écran reste stable » après le premier tour et a
prédit tout le reste — comme le cerveau humain dans une pièce familière.

## Le mode `--palais sommeil` (consolidation ADN)

```
--palais sommeil :
  7.4 Mo (PNG) → 9.2 Ko (empreintes .fpt) = 803× plus léger
  l'index pointe vers les empreintes, la recherche LOCI reste fonctionnelle
```

Inspiré de la consolidation hippocampe → cortex pendant le sommeil : les
souvenirs sont rejoués et ancrés en mémoire compacte.

## Fichiers

- `src/main.rs` : mode `--veille` v3 (cerveau complet)
- `src/analyse.rs` : `diff_bbox_entrelace()` (vision entrelacée)
- `src/palais.rs` : `sommeil()` + `empreinte_from_bytes()` + LOCI
- `docs/VEILLE_PREDICTIVE.md`, `docs/PALAIS_LOCI.md`
