# Architecture

## Modules Rust (`src-tauri/src`)

| Module | Rôle | Testé |
|---|---|---|
| `model.rs` | Schéma `accounts.json` (`schemaVersion: 1`) | via `store` |
| `store.rs` | Lecture, validation, upsert pur, écriture atomique sans écriture si inchangé | oui |
| `claude.rs` | Détection du CLI, profils `CLAUDE_CONFIG_DIR` idempotents, lanceurs, variable persistante | oui |
| `desktop.rs` | Localisation MSIX et lancement de Claude Desktop | manuel |
| `usage.rs` | Parsing JSONL, déduplication, blocs de 5 h, résumé | oui (fixture) |
| `recommend.rs` | Score et choix du compte | oui |
| `lib.rs` | Commandes Tauri (contrat `Result<T, AppError{code,message}>`) | — |

## Décisions (reuse-first)

- Isolation CLI par `CLAUDE_CONFIG_DIR` : **Adopt** (mécanisme officiel).
- Blocs de 5 h : **Learn** de ccusage, porté en Rust.
- Desktop multi-instance par `--user-data-dir` : **Adapt** de GiovanniTrevisan/claude-multi-account.
- Quota réel via `/api/oauth/usage` (ClaudeSwitch) : **non retenu** pour le MVP (endpoint non documenté, manipulation de jetons). Piste suivante : instantané `rate_limits` de la statusline officielle.
- Mica et barre de titre personnalisée : **Adopt** `windowEffects` de Tauri 2.
- Fiches forfait et recommandation : **Build**.

## Invariants

- Le profil principal (`config_dir: null`) est unique et jamais modifié.
- `ensure_profile` converge : un second appel ne fait aucune action ; un dossier réel existant n'est jamais remplacé par une jonction.
- `store::save` n'écrit pas si le contenu est identique.

## Limites connues

- Pourcentages estimés, pas le quota officiel d'Anthropic.
- Desktop hors contexte MSIX : protocole `claude://` et mises à jour à valider sur les instances secondaires ; même icône dans la barre des tâches.
- Hooks partagés : un hook qui écrit son état le partage entre profils.
