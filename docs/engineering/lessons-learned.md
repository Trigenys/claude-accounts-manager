# Lessons learned

## 2026-11-02 — Lanceur `.ps1` introuvable depuis Windows Terminal

- **Contexte** : app de dev lancée depuis un terminal intégré à Claude Desktop (paquet MSIX).
- **Cause racine** : Claude Desktop est un paquet MSIX ; les écritures de ses processus enfants sous `%APPDATA%` sont redirigées vers `%LOCALAPPDATA%\Packages\<pkg>\LocalCache\Roaming`. Les processus hors paquet (Windows Terminal) ne voient pas ces fichiers. `GetCurrentPackageFullName` ne le détecte pas (pas d'identité de paquet).
- **Classification** : environnement / hypothèse implicite sur le système de fichiers.
- **Résolution** : sonde au démarrage (`store::is_virtualized`) qui bloque les lancements avec un message actionnable ; import unique des données redirigées (`store::import_from`, testé).
- **Prévention** : test `imports_redirected_copy_once_without_junctions` ; lancer l'app de dev via l'Explorateur, jamais depuis un terminal de Claude Desktop.
- **Leçon** : tout chemin transmis à un autre processus doit être vérifié visible hors du contexte du processus qui l'écrit.

## 2026-11-04 — Vérification DNS faussée par le résolveur local

- **Contexte** : `site/scripts/deploy.sh --plan` affichait `ai-accounts.trigenys.com` résolu en 96.45.45.45 avant même la création de l'enregistrement.
- **Cause racine** : le réseau de l'utilisateur passe par un filtre web dont le DNS répond pour tout domaine ; la sortie de `nslookup` est en plus localisée (« Nom : ») et en CRLF sous Windows.
- **Classification** : environnement / hypothèse implicite sur le résolveur.
- **Résolution** : interrogation d'un résolveur public (1.1.1.1), parsing tolérant à la locale et au CRLF.
- **Prévention** : le plan affiche l'IP résolue publiquement, à comparer à l'IP du serveur avant l'apply.
- **Leçon** : un contrôle d'état distant ne doit pas dépendre du réseau du poste qui l'exécute.
