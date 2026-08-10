# GUIDE_DEPLOIEMENT.md — Installation complète prête à l'emploi

Ce guide rend `ecran-live` **fonctionnel du premier coup** avec tous les garde-fous.
Copiez-collez les blocs dans l'ordre.

---

## 1. Build + permissions (10 min)

```bash
# Build
cd ~/Projects/ecran-live
cargo build --release          # dépendances: libc, foreign-types, tiny-skia, objc
cp target/release/ecran-live /Applications/ecran-live

# PERMISSIONS — CRITIQUE (sinon Ax(-25211) ou capture noire)
# 1. Ouvrez Réglages Système → Confidentialité et sécurité
# 2. Accessibilité → [+] → ajoutez /Applications/ecran-live (cochez)
# 3. Capture d'écran → [+] → ajoutez /Applications/ecran-live (cochez)
# NOTE : après chaque rebuild (nouveau cdhash), macOS révoque → recommencez.
#   tccutil reset ScreenCapture com.nousresearch.hermes

# VÉRIFICATION RAPIDE (souris + clavier TRUSTED)
/Applications/ecran-live --mousepos                        # position réelle du curseur
/Applications/ecran-live --clickpid 825 63 40423          # clic TRUSTED (SLEventPostToPid)
/Applications/ecran-live --type "test"                    # clavier TRUSTED (auth SkyLight)
```

## 2. Serveur vision mlxcel (5 min)

```bash
# Compiler mlxcel (une seule fois)
cd ~/Projects/mlxcel
cargo build --release --features metal,accelerate

# Modèle 8-bit (STABLE — recommandé)
mkdir -p models/LFM2.5-VL-1.6B-8bit && cd models/LFM2.5-VL-1.6B-8bit
# Téléchargez depuis https://huggingface.co/mlx-community/LFM2.5-VL-1.6B-8bit :
#   model.safetensors, config.json, generation_config.json,
#   processor_config.json, tokenizer.json, chat_template.jinja

# Test manuel du serveur
cd ~/Projects/mlxcel
./target/release/mlxcel-server -m models/LFM2.5-VL-1.6B-8bit --port 8085 \
  --host 127.0.0.1 --parallel 4 --enable-vlm-prefix-cache
# Vérifiez : curl -s http://127.0.0.1:8085/v1/models
```

## 3. LaunchAgent serveur (démarrage auto au login)

`~/Library/LaunchAgents/com.hermes.mlxcel-vision.plist` :

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.hermes.mlxcel-vision</string>
    <key>ProgramArguments</key>
    <array>
        <string>/Users/francoisbernabe/Projects/mlxcel/target/release/mlxcel-server</string>
        <string>-m</string>
        <string>/Users/francoisbernabe/Projects/mlxcel/models/LFM2.5-VL-1.6B-8bit</string>
        <string>--port</string>
        <string>8085</string>
        <string>--host</string>
        <string>127.0.0.1</string>
        <string>--parallel</string>
        <string>4</string>
        <string>--enable-vlm-prefix-cache</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>/Users/francoisbernabe/.hermes/profiles/prompt-engineer/logs/mlxcel.log</string>
    <key>StandardErrorPath</key>
    <string>/Users/francoisbernabe/.hermes/profiles/prompt-engineer/logs/mlxcel.err.log</string>
</dict>
</plist>
```

```bash
launchctl load ~/Library/LaunchAgents/com.hermes.mlxcel-vision.plist
```

## 4. Watchdog de santé (anti mort-vivant — OBLIGATOIRE sur 8 Go)

Le serveur peut se bloquer en état `U` (uninterruptible sleep) quand la RAM < 20 %.
**Un check de process ne suffit pas** — il faut tester la réponse HTTP réelle.

`~/.hermes/scripts/hermes-mlxcel-watchdog.sh` (voir le fichier dans ce repo) :

```bash
chmod +x ~/.hermes/scripts/hermes-mlxcel-watchdog.sh

# Test une fois (devrait ne rien faire si tout va bien)
~/.hermes/scripts/hermes-mlxcel-watchdog.sh --once
```

`~/Library/LaunchAgents/com.hermes.mlxcel-watchdog.plist` :

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.hermes.mlxcel-watchdog</string>
    <key>ProgramArguments</key>
    <array>
        <string>/Users/francoisbernabe/.hermes/scripts/hermes-mlxcel-watchdog.sh</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
</dict>
</plist>
```

```bash
launchctl load ~/Library/LaunchAgents/com.hermes.mlxcel-watchdog.plist
```

## 5. Pont cua-driver (champs de saisie vides)

Notre binaire n'a pas Accessibilité → `--axclick` délègue à cua-driver pour les
champs vides (un champ de saisie n'a pas de texte OCR).

```bash
# Installer cua-driver (binaire Rust autonome)
# Voir https://github.com/trycua/cua — l'installateur officiel
cua-driver permissions grant   # → cochez Accessibilité + Capture d'écran

# Vérifier
cua-driver permissions status
# Accessibility: ✅ granted
# Screen Recording: ✅ granted
```

## 6. Vérification finale (tout en 30s)

```bash
# 1. Serveur répond
curl -s http://127.0.0.1:8085/v1/models | head -c 80
# {"object":"list","data":[{"id":"LFM2.5-VL-1.6B-8bit",...

# 2. Capture fonctionne
ecran-live --ocr 1600 2>&1 | head -3
# Capture OK / OCR ...

# 3. Grounding + clic fonctionnent
ecran-live --locate 1600 "Finder" 2>&1 | head -2
# 🎯 TROUVÉ « Finder » ... [score 0/2]

# 4. Pont AX fonctionne (si cua-driver installé)
ecran-live --axclick Safari "Description" 2>&1 | tail -2
# 🔌 PONT cua: élément [...] « Description »
# ✅ Clic cua effectué
```

---

## 🩺 Diagnostic rapide si ça ne marche pas

| Symptôme | Cause | Correctif |
|---|---|---|
| Capture noire | Screen Recording manquante | Réglages → Confidentialité → Capture d'écran |
| `Ax(-25211)` | Accessibilité manquante | Réglages → Confidentialité → Accessibilité |
| Serveur muet (curl timeout) | OOM / mort-vivant | `launchctl kickstart -k gui/501/com.hermes.mlxcel-vision` + watchdog |
| Clic au mauvais endroit | Coordonnées mélangées | `--clickxy` = écran réel ; OCR = capture 1600 (×1.6) ; cua = element_index |
| `Missing required integer field: window_id` | Pont cua sans window_id | `get_window_state` exige pid + window_id |
| Clic intercepté par un modal | Overlay ouvert | `--ocr` pour vérifier, Escape pour fermer |
