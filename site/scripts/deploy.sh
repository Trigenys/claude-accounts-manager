#!/usr/bin/env bash
# Publie le site statique (site/dist) derrière un Caddy existant, en HTTPS automatique.
# Idempotent : release datée + lien `current` ; build identique = aucun envoi, conf identique = aucune écriture.
#
#   DEPLOY_HOST=ubuntu@1.2.3.4 DEPLOY_SERVER_NAME=app.example.com site/scripts/deploy.sh --plan   # lecture seule
#   DEPLOY_HOST=ubuntu@1.2.3.4 DEPLOY_SERVER_NAME=app.example.com site/scripts/deploy.sh          # apply
#
# Variables : DEPLOY_HOST et DEPLOY_SERVER_NAME (obligatoires), DEPLOY_PATH (/var/www/<site>),
# DEPLOY_SITE (claude-accounts-manager), DEPLOY_SSH_OPTS (ex. "-i ~/.ssh/ma_cle"), DEPLOY_KEEP (3).
# Ne touche qu'à ce qu'il possède : DEPLOY_PATH et /etc/caddy/sites/<site>.caddy. Le Caddyfile n'est
# modifié que pour ajouter la ligne `import /etc/caddy/sites/*.caddy` si elle manque (sauvegarde .bak).
# Une conf refusée par `caddy validate` est annulée : la conf précédente est restaurée, rien n'est rechargé.
set -euo pipefail

HOST="${DEPLOY_HOST:?DEPLOY_HOST manquant (ex. ubuntu@1.2.3.4)}"
DOMAIN="${DEPLOY_SERVER_NAME:?DEPLOY_SERVER_NAME manquant : un domaine est requis pour le HTTPS}"
SITE="${DEPLOY_SITE:-claude-accounts-manager}"
ROOT="${DEPLOY_PATH:-/var/www/${SITE}}"
KEEP="${DEPLOY_KEEP:-3}"
SSH="ssh -o BatchMode=yes ${DEPLOY_SSH_OPTS:-}"
PLAN=0; [[ "${1:-}" == "--plan" ]] && PLAN=1

cd "$(dirname "$0")/.."
RELEASE="$(date -u +%Y%m%dT%H%M%SZ)-$(git rev-parse --short HEAD 2>/dev/null || echo local)"
CONF_PATH="/etc/caddy/sites/${SITE}.caddy"

CADDY_CONF="# ${SITE} : généré par site/scripts/deploy.sh, ne pas éditer à la main
${DOMAIN} {
	root * ${ROOT}/current
	encode zstd gzip
	header {
		Strict-Transport-Security \"max-age=31536000\"
		X-Content-Type-Options \"nosniff\"
		Referrer-Policy \"strict-origin-when-cross-origin\"
		X-Frame-Options \"DENY\"
		Permissions-Policy \"camera=(), microphone=(), geolocation=()\"
		Content-Security-Policy \"default-src 'self'; img-src 'self' data:; style-src 'self'; font-src 'self'; script-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'self'; form-action 'none'\"
		-Server
	}
	@assets path /assets/*
	header @assets Cache-Control \"public, max-age=31536000, immutable\"
	@media path /media/* /*.png
	header @media Cache-Control \"public, max-age=604800\"
	@html path / /index.html
	header @html Cache-Control \"no-cache\"
	file_server
}"

echo "→ build"
pnpm --ignore-workspace install --frozen-lockfile --silent >/dev/null
pnpm --ignore-workspace build >/dev/null
test -f dist/index.html
HASH=$(cd dist && find . -type f ! -name .release-hash -print0 | sort -z | xargs -0 sha256sum | sha256sum | cut -c1-16)

# État courant (lecture seule).
REMOTE=$($SSH "$HOST" "
  systemctl is-active --quiet caddy && echo caddy=actif || echo caddy=inactif
  echo hash=\$(cat ${ROOT}/current/.release-hash 2>/dev/null)
  grep -qE '^import /etc/caddy/sites/\*\.caddy' /etc/caddy/Caddyfile 2>/dev/null && echo import=present || echo import=absent
  sudo cat ${CONF_PATH} 2>/dev/null | sha256sum | cut -c1-16 | sed 's/^/conf=/'
")
grep -q 'caddy=actif' <<<"$REMOTE" || { echo "✗ Caddy n'est pas actif sur ${HOST} : ce script ne publie que derrière un Caddy existant." >&2; exit 1; }
REMOTE_HASH=$(sed -n 's/^hash=//p' <<<"$REMOTE")
REMOTE_CONF=$(sed -n 's/^conf=//p' <<<"$REMOTE")
LOCAL_CONF=$(printf '%s\n' "$CADDY_CONF" | sha256sum | cut -c1-16)
# Résolveur public : le DNS local peut être filtré ou menteur.
DNS=$(nslookup "$DOMAIN" 1.1.1.1 2>/dev/null | tr -d '\r' | awk '/^(Name|Nom)/ {n=1} n && /^Address/ {print $2}' | head -n1 || true)

echo "→ plan pour https://${DOMAIN} (DNS : ${DNS:-non résolu})"
[[ "$REMOTE_HASH" == "$HASH" ]] && echo "  release : identique (${HASH}), aucun envoi" || echo "  release : nouvelle ${RELEASE} (${HASH}, en ligne : ${REMOTE_HASH:-aucune})"
[[ "$REMOTE_CONF" == "$LOCAL_CONF" ]] && echo "  ${CONF_PATH} : identique" || echo "  ${CONF_PATH} : à écrire"
grep -q 'import=present' <<<"$REMOTE" && echo "  Caddyfile : import déjà présent" || echo "  Caddyfile : ajout de la ligne import (avec sauvegarde)"
(( PLAN )) && exit 0

if [[ "$REMOTE_HASH" != "$HASH" ]]; then
  echo "→ envoi de la release ${RELEASE}"
  $SSH "$HOST" "sudo mkdir -p ${ROOT}/releases/${RELEASE} && sudo chown -R \$USER ${ROOT}"
  tar -C dist -czf - . | $SSH "$HOST" "tar -C ${ROOT}/releases/${RELEASE} -xzf - && echo ${HASH} > ${ROOT}/releases/${RELEASE}/.release-hash"
  $SSH "$HOST" "chmod -R a+rX ${ROOT} && ln -sfn ${ROOT}/releases/${RELEASE} ${ROOT}/current"
fi

if [[ "$REMOTE_CONF" != "$LOCAL_CONF" ]] || ! grep -q 'import=present' <<<"$REMOTE"; then
  echo "→ conf Caddy"
  printf '%s\n' "$CADDY_CONF" | $SSH "$HOST" "
    set -e
    sudo mkdir -p /etc/caddy/sites
    tmp=\$(mktemp); cat > \"\$tmp\"
    prev=\$(mktemp); sudo cp ${CONF_PATH} \"\$prev\" 2>/dev/null || : > \"\$prev\"
    sudo install -m 644 \"\$tmp\" ${CONF_PATH}
    if ! grep -qE '^import /etc/caddy/sites/\*\.caddy' /etc/caddy/Caddyfile; then
      sudo cp /etc/caddy/Caddyfile /etc/caddy/Caddyfile.bak-\$(date +%s)
      printf '\n# sites déployés par scripts\nimport /etc/caddy/sites/*.caddy\n' | sudo tee -a /etc/caddy/Caddyfile >/dev/null
    fi
    if ! sudo caddy validate --config /etc/caddy/Caddyfile --adapter caddyfile >/dev/null 2>&1; then
      if [ -s \"\$prev\" ]; then sudo install -m 644 \"\$prev\" ${CONF_PATH}; else sudo rm -f ${CONF_PATH}; fi
      echo '✗ caddy validate a échoué : conf précédente restaurée, rien rechargé' >&2; exit 1
    fi
    rm -f \"\$tmp\" \"\$prev\"
    sudo systemctl reload caddy
    echo '  conf écrite et Caddy rechargé'
  "
fi

$SSH "$HOST" "ls -1dt ${ROOT}/releases/* | tail -n +$((KEEP + 1)) | xargs -r rm -rf; echo \"  en ligne : \$(readlink ${ROOT}/current)\""

echo "→ vérification"
for i in 1 2 3 4 5 6; do
  curl -fsS -o /dev/null -w "  HTTP %{http_code} https://${DOMAIN}/\n" "https://${DOMAIN}/" && exit 0
  sleep 10  # premier certificat : laisser le temps à l'ACME
done
echo "✗ https://${DOMAIN}/ ne répond pas (DNS ou certificat ?)" >&2; exit 1
