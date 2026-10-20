#!/usr/bin/env bash
# Put the website (the repo's site/ folder, which isn't in git) on the
# server, where nginx serves it as spooch.app:
#
#   service/deploy/site.sh user@host
set -euo pipefail

host=${1:?give the server as user@host}
site="$(dirname "$0")/../../site"
[ -f "$site/index.html" ] || { echo "no site/index.html next to the repo" >&2; exit 1; }

rsync -az --delete "$site/" "$host:spooch-site/"
ssh "$host" 'sudo bash -s' <<'REMOTE'
set -euo pipefail
stage=$(getent passwd "$SUDO_USER" | cut -d: -f6)/spooch-site
install -d -m 0755 /var/www/spooch-site
rsync -a --delete --chown=root:root --chmod=D0755,F0644 "$stage/" /var/www/spooch-site/
echo "site: $(find /var/www/spooch-site -type f | wc -l) files"
REMOTE
