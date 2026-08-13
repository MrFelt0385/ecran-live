# Contexte global — le modèle mental de la scène

## Le problème

Quand on crop une zone pour la vitesse, on **coupe le contexte global** :
le VLM reçoit une image flottante, sans savoir où elle se trouve dans
l'écran ni ce qu'il y a autour. Résultat : des hallucinations (« piano »,
« drapeau », « voiture » sur des images simples).

## La solution biomimétique : le modèle mental

Le cerveau ne re-regarde jamais une scène entière : il maintient un
**modèle mental** (quel type de pièce, où sont les objets) et **ancre
chaque regard fovéal dans ce qu'il sait déjà**.

## Tests comparatifs (13/08)

| Méthode | Résultat |
|---|---|
| Crop seul | ❌ « pas mentionné » / hallucination |
| Composite 2 panneaux (global + zoom) | ❌ confond les panneaux |
| **Écran complet + cadre bleu (doigt pointé)** | ✅ « section surlignée, interface de code » |
| **Modèle mental + injection contexte** | ✅ analyse locale cohérente avec le global |

Le **cadre bleu** (distinct de l'objet) ancre l'attention — c'est le doigt
pointé d'un humain. La **double vue 2 panneaux** échoue car le 3B ne
comprend pas la composition.

## Implémentation (`--contexte`)

```
ecran-live --contexte [largeur] [question] [x0 y0 x1 y1]
```

1. **ANALYSE GLOBALE** : l'écran entier réduit à 512px (le sweet spot VLM)
   → le VLM établit le contexte (« c'est un éditeur avec terminal »)
2. **MODÈLE MENTAL** : le résumé est gardé
3. **ANALYSE LOCALE ENRICHIE** (si zone donnée) : crop de la zone + question
   PRÉFIXÉE du contexte — le VLM sait où il est

`analyze_image_ctx()` injecte : « Contexte global de l'écran : [résumé].
Question : [question] »

## Testé

```
🧠 MODÈLE MENTAL : « écran avec deux fenêtres, fenêtre bleu foncé à gauche »
🔍 ANALYSE LOCALE : « écran bleu foncé avec un petit rectangle noir dans le
   coin supérieur droit » — cohérente avec le contexte (3.83s)
```

L'analyse locale est guidée par le global : la même scène décrite de façon
cohérente, pas d'hallucination.

## Intégration future

Le modèle mental peut être injecté dans TOUS les modes (blob, veille,
fovéa) : une analyse globale initiale (3-5s) puis toutes les analyses
locales enrichies (~2-4s chacune, mais FIABLES).
