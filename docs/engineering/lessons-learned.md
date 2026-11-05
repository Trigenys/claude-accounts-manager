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

## 2026-11-04 — Copie de travail en CRLF : patch scripté et empreintes faussés

- **Contexte** : un correctif scripté de `site/scripts/deploy.sh` a échoué (motif introuvable) et les empreintes locales du relais ne pouvaient pas égaler celles du VPS.
- **Cause racine** : Git sous Windows extrait les fichiers en CRLF ; le VPS reçoit et hache des fichiers LF.
- **Classification** : environnement / hypothèse implicite sur les fins de ligne.
- **Résolution** : modifications par remplacement exact tolérant ; le déploiement normalise en LF (`tr -d '\r'`) avant de hacher et d'envoyer le relais et l'unité systemd.
- **Prévention** : le second `--plan` après un apply doit annoncer « inchangé » pour le relais, sinon la normalisation a régressé.
- **Leçon** : toute comparaison d'état entre le poste et un serveur doit porter sur un contenu normalisé, jamais sur les octets de la copie de travail.

## 2026-11-05 — « Connecté » affiché pour un compte dont la session avait expiré

**Contexte :** deux comptes affichaient « Connecté » alors que le relevé échouait avec « compte non connecté ? ».
**Cause racine :** l'état de connexion se déduisait de la seule présence de `.credentials.json` ; une session expirée ou révoquée garde le fichier. Le relevé tuait en plus le CLI en cours d'écriture, laissant des `.claude.json.tmp.*`.
**Résolution :** `quota::parse_response` distingue une réponse sans abonnement (`Readout::LoggedOut`) ; le dernier relevé alimente `sessionExpired` affiché dans l'UI ; le CLI reçoit un EOF et 5 s pour sortir avant d'être tué.
**Prévention :** tests `detects_logged_out_profile` et `expired_flag_follows_last_readout`.
**Leçon :** un fichier présent ne prouve pas un état valide ; dériver un état de la dernière réponse faisant autorité.
