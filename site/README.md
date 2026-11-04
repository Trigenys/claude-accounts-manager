# Site vitrine

Page d'atterrissage statique de Trigenys Claude Accounts Manager (Vite, HTML/CSS, ~3 Ko de JS).
Elle vit dans le dépôt de l'app : le lien de téléchargement, l'empreinte SHA-256 et les captures
suivent la version publiée.

## Développer

```bash
pnpm --dir site --ignore-workspace install
pnpm --dir site --ignore-workspace dev      # http://localhost:1430
pnpm --dir site --ignore-workspace build    # site/dist
```

- Icônes : Phosphor (light), insérées au build par `vite.config.ts` via `<i data-icon="nom"></i>`.
- Police : Geist, auto-hébergée (`@fontsource-variable/geist`), aucune requête externe.
- Captures : `public/media/*.webp` sont prises sur l'app en mode démo (`pnpm dev` à la racine),
  en clair et en sombre. Les données affichées sont fictives.

## Nouvelle version de l'app

Mettre à jour dans `index.html` : les deux liens `releases/download/vX.Y.Z/...`, le lien des notes de
version, le numéro de version et l'empreinte SHA-256 (`gh release view vX.Y.Z --json assets`).
Recapturer les écrans si l'interface a changé.

## Déployer

En production : https://ai-accounts.trigenys.com (VPS OVH, Caddy hôte, DNS chez Spaceship).

Prérequis : un serveur avec Caddy actif, un accès SSH avec sudo, et un enregistrement DNS A du
domaine vers ce serveur (Caddy obtient le certificat HTTPS tout seul).

```bash
export DEPLOY_HOST=ubuntu@<ip> DEPLOY_SERVER_NAME=<domaine> DEPLOY_SSH_OPTS="-i ~/.ssh/<cle>"
site/scripts/deploy.sh --plan   # lecture seule : release, conf Caddy, ligne import, DNS
site/scripts/deploy.sh          # apply
```

Le script :

- construit `site/dist` et calcule son empreinte ; si la release en ligne est identique, rien n'est envoyé ;
- publie dans `/var/www/claude-accounts-manager/releases/<date>-<commit>` puis bascule le lien `current`
  (retour arrière : repointer `current` vers une release précédente, 3 sont conservées) ;
- écrit `/etc/caddy/sites/claude-accounts-manager.caddy` seulement s'il diffère (en-têtes de sécurité,
  CSP stricte, cache long sur `/assets/`) ; ajoute `import /etc/caddy/sites/*.caddy` au Caddyfile s'il
  manque, avec sauvegarde ;
- valide avec `caddy validate` avant de recharger ; en cas d'échec, restaure la conf précédente ;
- ne touche à aucun autre site, au pare-feu ni aux conteneurs.

Une seconde exécution sans changement n'écrit rien.
