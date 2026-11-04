# Trigenys Claude Accounts Manager

Application de bureau Windows (Tauri 2 + React) pour utiliser plusieurs comptes Claude sur une même machine :

- **Profils Claude Code isolés** : chaque compte a son dossier `CLAUDE_CONFIG_DIR` (connexion, historique, `.claude.json`).
  `skills/`, `agents/`, `hooks/` et `commands/` sont partagés avec le profil principal par jonctions ; `CLAUDE.md` importe les consignes globales ; `settings.json` est copié une fois.
- **Claude Desktop** : le profil principal ouvre l'app installée ; les autres comptes lancent l'exécutable avec leur propre `--user-data-dir`.
- **Forfaits** : type (Pro, Max 5×, Max 20×), prix, date de renouvellement, couleur.
  « Remplir depuis la connexion » lit les métadonnées non secrètes du profil (`subscriptionType`, `rateLimitTier`, `oauthAccount.emailAddress`, `subscriptionCreatedAt`) : forfait et email exacts, renouvellement **estimé** (anniversaire mensuel), prix public pré-rempli à ajuster.
- **Conso** : tokens de la fenêtre de 5 h (règle ccusage) et des 7 derniers jours, lus dans `<profil>/projects/**/*.jsonl`. Le pourcentage est une estimation : plafond saisi, sinon pic historique du compte.
- **Recommandation** : le compte le moins entamé (`0,6 × 5 h + 0,4 × semaine`), en excluant ceux à 90 % ou plus sur 5 h.

## Données et sécurité

| Élément | Emplacement |
|---|---|
| Profil principal | `~/.claude` et `~/.claude.json`, jamais modifiés par l'app |
| Fiches comptes | `%APPDATA%\claude-account-manager\accounts.json` (écriture atomique, `.bak`) |
| Profils 2+ | `%APPDATA%\claude-account-manager\profiles\<id>\{cli,desktop}` |
| Lanceurs | `%APPDATA%\claude-account-manager\launchers\<id>.ps1` |

L'app ne lit ni ne transmet aucun jeton d'authentification. Retirer un compte supprime sa fiche, pas son dossier de profil.

« Utiliser par défaut » écrit `CLAUDE_CONFIG_DIR` dans les variables d'environnement utilisateur (supprimée pour le profil principal) : cela s'applique aussi aux extensions IDE après redémarrage.

## Développement

```bash
pnpm install
pnpm tauri dev
```

```bash
cd src-tauri && cargo test
```

Prérequis : Rust stable MSVC, Node 20+, pnpm, WebView2.

Architecture et décisions : [docs/architecture.md](docs/architecture.md).
