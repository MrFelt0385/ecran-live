# LEÇONS_APPRISTES.md — Les galères techniques détaillées (pourquoi, comment)

Ce document explique en profondeur les problèmes rencontrés pendant le
développement, pour que les contributeurs comprennent les décisions
d'architecture SANS les revivre. Lisez le README d'abord (résumé), ceci est
le détail.

---

## 1. Pourquoi `--axclick` délègue à cua-driver (au lieu de lire l'AX nous-mêmes)

**Symptôme** : notre binaire lit l'AX tree → `Err Ax(-25211)` sur TOUT.

**Cause** : macOS retourne `kAXErrorAPIDisabled` (-25211) quand le processus
appelant n'a PAS la permission Accessibilité (TCC). Notre binaire a la
permission *Capture d'écran* (les clics CGEvent fonctionnent), mais PAS
*Accessibilité* — deux autorisations distinctes dans Réglages Système.

**Pourquoi ne pas juste ajouter la permission ?** Parce que chaque `cargo
build` produit un nouveau cdhash (signature), et macOS révoque l'autorisation.
Sur une machine de dev qui rebuild souvent, c'est un cauchemar récurrent.

**Solution d'architecture** : on lit l'AX tree via `cua-driver`, un binaire
séparé qui A la permission et reste signé. Notre `--axclick` fait :
1. `pgrep -x <app>` → PID exact (jamais `-f` : il attrape les extensions !)
2. `cua-driver call get_window_state {"pid":..., "window_id":...}` → snapshot
3. Cherche l'élément par label (priorité TextField > Button > StaticText)
4. `cua-driver call click {"pid":..., "element_index":...}` → chemin AX fiable

**Leçon plus large** : quand un outil système a une permission que votre
binaire n'a pas, ne réimplémentez pas — **faites un pont vers l'outil**.
C'est plus fiable et plus maintenable.

## 2. Pourquoi `get_window_state` exige `window_id`

**Symptôme** : `Missing required integer field: window_id`.

**Cause** : le snapshot AX de cua est indexé par (pid, window_id) — il faut
dire QUELLE fenêtre de l'app on veut inspecter (une app peut avoir plusieurs
fenêtres, dont certaines invisibles).

**Solution** : on récupère le window_id dynamiquement via
`cua-driver call list_apps {}` (cherche le pid, lit `windows[0].window_id`).
Fallback : ids connus.

## 3. Pourquoi `pgrep -x` et pas `pgrep -f`

**Symptôme** : `--axclick Safari "Description"` → "introuvable" alors que
l'élément existe.

**Cause** : `pgrep -f Safari` retourne 3 PID :
- SafariWidgetExtension (extension !)
- SafariBookmarksSyncAgent (daemon !)
- Web App (une PWA Safari !)

Le premier PID n'est PAS Safari — l'AX tree de l'extension ne contient pas
le formulaire GitHub !

**Solution** : `pgrep -x Safari` → le PID exact du processus principal (40423).
`-x` matche le nom exact du processus, pas le chemin complet.

## 4. Pourquoi le clic a écrit la description dans le champ Repository name

**Symptôme** : on a cliqué "Description" mais le texte est allé dans le champ
nom ("Name cannot be more than 100 characters").

**Cause (triple)** :
1. Le premier clic utilisait `find_text_ocr` (ANCIEN) qui retournait le
   PREMIER match flou — "Description" matchait "New repositon" dans la sidebar !
2. Même corrigé, l'OCR trouvait le LABEL "Description" (un StaticText) et
   cliquait dessus — mais le CHAMP de saisie est à droite, vide (pas de texte
   OCR !)
3. Pas de vérification GLOBALE après le clic : on regardait la zone du champ
   Description sans voir que le focus était resté dans le champ nom.

**Correctifs** :
- `find_text_ocr_best()` : retourne le MEILLEUR match (score Levenshtein
  minimal), pas le premier.
- Clic au CENTRE du bounding box (moyenne des vertices), pas coin + offset.
- `--axclick` : utilise l'AX tree (label) pour trouver les champs VIDES.
- **RÈGLE D'OR** : après CHAQUE action, vérifier l'écran ENTIER avec `--ocr`,
  pas juste la zone ciblée.

## 5. Pourquoi le serveur vision devient "mort-vivant" (OOM)

**Symptôme** : le serveur répondait, puis plus rien — process présent mais
timeout sur TOUTE requête, état `U` (uninterruptible sleep), RSS bizarre.

**Cause** : Mac mini 8 Go. Quand la RAM tombe sous ~20 % libre (Brave +
serveurs + modèles), le chargement d'un modèle ML (2 Go) se bloque en attente
mémoire kernel — `U` = le thread ne peut PAS être tué par un signal normal.

**Pourquoi un check de process ne suffit pas** : le process EXISTE, il est
juste bloqué. Il faut tester la **réponse HTTP réelle** avec un timeout court.

**Solution** : watchdog santé (`hermes-mlxcel-watchdog.sh`) toutes les 30s :
- `lsof -iTCP:8085 -sTCP:LISTEN` → le port écoute ?
- `curl -s -m 8 http://127.0.0.1:8085/v1/models` → il RÉPOND ?
- Si non → `pkill -9 -f mlxcel-server` + kickstart LaunchAgent
- Prévention : si RAM < 20 %, arrête le serveur de SECOURS (mistralrs 8080)
  pour libérer avant l'OOM.

**Leçon plus large** : sur machines à RAM limitée, un "health check" doit
tester la capacité réelle de service, pas l'existence du process.

## 6. Pourquoi le marqueur rose (souris colorée) est un overlay AppKit

**Symptôme** : on voulait une "souris auxiliaire visible" comme cua-driver.

**Approche initiale (ratée)** : CGEvent déplace la vraie souris — mais ça
vole le curseur de l'utilisateur ! Inacceptable en co-working.

**Solution (pattern cua-driver overlay.rs)** : une NSWindow transparente
click-through (`setIgnoresMouseEvents: true`) au-dessus de tout, avec un
CALayer dont le contenu est un cercle rose dessiné par tiny_skia (Pixmap →
CGImage → `layer.setContents`). La vraie souris de l'utilisateur ne bouge
jamais ; le marqueur est purement visuel.

**Leçon** : pour montrer où "on" agit sans déranger l'humain, un overlay
transparent est la bonne brique — pas un mouvement de curseur réel.

## 7. Pourquoi le serveur 4-bit se bloquait au chargement

**Symptôme** : `Converting 494 bf16 weight tensors to f16` en boucle, CPU 0%,
état U.

**Cause** : le modèle 4-bit quantifié contient des tenseurs bf16 (embedding,
head) que mlxcel convertit en f16 au chargement. Sur 8 Go saturés, cette
conversion massive se bloque en attente mémoire.

**Solution** : revenir au **8-bit** (STABLE) — il charge en 1.5s et tourne
des heures sans problème. Le 4-bit + VLM prefix cache donne ÷12 sur les
analyses répétées, mais au prix d'un chargement fragile sur machine à RAM
limite. Compromis documenté : 8-bit par défaut, 4-bit si RAM ≥ 16 Go.

## 8. Pourquoi l'OCR voit "Descriotio" et pas "Description"

Le modèle OCR (ocrs) fait des fautes sur les petits textes d'interface.
Notre matching flou (Levenshtein ≤ 2) tolère ça : "Descriotio" → "Description"
distance 2 = match.

**Piège** : distance ≤ 2 sur des mots courts est TROP permissif
("Repository" matche "New repositon" distance 2 !). Correctif :
- `fuzzy_score()` retourne la distance (pas juste bool)
- `find_text_ocr_best()` choisit le score MINIMAL
- Priorité au match EXACT du label AX dans `--axclick`

## 9. Pourquoi le HUD Hermes semblait "vide" après nos manipulations

**Symptôme** : la fenêtre Hermes (notre chat) devenait vide/transparente.

**Cause** : un drag raté a poussé la fenêtre hors de l'écran (coordonnées
2500,1350 pour une fenêtre de 1002×860 → débordement x=3502 > 2560).

**Leçon** : ne JAMAIS déplacer une fenêtre par drag de coordonnées arbitraires.
Pour changer de fenêtre active, un simple CLIC dessus suffit (macOS gère le
focus). Le drag n'est nécessaire que via la TOPBAR de la fenêtre (clic
maintenu + mouvement).

---

## Règle d'or finale

**ŒIL → MAIN → VÉRIFICATION → APPRENTISSAGE** :
1. Regarder l'écran ENTIER (OCR) avant d'agir
2. Agir (clic, scroll) — une action à la fois
3. Re-regarder l'écran ENTIER pour vérifier l'effet
4. Documenter chaque galère pour ne jamais la revivre
