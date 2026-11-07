#!/usr/bin/env bash
# Remove the DSP web panel. Configuration and data are kept unless
# --purge is given.

set -euo pipefail

if [[ $EUID -ne 0 ]]; then
  echo "Please run as root: sudo $0 [--purge]" >&2
  exit 1
fi

systemctl disable --now dsp-panel 2>/dev/null || true
rm -f /etc/systemd/system/dsp-panel.service /usr/local/sbin/dsp-panel-passwd
systemctl daemon-reload
rm -rf /opt/dsp-panel

if [[ "${1:-}" == "--purge" ]]; then
  rm -rf /etc/dsp-panel /var/lib/dsp-panel
  userdel dsp-panel 2>/dev/null || true
  echo "DSP panel removed, including configuration and data."
else
  echo "DSP panel removed. Kept /etc/dsp-panel and /var/lib/dsp-panel (use --purge to delete)."
fi
