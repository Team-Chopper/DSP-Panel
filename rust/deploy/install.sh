#!/usr/bin/env bash
# Install or update the Rust DSP panel (dsp-panel-rs) on Ubuntu/Debian, next
# to an existing Dart installation (dsp-panel): own service, directories and
# port (8444), so both can run side by side while testing.
#
#   tar xzf dsp-panel-rs-<version>-linux-x64.tar.gz
#   cd dsp-panel-rs-<version>-linux-x64
#   sudo ./install.sh                                   # fresh data
#   sudo ./install.sh --import-from /var/lib/dsp-panel  # copy DSP list, presets, password
#
# Re-running updates the program and web client; configuration, DSP list,
# password and certificate are kept (an import is only done on request).

set -euo pipefail

NAME=dsp-panel-rs
PREFIX=/opt/$NAME
CONF_DIR=/etc/$NAME
DATA_DIR=/var/lib/$NAME
SERVICE_USER=$NAME
SRC="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
IMPORT_FROM=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --import-from) IMPORT_FROM="${2:-}"; shift 2 ;;
    *) echo "Unknown option $1" >&2; exit 64 ;;
  esac
done

if [[ $EUID -ne 0 ]]; then
  echo "Please run as root: sudo $0 $*" >&2
  exit 1
fi
for f in dsp-server web $NAME.service config.example.yaml; do
  if [[ ! -e "$SRC/$f" ]]; then
    echo "Missing $SRC/$f – run install.sh from the unpacked release folder." >&2
    exit 1
  fi
done
if [[ -n "$IMPORT_FROM" && ! -d "$IMPORT_FROM" ]]; then
  echo "Import directory $IMPORT_FROM not found." >&2
  exit 1
fi
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
cat > /usr/local/sbin/$NAME-passwd <<EOF
#!/bin/sh
exec runuser -u $SERVICE_USER -- $PREFIX/dsp-server passwd --config $CONF_DIR/config.yaml
EOF
chmod 755 /usr/local/sbin/$NAME-passwd

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

if [[ -n "$IMPORT_FROM" ]]; then
  echo "==> Importing data from $IMPORT_FROM"
  systemctl stop $NAME 2>/dev/null || true
  for f in devices.json presets.json auth.json secret.key; do
    if [[ -f "$IMPORT_FROM/$f" ]]; then
      install -m 600 -o "$SERVICE_USER" -g "$SERVICE_USER" "$IMPORT_FROM/$f" "$DATA_DIR/$f"
      echo "    $f"
    fi
  done
  if [[ -f "$DATA_DIR/devices.json" ]]; then
    # Both panels must not talk to the same DSP at once: the imported DSPs
    # start disconnected; connect one in this panel after disconnecting it
    # in the other one.
    sed -i 's/"enabled": true/"enabled": false/g' "$DATA_DIR/devices.json"
    echo "    (imported DSPs start disconnected)"
  fi
  if [[ -f "$IMPORT_FROM/tls/cert.pem" && -f "$IMPORT_FROM/tls/key.pem" ]]; then
    install -m 644 -o "$SERVICE_USER" -g "$SERVICE_USER" "$IMPORT_FROM/tls/cert.pem" "$DATA_DIR/tls/cert.pem"
    install -m 600 -o "$SERVICE_USER" -g "$SERVICE_USER" "$IMPORT_FROM/tls/key.pem" "$DATA_DIR/tls/key.pem"
    echo "    tls/cert.pem, tls/key.pem"
  fi
fi

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
install -m 644 "$SRC/$NAME.service" /etc/systemd/system/$NAME.service
systemctl daemon-reload

if [[ ! -f "$DATA_DIR/auth.json" ]] && [[ -t 0 ]]; then
  echo
  echo "==> No panel password set yet."
  read -r -p "    Set it now? [Y/n] " answer
  if [[ ! "$answer" =~ ^[Nn] ]]; then
    /usr/local/sbin/$NAME-passwd
  else
    echo "    Later: sudo $NAME-passwd (or set auth.enabled: false in $CONF_DIR/config.yaml)"
  fi
fi

systemctl enable $NAME >/dev/null 2>&1
systemctl restart $NAME
sleep 1

PORT="$(sed -n 's/^[[:space:]]*port:[[:space:]]*\([0-9]*\).*/\1/p' "$CONF_DIR/config.yaml" | head -n1)"
echo
if systemctl is-active --quiet $NAME; then
  echo "DSP panel (Rust) is running:"
  for ip in $(hostname -I 2>/dev/null); do echo "    https://$ip:${PORT:-8444}"; done
  echo
  echo "The browser warns once about the self-signed certificate – accept it to continue."
  echo "A DSP accepts probably only one connection: disconnect it in the other panel first."
  echo "Firewall (if ufw is active): sudo ufw allow ${PORT:-8444}/tcp"
  echo "Logs: journalctl -u $NAME -f"
else
  echo "The service did not start – see: journalctl -u $NAME -n 50" >&2
  exit 1
fi
