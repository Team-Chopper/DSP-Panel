#!/usr/bin/env bash
# Remove the Rust DSP panel (dsp-panel-rs). Configuration and data are kept
# unless --purge is given. The Dart installation (dsp-panel) is not touched.

set -euo pipefail

NAME=dsp-panel-rs

if [[ $EUID -ne 0 ]]; then
  echo "Please run as root: sudo $0 [--purge]" >&2
  exit 1
fi

systemctl disable --now $NAME 2>/dev/null || true
rm -f /etc/systemd/system/$NAME.service /usr/local/sbin/$NAME-passwd
systemctl daemon-reload
rm -rf /opt/$NAME

if [[ "${1:-}" == "--purge" ]]; then
  rm -rf /etc/$NAME /var/lib/$NAME
  userdel $NAME 2>/dev/null || true
  echo "DSP panel (Rust) removed, including configuration and data."
else
  echo "DSP panel (Rust) removed. Kept /etc/$NAME and /var/lib/$NAME (use --purge to delete)."
fi
