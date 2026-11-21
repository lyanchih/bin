# sys-info

以 Rust 編寫的主機資訊採集 CLI。支援 Linux、macOS 與 Windows 的 x86_64 和 aarch64，包含 Apple M 系列。採集使用系統 API 與核心公開的資料，執行時不依賴外部命令。

## 使用方式

```sh
sys-info
sys-info -t cbor > host.cbor
sys-info --content-type json-gzip --protocol stdout > host.json.gz
sys-info -t cbor-gzip -p https://collector.example/inventory -p coap://collector.example/inventory
sys-info -t json -p mqtt://broker1.example/hosts/inventory -p mqtt://broker2.example/hosts/inventory -p stdout
```

`-t`／`--content-type` 可設定一次，選擇 `json`（預設）、`cbor`、`json-gzip` 或 `cbor-gzip`。四種格式使用相同的報告結構，帶有 `-gzip` 的格式輸出一個 gzip 串流。

`-p`／`--protocol` 直接接受目的地 URI 或 `stdout`。可重複指定，程式會採集並編碼一次，再依指定順序交付。省略所有 `-p` 時只輸出到 stdout。有指定 `-p` 時，只有列出 `stdout` 才會輸出至 stdout。

| 協定 | 傳輸方式 |
| --- | --- |
| `stdout` | 直接輸出報告。未壓縮的 JSON 結尾有換行。 |
| `http` | 對 `http://` 或 `https://` URI 發出 POST，使用 `Content-Type: application/json` 或 `application/cbor`，gzip 另外使用 `Content-Encoding: gzip`。URI 帳密使用 HTTP Basic。收到 2xx 即成功。 |
| `coap` | 對 `coap://` URL 透過 UDP 發出確認式 POST，預設連接埠 5683。JSON 和 CBOR 的 Content-Format 分別為 50 和 60。JSON-gzip 和 CBOR-gzip 使用實驗用編號 65000 和 65001，接收端需採用相同編號。超過 1024 bytes 的報告使用 Block1。 |
| `mqtt` | 對 `mqtt://host[:port]/topic` 發佈 MQTT 5 訊息，預設連接埠 1883，採用 QoS 1 並等待 broker 的 PUBACK。媒體類型寫入 Content Type，gzip 另帶 `content-encoding=gzip` User Property。可用 `mqtt://user:password@host/topic` 指定帳密，連線為明文 TCP。 |

網路目的地會從 URI 取得協定、主機或 IP、連接埠及路徑。HTTP 和 MQTT 可以在 URI 設定帳密。CoAP 使用 UDP，帳密欄位在此協定下會視為參數錯誤。

個別交付失敗會寫至 stderr，並繼續處理後面的目的地。所有目的地交付成功時退出碼為 0，序列化或任一交付失敗為 1，參數錯誤為 2。採集錯誤會隨報告傳送。

## 報告欄位

| 欄位 | 內容 |
| --- | --- |
| `schema_version` | 結構版本，目前為 3 |
| `cpu` | 製造商 `band`、型號 `type`、基礎頻率 `frequency`（Hz）、架構 `arch`、邏輯 CPU 數 `count` |
| `gpu[]` | 裝置識別 `id`、製造商 `band`、型號 `type`、驅動版本 `version`、記憶體容量 `size` 與類型 `memory_kind`。視平台另附 `pci_id`、`driver` 或 `shared_limit` |
| `ram` | 系統可見的實體記憶體總容量 `total` |
| `disk[]` | 掛載點、檔案系統類型、總容量 `total` 與已用容量 `used` |
| `mount[]` | 已掛載的網路檔案系統。各筆含 `mount_point`，可取得時另附 `filesystem` 和 `source` |
| `distribution` | 作業系統 `os`、發行版識別 `dis`、名稱、版本及核心版本 |
| `host` | `hostname`、`fqdn`、DNS 網域 `domain`、執行環境 `type`、執行採集程式的系統帳號 `user` 及有效 `uid`、`gid` |
| `network` | 首個 IPv4 預設路由的 `default_gateway` 與其介面的本機位址 `external_ip`、介面名稱與 MAC、IPv4 位址及數字前綴長度、全機可見的 IPv4 DNS 伺服器 |
| `power[]` | 量測來源 `source` 與功率大小 `watt` |
| `runtime` | 採集時的 UTC Unix 時間 `system_time`、開機經過的 `uptime`（皆以秒表示），以及主機的開機識別字串 `boot_id` |
| `error[]` | 採集失敗的 `field`、原因 `message` 及適用時的裝置或介面 `subject` |

容量一律以 bytes 表示。缺值、空集合及值為 0 的容量欄位會省略。採集失敗仍在 `error` 記錄，例如 `disk.space` 同時涵蓋總容量和已用容量。CPU 與 GPU 的 `type` 會縮短常見型號名稱，例如 `i5-9400` 與 `RTX 3060 Ti`。CPU 的 `frequency` 只使用基礎頻率。Linux 優先採用 CPUFreq 的 `base_frequency`，若有多個政策則取最高基礎頻率。其他來源包括型號標示的額定頻率與 x86 CPUID 基礎頻率欄位。平台未提供可確認的基礎頻率時，省略此欄位並寫入 `error`。

`boot_id` 用於同一台主機的開機識別。Linux 和 macOS 提供 UUID，Windows 提供系統開機計數器的字串值。

磁碟清單優先回報 OS 所在的本機磁區，容量須至少 1 GB。其他內接磁區須已掛載且容量大於 500 GB。門檻採十進位 GB。開機、EFI、系統輔助、網路及可辨識的外接磁區會排除。Linux 保留容器 overlay 的根檔案系統，Windows 支援磁碟機代號和資料夾掛載點。大型資料磁區若無法核實是內接儲存（例如無法辨識底層裝置的 Linux ZFS），會省略該磁區並在 `error` 記錄原因。OS 磁區讀不到裝置類型或容量時會保留可取得的資訊並記錄錯誤。

`mount` 和本機 `disk` 分開。清單只採集已掛載的網路檔案系統，例如 NFS、SMB 和 SSHFS。Windows 使用本登入工作階段已連線的網路磁碟機，提供掛載點和遠端來源，省略檔案系統類型。採集讀取現有掛載資訊，無須向遠端查詢容量。來源包含遠端主機及共享路徑，URL 的帳號密碼、查詢參數與片段會省略。沒有網路掛載時省略整個 `mount`。

`host.type` 是依本機跡象判斷的執行環境，可為 `baremetal`、`vm`、`microvm`、`container`、`lambda` 或 `wsl`。可辨識多層環境時，Lambda 和容器優先。MicroVM 需有可辨識的機型或 hypervisor 標記。

`host.user` 採作業系統回報的有效帳號。這可能是服務帳號，或是以提升權限執行時所使用的帳號。帳號無法解析時會省略欄位，並在 `error` 記錄 `host.user`。Linux 和 macOS 同時提供有效使用者 ID `uid` 和有效群組 ID `gid`，ID 為 0 時仍會回報。Windows 使用 SID 識別帳號及群組，省略這兩個 POSIX 欄位。

Apple Silicon 的 GPU 使用統一記憶體，`memory_kind` 為 `unified` 時的 `size` 是主機的實體記憶體總容量。Windows 共享記憶體上限與 macOS Metal 建議工作集上限各自回報於 `shared_limit`。部分 GPU 識別只在本次開機有效。

功率為電池充放電或感測器回報的功率大小，平台可依 `source` 判讀。Linux 可讀取 power supply 或 hwmon，macOS 與 Windows 讀取系統電池。沒有相應遙測時會記錄在 `error`。

網路介面若有 IPv4 預設路由，只列出優先的預設路由介面，同一優先順序可能有多個介面。預設路由經由 VPN 或其他虛擬介面時仍會列出該介面。無預設路由時只列出啟用的實體介面，省略 `external_ip` 與 `default_gateway`。若預設路由是直連路由，也會省略 `default_gateway`。有同優先順序路由時，兩個欄位都採系統先列出的路由。介面的第一個 IPv4 位址可能是私有位址。Linux 的路由來源為核心 IPv4 主路由表；macOS 優先採用未限定介面的預設路由；Windows 依路由與介面的合計 metric 選擇。Linux 的 DNS 來自 `/etc/resolv.conf`，系統使用本機 stub resolver 時可看到 loopback 位址。macOS 沒有 DNS 完整主機名時可使用系統設定的 `.local` 名稱。

## 採集備援

共用動態函式庫載入器在 `src/dynamic_library.rs`，按執行檔的 OS 與 CPU 架構查表。外部執行檔白名單在 `src/execute_binary.rs`，按欄位、OS 與架構指定可執行的絕對路徑和固定參數。適用的採集順序為動態函式庫、白名單執行檔、底層 API。

| OS 和架構 | 動態後端 | 外部執行檔 | 底層來源 |
| --- | --- | --- | --- |
| Linux GNU x86_64、aarch64 | NVIDIA GPU 使用架構白名單內的 `libnvidia-ml.so.1` | `gpu.size` 可用 `nvidia-smi` | PCI／DRM sysfs、procfs 或 Nouveau DRM ioctl |
| Linux musl x86_64、aarch64 | 靜態版會將無法載入 NVML 記錄在 `error` | `gpu.size` 可用 `nvidia-smi` | 同上 |
| macOS x86_64、aarch64 | 系統 Metal framework | 無 | IOKit GPU 裝置與屬性 |
| Windows x86_64、aarch64 | 系統目錄內的 `dxgi.dll` | 無 | SetupAPI 顯示裝置資訊 |

Linux NVIDIA GPU 只有在 NVML 無法提供容量時才呼叫 `nvidia-smi`。每次採集最多查詢一次，依 PCI 位址配對 GPU，並將 MiB 轉成 bytes。執行器直接啟動白名單路徑，限制執行時間為 5 秒，每個輸出串流最多 64 KiB。動態載入與執行檔的失敗各自記錄於 `gpu.library` 和 `gpu.binary`，底層若能補到資料仍會輸出。musl 主程式維持靜態連結，使用此備援的主機需裝有可執行的 `nvidia-smi`。

## 編譯

需要 Rust 1.88 或更新版本，以及目標平台的 C 編譯器。HTTPS 使用靜態連結的 `ring`。Linux 跨架構編譯可用 Clang，並透過 `CC_<target>` 和 `AR_<target>` 指定工具，環境變數中的目標名稱以底線取代連字號。Windows ARM64 編譯需安裝 Clang。從專案目錄執行以下命令，讓 Cargo 載入專案的目標設定。

```sh
cd sys-info

# Linux，選擇其中一個架構，musl 產生靜態執行檔
rustup target add x86_64-unknown-linux-musl
cargo build --locked --release --target x86_64-unknown-linux-musl
rustup target add aarch64-unknown-linux-musl
cargo build --locked --release --target aarch64-unknown-linux-musl

# Linux，原生 GNU 版本，支援選擇性載入 NVIDIA NVML
cargo build --locked --release

# macOS 11+，在裝有 Xcode Command Line Tools 的 Mac 選擇一個目標
cargo build --locked --release --target aarch64-apple-darwin
cargo build --locked --release --target x86_64-apple-darwin

# Windows 10 build 17763+ 或 Windows Server 2019+，使用 MSVC 和 Windows SDK
cargo build --locked --release --target x86_64-pc-windows-msvc
cargo build --locked --release --target aarch64-pc-windows-msvc
```

macOS 與 Windows 如需編譯非 Rust host 的架構，先用 `rustup target add <target>` 安裝目標，並備妥該平台的工具鏈。Windows 的 Cargo 設定啟用靜態 CRT，其餘連結使用 Windows 隨附的系統 DLL。macOS 連結系統隨附的 frameworks 和系統 runtime。

共用報告結構在 `src/model.rs`，共用 CPU 與記憶體採集在 `src/hardware.rs`，各平台實作在 `src/linux.rs`、`src/macos.rs` 和 `src/windows.rs`，Unix 共用的主機與介面採集在 `src/unix.rs`。
