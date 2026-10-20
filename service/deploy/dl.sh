#!/usr/bin/env bash
# Publish a release's files (the packages and SHA256SUMS, as `gh release
# download` saves them) at https://spooch.app/dl/:
#
#   service/deploy/dl.sh DIR user@host
set -euo pipefail

dir=${1:?give the folder holding the release files}
host=${2:?give the server as user@host}
[ -f "$dir/SHA256SUMS" ] || { echo "no SHA256SUMS in $dir" >&2; exit 1; }
(cd "$dir" && sha256sum -c SHA256SUMS)

rsync -az "$dir/" "$host:spooch-dl/"
ssh "$host" 'sudo bash -s' <<'REMOTE'
set -euo pipefail
stage=$(getent passwd "$SUDO_USER" | cut -d: -f6)/spooch-dl
install -d -m 0755 /var/www/spooch-dl
install -m 0644 "$stage"/* /var/www/spooch-dl/
ls -la /var/www/spooch-dl
REMOTE
