# Lessons learned

## 2026-11-02 — Lanceur `.ps1` introuvable depuis Windows Terminal

- **Contexte** : app de dev lancée depuis un terminal intégré à Claude Desktop (paquet MSIX).
- **Cause racine** : Claude Desktop est un paquet MSIX ; les écritures de ses processus enfants sous `%APPDATA%` sont redirigées vers `%LOCALAPPDATA%\Packages\<pkg>\LocalCache\Roaming`. Les processus hors paquet (Windows Terminal) ne voient pas ces fichiers. `GetCurrentPackageFullName` ne le détecte pas (pas d'identité de paquet).
- **Classification** : environnement / hypothèse implicite sur le système de fichiers.
- **Résolution** : sonde au démarrage (`store::is_virtualized`) qui bloque les lancements avec un message actionnable ; import unique des données redirigées (`store::import_from`, testé).
- **Prévention** : test `imports_redirected_copy_once_without_junctions` ; lancer l'app de dev via l'Explorateur, jamais depuis un terminal de Claude Desktop.
- **Leçon** : tout chemin transmis à un autre processus doit être vérifié visible hors du contexte du processus qui l'écrit.
