# quota-ui

AI 訂閱額度光圈浮動 HUD（Claude / Codex / Gemini / Cursor）。純 Rust X11、musl 靜態單一執行檔。

## 安裝

```bash
# x86_64（ARM64 請改用 bin/quota-ui-aarch64）
sudo install -m 755 bin/quota-ui /usr/bin/quota-ui
```

## 使用

```bash
quota-ui    # 啟動浮動 HUD
```

- 按住膠囊列可拖曳，位置 / 方向 / 尺寸會存到 `~/.config/quota-ui/config.json`
- 快捷鍵：`o` 切換直橫排列、`r` 手動刷新、`Esc` 退出
- 若存的位置不在任何螢幕上，啟動時會自動移到主螢幕左上角

## 設定為開機自動啟動（systemd user service）

quota-ui 是桌面視窗程式，必須跑在**登入後的圖形 session** 裡，請用 **user service**，不要用 system service。

> ⚠️ 不要放在 `/etc/systemd/system/`。system service 拿不到 `DISPLAY` / `XAUTHORITY`，
> 而且會在登入前就啟動，會出現 `DisplayNotSet` 或 `SetupFailed` 錯誤。
> GNOME Wayland 的 `XAUTHORITY`（`/run/user/<uid>/.mutter-Xwaylandauth.XXXXXX`）每次登入檔名都不同，無法寫死。

### 1. 建立 user service

```bash
mkdir -p ~/.config/systemd/user
cat > ~/.config/systemd/user/quota-ui.service <<'EOF'
[Unit]
Description=AI quota UI
PartOf=graphical-session.target
After=graphical-session.target

[Service]
ExecStart=/usr/bin/quota-ui
Restart=on-failure
RestartSec=3

[Install]
WantedBy=graphical-session.target
EOF
```

### 2. 啟用並啟動

```bash
systemctl --user daemon-reload
systemctl --user enable --now quota-ui.service
```

之後每次登入桌面都會自動啟動，登出時自動停止。

### 3. 檢查狀態 / 日誌

```bash
systemctl --user status quota-ui.service
journalctl --user -u quota-ui.service -f
```

### 從舊的 system service 遷移

如果之前裝在 `/etc/systemd/system/quota-ui.service`，先移除：

```bash
sudo systemctl disable --now quota-ui.service
sudo rm /etc/systemd/system/quota-ui.service
sudo systemctl daemon-reload
```

### 停用

```bash
systemctl --user disable --now quota-ui.service
rm ~/.config/systemd/user/quota-ui.service
systemctl --user daemon-reload
```

## 疑難排解

| 錯誤 / 現象 | 原因 | 解法 |
|---|---|---|
| `DisplayParsingError(DisplayNotSet)` | 沒有 `DISPLAY` | 改用上面的 user service |
| `SetupFailed` | X server 拒絕連線（缺 `XAUTHORITY`） | 改用上面的 user service |
| `Start request repeated too quickly` | 開機時在登入前就啟動，連續失敗 | 改用上面的 user service（綁 `graphical-session.target`） |
| user service 仍拿不到 `DISPLAY` | 非 GNOME 等桌面沒有匯入環境變數 | 在桌面自動啟動腳本加 `systemctl --user import-environment DISPLAY XAUTHORITY WAYLAND_DISPLAY` |
| 程式在跑但看不到畫面 | 舊版本預設位置落在螢幕之間的空隙 | 更新到最新版，或刪除 `~/.config/quota-ui/config.json` |
| `Gemini fetch error: 找不到 Antigravity 憑證` | 沒裝 / 沒登入 Antigravity | 只影響 Gemini 額度，不影響畫面 |

## 建置

```bash
cd quota-ui
rustup target add x86_64-unknown-linux-musl aarch64-unknown-linux-musl
cargo build --release                                      # x86_64（.cargo/config 預設 target）
cargo build --release --target aarch64-unknown-linux-musl  # ARM64
cp target/x86_64-unknown-linux-musl/release/quota-ui ../bin/quota-ui
cp target/aarch64-unknown-linux-musl/release/quota-ui ../bin/quota-ui-aarch64
```
