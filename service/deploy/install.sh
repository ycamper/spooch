#!/usr/bin/env bash
# One-time (and re-runnable) server setup for spoochd on Ubuntu 24.04 with
# Postgres 16, nginx and certbot already there. Run as root from a copy of
# this directory:
#
#   sudo bash install.sh
#
# Creates the spooch system user, its database (peer auth, no password)
# and settings file, gets the certificate, and installs the nginx site and
# systemd unit. It never overwrites /etc/spooch/spooch.env. deploy.sh puts
# the binary in place.
set -euo pipefail

[ "$(id -u)" = 0 ] || { echo "run as root" >&2; exit 1; }
here=$(cd "$(dirname "$0")" && pwd)
names=(api.spooch.app img.spooch.app spooch.app www.spooch.app)

echo "== user and directories"
id spooch >/dev/null 2>&1 || useradd --system --no-create-home --home-dir /nonexistent --shell /usr/sbin/nologin spooch
install -d -o root -g root -m 0755 /opt/spooch /opt/spooch/bin
install -d -o root -g spooch -m 0750 /etc/spooch

echo "== database"
if ! sudo -u postgres psql -tAc "select 1 from pg_roles where rolname='spooch'" | grep -q 1; then
	sudo -u postgres createuser spooch
fi
if ! sudo -u postgres psql -tAc "select 1 from pg_database where datname='spooch'" | grep -q 1; then
	sudo -u postgres createdb -O spooch spooch
fi

echo "== settings"
if [ ! -f /etc/spooch/spooch.env ]; then
	install -o root -g spooch -m 0640 "$here/spooch.env.example" /etc/spooch/spooch.env
	echo "created /etc/spooch/spooch.env; fill in SPOOCH_S3_KEY and SPOOCH_S3_SECRET"
fi

echo "== admin wrapper"
cat > /usr/local/bin/spoochd <<'WRAP'
#!/usr/bin/env bash
# spoochd's admin commands (user add, token add/list/revoke) as the spooch
# user, with the settings read as the service reads them.
exec sudo systemd-run --quiet --pipe --wait --collect --uid=spooch --gid=spooch \
	-p EnvironmentFile=/etc/spooch/spooch.env /opt/spooch/bin/spoochd "$@"
WRAP
chmod 0755 /usr/local/bin/spoochd

echo "== certificate"
if [ ! -f /etc/letsencrypt/live/spooch.app/fullchain.pem ]; then
	# Port 80 only, until there's a certificate for the full site.
	cat > /etc/nginx/sites-available/spooch <<CONF
server {
    listen 80;
    listen [::]:80;
    server_name ${names[*]};
    location /.well-known/acme-challenge/ { root /var/www/letsencrypt; }
}
CONF
	ln -sfn /etc/nginx/sites-available/spooch /etc/nginx/sites-enabled/spooch
	nginx -t && systemctl reload nginx
	certbot certonly -n --webroot -w /var/www/letsencrypt --cert-name spooch.app \
		--key-type ecdsa --expand $(printf -- '-d %s ' "${names[@]}")
fi

echo "== nginx"
install -m 0644 "$here/nginx/spooch-proxy.conf" /etc/nginx/snippets/
install -m 0644 "$here/nginx/spooch" /etc/nginx/sites-available/spooch
ln -sfn /etc/nginx/sites-available/spooch /etc/nginx/sites-enabled/spooch
nginx -t && systemctl reload nginx

echo "== systemd unit"
install -m 0644 "$here/systemd/spoochd.service" /etc/systemd/system/
systemctl daemon-reload
echo "done; now run deploy.sh from a development machine"
