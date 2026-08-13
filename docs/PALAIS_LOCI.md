# Méthode LOCI biomimétique pour le Palais — activation diffuse + mémoire de travail

## Le concept biologique

La méthode des loci (ou palais de mémoire) est une technique mnémotechnique
utilisée depuis l'Antiquité (Cicéron, Simonides) : on associe des souvenirs à
des LIEUX spatiaux familiers. Le cerveau ne « scanne » jamais tout le palais
pour retrouver un souvenir — il suit des **routes associatives** :

1. **Activation diffuse (spreading activation)** : un souvenir est retrouvé
   par SES ASSOCIATIONS (le nom active les lieux qui le contiennent), pas par
   une recherche exhaustive. Un même souvenir est accessible par PLUSIEURS
   chemins (blob-like).

2. **Mémoire de travail 7±2 (Miller 1956)** : seuls ~7 items sont gardés
   actifs — les données récentes sont retrouvées par le chemin le PLUS COURT
   (hot cache), les autres par des chemins plus longs.

## Implémentation (palais.rs)

### Index inversé : `associations: BTreeMap<nom, Vec<pièces>>`
- `associer(nom, pièce)` : crée le lien nom → pièce (appelé par `ranger`)
- Un même nom peut vivre dans plusieurs pièces → **plusieurs chemins d'accès**
- Blob-like : la donnée n'est pas dupliquée, mais référencée par N routes

### Activation diffuse : `retrouver <nom>`
- Recherche EXACTE dans l'index inversé (O(1)) → toutes les pièces
- Recherche PARTIELLE (fragments) si pas d'exact : « compt » → compteur_hud
- Les pièces en mémoire de travail sont triées en premier (chemin le plus court)

### Mémoire de travail : `travail: Vec<String>` (max 7)
- `noter_travail(pièce)` : appelé par `chercher` et `ranger`
- Les 7 dernières pièces consultées restent « chaudes »
- Affiche : `--palais travail`

## Commandes

```
ecran-live --palais ranger <img> <l> <c> <nom>   — range + associe le nom
ecran-live --palais chercher <l> <c>             — liste la pièce (O(1))
ecran-live --palais retrouver <nom>              — activation diffuse (LOCI)
ecran-live --palais travail                      — mémoire de travail 7±2
ecran-live --palais visiter                      — vue d'ensemble
```

## Testé

```
--palais ranger simple_test.png 3 2 bouton_clique   → l3_c2
--palais ranger mosaic_256.png 3 2 compteur_hud     → l3_c2
--palais ranger simple_test.png 5 7 bouton_clique   → l5_c7 (2e chemin !)

--palais retrouver bouton →
  ACTIVATION DIFFUSE — « bouton » → 2 chemin(s) d'accès :
  [0] l3_c2 🔥 mémoire de travail
  [1] l5_c7 🔥 mémoire de travail        ← blob-like

--palais retrouver compt →
  « compt » → 1 chemin(s) : l3_c2        ← reconnaissance par fragments

--palais travail →
  MÉMOIRE DE TRAVAIL : [0] l3_c2 (récent 2) [1] l5_c7 (récent 1)
```

## Gains

- **Rapidité de recherche** : O(1) par association (vs O(n) scan du palais)
- **Chemins multiples** : un souvenir accessible par N routes (blob-like)
- **Fiabilité RAM** : l'index est un petit JSON (quelques Ko), pas de
  duplication des captures — les fichiers restent à UNE seule place
- **Récence** : les données chaudes sont priorisées (mémoire de travail)
