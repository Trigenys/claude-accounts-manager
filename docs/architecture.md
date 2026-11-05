# Architecture

## Modules Rust (`src-tauri/src`)

| Module | Rôle | Testé |
|---|---|---|
| `model.rs` | Schéma `accounts.json` (`schemaVersion: 1`) | via `store` |
| `store.rs` | Lecture, validation, upsert pur, écriture atomique sans écriture si inchangé | oui |
| `claude.rs` | Détection du CLI, profils `CLAUDE_CONFIG_DIR` idempotents, lanceurs, variable persistante | oui |
| `sessions.rs` | Conversations d'un profil (`projects/*/<uuid>.jsonl`) : lecture bornée tête/queue, `SessionId` validé, dossier de reprise relu côté Rust | oui |
| `desktop.rs` | Localisation MSIX et lancement de Claude Desktop | manuel |
| `usage.rs` | Parsing JSONL, déduplication, blocs de 5 h, résumé | oui (fixture) |
| `recommend.rs` | Score et choix du compte | oui |
| `webpush.rs` | VAPID, chiffrement Web Push aes128gcm, envoi ([détail](phone-push.md)) | oui |
| `phones.rs` | Téléphones associés, notifications, planning du relais | oui |
| `lib.rs` | Commandes Tauri (contrat `Result<T, AppError{code,message}>`) | — |

## Décisions (reuse-first)

- Isolation CLI par `CLAUDE_CONFIG_DIR` : **Adopt** (mécanisme officiel).
- Blocs de 5 h : **Learn** de ccusage, porté en Rust.
- Desktop multi-instance par `--user-data-dir` : **Adapt** de GiovanniTrevisan/claude-multi-account.
- Quota réel via `/api/oauth/usage` (ClaudeSwitch) : **non retenu** pour le MVP (endpoint non documenté, manipulation de jetons). Piste suivante : instantané `rate_limits` de la statusline officielle.
- Mica et barre de titre personnalisée : **Adopt** `windowEffects` de Tauri 2.
- Fiches forfait et recommandation : **Build**.
- Web Push : **Adopt** `web-push-native` (RustCrypto pur, construit la requête sans client HTTP) plutôt que
  `web-push` (OpenSSL via `ece` et client isahc/libcurl ou hyper-tls) ; envoi par `ureq`.
- Liste des conversations : **Learn** du format lu par `/resume` (`custom-title`, `ai-title`, `cwd`) et des
  sélecteurs existants (cc-index, ccpane, qui lisent tout le fichier) ; **Build** d'un lecteur borné en Rust
  (64 Kio en tête, 64 Kio en queue : les transcripts dépassent 100 Mo). Reprise par `claude --resume <uuid>`.
- Relais push : **Build** minimal en Python stdlib (le VPS n'a ni Node ni Docker utilisable).

## Invariants

- Le profil principal (`config_dir: null`) est unique et jamais modifié.
- `ensure_profile` converge : un second appel ne fait aucune action ; un dossier réel existant n'est jamais remplacé par une jonction.
- `store::save` n'écrit pas si le contenu est identique.
- Notifications de réinitialisation sur téléphone : le relais VPS en est seul propriétaire (pas de doublon PC/VPS) ;
  un planning inchangé n'entraîne aucun `PUT` ; le relais ne stocke ni pourcentages ni clé ni jeton ([détail](phone-push.md)).
- Lancement du CLI : seuls des arguments fixes atteignent le lanceur (`LaunchMode`) ; l'id de reprise est un UUID
  canonique et le dossier de reprise vient du transcript (absolu, existant, sans `;`), jamais de l'interface.
- Dérogation RAIDER documentée : JWT VAPID pré-signé sans `iat` pour le relais ([phone-push.md](phone-push.md#dérogation-raider--jeton-vapid-pré-signé)).

## Limites connues

- Pourcentages estimés, pas le quota officiel d'Anthropic.
- Desktop hors contexte MSIX : protocole `claude://` et mises à jour à valider sur les instances secondaires ; même icône dans la barre des tâches.
- Hooks partagés : un hook qui écrit son état le partage entre profils.
- Téléphones : un navigateur mobile n'a qu'un abonnement par clé VAPID (associer un 2e PC remplace le 1er) ;
  JWT sans `iat` possiblement refusé par Apple ou Mozilla (à valider).
