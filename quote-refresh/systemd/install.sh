#!/bin/sh
# 用法: sudo ./install.sh
set -e
cd "$(dirname "$0")"
install -m 644 quote-refresh.service quote-refresh.timer /etc/systemd/system/
install -m 755 system-sleep-hook /usr/lib/systemd/system-sleep/quote-refresh
systemctl daemon-reload
systemctl enable --now quote-refresh.timer
# 移除舊的 crontab 項目，避免重複執行
crontab -u acclyan -l 2>/dev/null | grep -v 'bin/quote-refresh' | crontab -u acclyan - || true
systemctl list-timers quote-refresh.timer --no-pager
