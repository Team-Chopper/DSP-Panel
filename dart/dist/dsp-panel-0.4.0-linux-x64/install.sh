#!/usr/bin/env bash
# Install or update the DSP408 web panel on Ubuntu/Debian.
#
#   tar xzf dsp-panel-<version>-linux-x64.tar.gz
#   cd dsp-panel-<version>
#   sudo ./install.sh
#
# Re-running updates the program and web client; configuration, DSP list,
# password and certificate are kept.

set -euo pipefail

PREFIX=/opt/dsp-panel
CONF_DIR=/etc/dsp-panel
DATA_DIR=/var/lib/dsp-panel
SERVICE_USER=dsp-panel
SRC="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if [[ $EUID -ne 0 ]]; then
  echo "Please run as root: sudo $0" >&2
  exit 1
fi
for f in dsp-server web dsp-panel.service config.example.yaml; do
  if [[ ! -e "$SRC/$f" ]]; then
    echo "Missing $SRC/$f – run install.sh from the unpacked release folder." >&2
    exit 1
  fi
done
command -v openssl >/dev/null || { echo "openssl is required (sudo apt install openssl)" >&2; exit 1; }

echo "==> Service user"
if ! id -u "$SERVICE_USER" >/dev/null 2>&1; then
  useradd --system --home-dir "$DATA_DIR" --no-create-home --shell /usr/sbin/nologin "$SERVICE_USER"
fi

echo "==> Program files ($PREFIX)"
install -d -m 755 "$PREFIX"
install -m 755 "$SRC/dsp-server" "$PREFIX/dsp-server.new"
mv -f "$PREFIX/dsp-server.new" "$PREFIX/dsp-server"
rm -rf "$PREFIX/web.new"
cp -r "$SRC/web" "$PREFIX/web.new"
chmod -R u=rwX,go=rX "$PREFIX/web.new"
rm -rf "$PREFIX/web"
mv "$PREFIX/web.new" "$PREFIX/web"

# Helper so the password can be set without remembering the paths
cat > /usr/local/sbin/dsp-panel-passwd <<EOF
#!/bin/sh
exec runuser -u $SERVICE_USER -- $PREFIX/dsp-server passwd --config $CONF_DIR/config.yaml
EOF
chmod 755 /usr/local/sbin/dsp-panel-passwd

echo "==> Configuration ($CONF_DIR)"
install -d -m 755 "$CONF_DIR"
if [[ ! -f "$CONF_DIR/config.yaml" ]]; then
  install -m 644 "$SRC/config.example.yaml" "$CONF_DIR/config.yaml"
  echo "    created $CONF_DIR/config.yaml"
else
  echo "    keeping existing $CONF_DIR/config.yaml"
fi

echo "==> Data directory ($DATA_DIR)"
install -d -m 750 -o "$SERVICE_USER" -g "$SERVICE_USER" "$DATA_DIR" "$DATA_DIR/tls"

if [[ ! -f "$DATA_DIR/tls/cert.pem" ]]; then
  echo "==> Creating self-signed TLS certificate"
  HOST="$(hostname -f 2>/dev/null || hostname)"
  SAN="DNS:$HOST,DNS:$(hostname),DNS:localhost,IP:127.0.0.1"
  for ip in $(hostname -I 2>/dev/null); do SAN="$SAN,IP:$ip"; done
  openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -days 3650 \
    -subj "/CN=$HOST" -addext "subjectAltName=$SAN" \
    -keyout "$DATA_DIR/tls/key.pem" -out "$DATA_DIR/tls/cert.pem" 2>/dev/null
  chown "$SERVICE_USER:$SERVICE_USER" "$DATA_DIR/tls/key.pem" "$DATA_DIR/tls/cert.pem"
  chmod 600 "$DATA_DIR/tls/key.pem"
  chmod 644 "$DATA_DIR/tls/cert.pem"
  echo "    certificate valid for: $SAN"
fi

echo "==> systemd service"
install -m 644 "$SRC/dsp-panel.service" /etc/systemd/system/dsp-panel.service
systemctl daemon-reload

if [[ ! -f "$DATA_DIR/auth.json" ]] && [[ -t 0 ]]; then
  echo
  echo "==> No panel password set yet."
  read -r -p "    Set it now? [Y/n] " answer
  if [[ ! "$answer" =~ ^[Nn] ]]; then
    /usr/local/sbin/dsp-panel-passwd
  else
    echo "    Later: sudo dsp-panel-passwd (or set auth.enabled: false in $CONF_DIR/config.yaml)"
  fi
fi

systemctl enable dsp-panel >/dev/null 2>&1
systemctl restart dsp-panel
sleep 1

PORT="$(sed -n 's/^[[:space:]]*port:[[:space:]]*\([0-9]*\).*/\1/p' "$CONF_DIR/config.yaml" | head -n1)"
echo
if systemctl is-active --quiet dsp-panel; then
  echo "DSP408 panel is running:"
  for ip in $(hostname -I 2>/dev/null); do echo "    https://$ip:${PORT:-8443}"; done
  echo
  echo "The browser warns once about the self-signed certificate – accept it to continue."
  echo "Firewall (if ufw is active): sudo ufw allow ${PORT:-8443}/tcp"
  echo "Logs: journalctl -u dsp-panel -f"
else
  echo "The service did not start – see: journalctl -u dsp-panel -n 50" >&2
  exit 1
fi
