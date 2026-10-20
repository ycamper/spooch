#!/usr/bin/env bash
# Build spoochd here, install it on the server and restart it:
#
#   service/deploy/deploy.sh user@host
#
# Builds with Go if it's installed, else in the golang:1.23 image. Needs
# sudo on the server and install.sh run there once beforehand. The schema
# is applied by spoochd itself at startup.
set -euo pipefail

host=${1:?give the server as user@host}
cd "$(dirname "$0")/.."
out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT

echo "== build"
build='CGO_ENABLED=0 GOOS=linux GOARCH=amd64 go build -trimpath -o "$0/spoochd" ./cmd/spoochd'
if command -v go >/dev/null; then
	sh -c "$build" "$out"
else
	docker run --rm -v "$PWD":/src -v "$out":/out -w /src golang:1.23 sh -c "$build" /out
fi

echo "== install on $host, restart"
scp -q "$out/spoochd" "$host:spoochd.new"
ssh "$host" 'sudo bash -s' <<'REMOTE'
set -euo pipefail
home=$(getent passwd "$SUDO_USER" | cut -d: -f6)
install -m 0755 "$home/spoochd.new" /opt/spooch/bin/spoochd
rm -f "$home/spoochd.new"
systemctl enable --quiet spoochd
systemctl restart spoochd
sleep 2
systemctl --no-pager --lines=3 status spoochd | grep -E '●|Active:|spoochd on'
REMOTE
