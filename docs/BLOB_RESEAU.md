# Le BLOB-RÉSEAU — débit adaptatif par zones (Physarum polycephalum)

## Le concept biologique

Physarum polycephalum (le blob) construit un **réseau de veines** pour
transporter la nourriture. Les veines où passe beaucoup de nourriture
**s'épaississent** (débit ↑), les veines inutilisées **s'atrophient**
(débit ↓). C'est l'optimisation naturelle du transport : l'énergie est
allouée dynamiquement là où elle est utile.

Application à la vision d'écran : l'écran = le terrain, le mouvement =
la nourriture, les requêtes VLM = le transport d'information.

## Implémentation (`--blob`)

```
ecran-live --blob [largeur] [secondes] [question]
```

1. **GRILLE** : l'écran est divisé en 3×3 zones
2. **MESURE** : le diff entrelacé (micro-saccades) détecte le mouvement →
   la zone concernée voit son SCORE DE PASSAGE augmenter (la veine se nourrit)
3. **ADAPTE** : le débit de requêtes VLM dépend du score de la zone :
   - zone chaude (score ≥ 5) → analyse toutes les 3s (veine épaisse)
   - zone tiède (score ≥ 2) → analyse toutes les 6s
   - zone froide → jamais analysée (atrophie), suivi pixels gratuit
4. **ATROPHIE** : quand une zone se calme, son score décroît — la veine
   redevient fine (0 requête)
5. **CARTE THERMIQUE** : à la fin, la grille des scores visualise le trafic

## Résultat mesuré (25s réel, animation dans une zone)

```
Carte des veines (3×3) :
     [  0] [  0] [  0] 
     [  0] [  2] [ 11]     ← veine ÉPAISSE zone [2,1] (score 11)
     [  0] [  0] [  0] 

14 tours | 4 requêtes VLM | 9 suivis gratuits
```

- La zone où bouge l'animation → score 11, analysée à débit élevé
- Toutes les autres zones → 0 (atrophiées, jamais analysées)
- **La RAM et les requêtes s'allouent dynamiquement selon le contexte réel**

## Gains

- **RAM mieux utilisée** : les requêtes VLM (coûteuses en RAM/latence) sont
  concentrées là où il y a du passage — 4 requêtes pour 14 tours
- **Rapidité d'échange** : le suivi pixels (gratuit) couvre les zones calmes
- **Adaptation au contexte** : le blob réapprend en continu (scores qui
  montent et descendent) — comme le réseau de veines vivant

## Sources biomimétiques

- Physarum polycephalum : réseau de veines optimal (recherche de Tero et al.,
  Science 2010 — le blob a reconstruit le réseau ferroviaire du Japon)
- Principe général : l'économie d'énergie par l'adaptation du transport
