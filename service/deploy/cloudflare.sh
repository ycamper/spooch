#!/usr/bin/env bash
# Refresh Cloudflare's address ranges for nginx. Run as root on the server
# (install.sh runs it too; again whenever Cloudflare changes its ranges):
#
#   sudo bash cloudflare.sh
#
# Writes two snippets from one fetch, then reloads nginx:
#   cloudflare-realip.conf  the visitor's address from CF-Connecting-IP
#                           (other sites on this nginx include it too)
#   cloudflare-geo.conf     the same ranges for a geo block, so spooch.app
#                           answers only Cloudflare
# A short list changes nothing: an empty one would shut every visitor out.
set -euo pipefail

[ "$(id -u)" = 0 ] || { echo "run as root" >&2; exit 1; }
snippets=/etc/nginx/snippets
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

for v in 4 6; do
	curl -fsS --max-time 20 "https://www.cloudflare.com/ips-v$v" >"$tmp/v$v"
	# The lists end without a newline; without this, cat below would run
	# the last IPv4 range into the first IPv6 one.
	echo >>"$tmp/v$v"
done
ranges=$(cat "$tmp/v4" "$tmp/v6" | tr -d '\r' | grep -E '^[0-9a-f:.]+/[0-9]+$' || true)
n=$(printf '%s\n' "$ranges" | grep -c . || true)
if [ "$n" -lt 10 ]; then
	echo "only $n ranges from Cloudflare; leaving nginx as it is" >&2
	exit 1
fi

day=$(date -u +%F)
{
	echo "# Cloudflare's edge ranges (https://www.cloudflare.com/ips-v4, /ips-v6),"
	echo "# fetched $day by spooch's cloudflare.sh. From these addresses only, the"
	echo "# visitor's address is taken from CF-Connecting-IP; a request sent"
	echo "# straight to the server keeps its own."
	printf 'set_real_ip_from %s;\n' $ranges
	echo "real_ip_header CF-Connecting-IP;"
} >"$tmp/cloudflare-realip.conf"
{
	echo "# Cloudflare's edge ranges, fetched $day by spooch's cloudflare.sh,"
	echo "# for a geo block."
	printf '%s 1;\n' $ranges
} >"$tmp/cloudflare-geo.conf"

for f in cloudflare-realip.conf cloudflare-geo.conf; do
	if [ -f "$snippets/$f" ]; then
		cp -p "$snippets/$f" "$tmp/$f.old"
	fi
	install -m 0644 "$tmp/$f" "$snippets/$f"
done
if ! nginx -t; then
	for f in cloudflare-realip.conf cloudflare-geo.conf; do
		if [ -f "$tmp/$f.old" ]; then
			cp -p "$tmp/$f.old" "$snippets/$f"
		else
			rm -f "$snippets/$f"
		fi
	done
	echo "nginx refused the new ranges; put the old ones back" >&2
	exit 1
fi
systemctl reload nginx
echo "$n Cloudflare ranges in place"
