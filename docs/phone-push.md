# Notifications sur téléphone (Web Push)

Les alertes de consommation, les réinitialisations de quota et un relevé des pourcentages arrivent sur
le téléphone par Web Push standard (RFC 8030/8291/8292), sans APK : une PWA sous
`https://ai-accounts.trigenys.com/app/`. Les réinitialisations arrivent aussi PC éteint.

## Composants

| Élément | Rôle |
|---|---|
| `src-tauri/src/webpush.rs` | Clé VAPID (`push-vapid.json`, créée une fois), chiffrement aes128gcm, envoi HTTP, liste blanche des services push |
| `src-tauri/src/phones.rs` | Téléphones associés (`push-devices.json`), contenu des notifications, synchro du planning avec le relais |
| `lib.rs` (`spawn_reset_watcher`, `phone_*`) | Envoi à chaque tour du veilleur ; commandes d'association, test, bascule, retrait |
| `src/components/PhonesSection.tsx` | Section « Téléphones » du popover des alertes |
| `site/app`, `site/public/app` | PWA : appairage, service worker, décomptes hors ligne (IndexedDB) |
| `site/relay/cam_relay.py` | Relais VPS (Python stdlib, systemd `cam-relay`, 127.0.0.1:8787 derrière Caddy `/api/*`) |

## Flux

1. **Association** : le PC ouvre une offre `{code, vapidKey}` sur le relais (boîte aux lettres en mémoire,
   10 min) et affiche un QR `…/app/#pair=CODE`. Le téléphone lit la clé, s'abonne (`userVisibleOnly`),
   dépose son abonnement une seule fois ; le PC le relève une seule fois (la boîte est alors supprimée)
   et l'enregistre localement. Sur iPhone (iOS 16.4+), la PWA doit être ajoutée à l'écran d'accueil et le
   code saisi à la main.
2. **PC allumé** : le PC chiffre et envoie lui-même alertes, test et relevé (silencieux, au plus toutes
   les 15 min et seulement si un palier de 5 points ou une heure de réinitialisation change).
3. **Réinitialisations** : le PC prépare pour chaque fenêtre une notification déjà chiffrée et signée, et
   remplace en bloc le planning du téléphone sur le relais (`PUT /api/devices/{id}/schedule`) seulement si
   l'empreinte des réinitialisations a changé. Le relais l'envoie à `resets_at`, PC allumé ou non.
   **Règle de déduplication : le relais est seul propriétaire des notifications de réinitialisation** ;
   le PC n'en envoie aucune directement.
4. **Abonnement expiré** : un 404/410 du service push supprime l'appareil côté PC (envoi direct ou test)
   et côté relais (envoi planifié, puis 410 renvoyé au PC à la synchro suivante).

## Ce que voit le relais

L'URL d'abonnement, l'empreinte SHA-256 de la clé de planning de l'appareil, des heures de
réinitialisation, des corps chiffrés opaques et des en-têtes VAPID pré-signés. Jamais de pourcentages,
de clé privée VAPID ni de jeton Claude. Les entrées expirent 6 h après la réinitialisation ; un appareil
sans planning depuis 60 jours est oublié.

## Dérogation RAIDER : jeton VAPID pré-signé

Le relais n'a pas la clé privée VAPID. Le PC signe donc à l'avance, par réinitialisation, un JWT VAPID
sans `iat`, borné à l'origine du service push (`aud`) et expirant 6 h après la réinitialisation (`exp`).
Compromis assumé : une autorisation d'envoi courte, limitée à un service et inutilisable sans les clés de
chiffrement de l'abonnement (que seul le PC et le téléphone ont), plutôt que la clé elle-même sur le VPS.
Risque : un service push qui exigerait `iat` ou une durée totale ≤ 24 h refuserait ces jetons ; à
surveiller à l'essai de bout en bout sur chaque plateforme (Android/FCM vérifié en premier).

## Limites

- Un téléphone associé à deux PC garde l'abonnement du dernier (une clé VAPID par abonnement).
- Les relevés de la PWA vieillissent PC éteint : ils sont étiquetés « relevé il y a … ».
- Chrome impose une notification visible pour chaque push : le relevé est une notification silencieuse
  remplacée sur place (`tag: cam-status`).
