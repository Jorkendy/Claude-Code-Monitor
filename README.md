# cc-monitor

Local-first usage monitor cho Claude Code. Đọc trực tiếp `~/.claude/`,
hiển thị **tên session do bạn đặt (`/rename`)** thay vì UUID, kèm context
window %, burn rate USD/hr, projection cho block 5h, và cost ước tính —
trong CLI hoặc macOS menubar app.

Hoàn toàn offline. Read-only với mọi file trong `~/.claude/` (trừ cache
riêng của tool).

---

## Tại sao dùng cc-monitor

Bảng so sánh với các tool cùng loại (state-of-the-art ngày 2026-06):

| Tính năng | cc-monitor | ccusage | claude-code-usage-monitor | cclog-cli |
|---|---|---|---|---|
| Hiển thị tên `/rename` thay vì UUID | ✓ | ✗ | ✗ | ✗ |
| PID liveness (`active`/`idle`/`inactive`) | ✓ | ✗ | ✗ | ✗ |
| Subagent token breakdown | ✓ | ✗ | ✗ | ✗ |
| 5-hour billing block | ✓ | ✓ | ✓ | ✗ |
| Cost ước tính (per-model) | ✓ | ✓ | ✓ | ✗ |
| Per-million pricing override | ✓ TOML | partial | ✗ | ✗ |
| **Context window % per session** | ✓ | ✗ | ✗ | ✗ |
| **Burn rate USD/hr live** | ✓ | ✗ | ✗ | ✗ |
| **Block projection (est. total at reset)** | ✓ | ✗ | ✗ | ✗ |
| **Proactive budget alert (notify trước khi vượt)** | ✓ | ✗ | ✗ | ✗ |
| macOS menubar app | ✓ | ✗ | ✗ | ✗ |
| Native macOS notifications | ✓ | ✗ | ✗ | ✗ |
| Filesystem watcher (auto-refresh) | ✓ | ✗ | partial | ✗ |
| Incremental cache (warm parse ~5ms) | ✓ | ✗ | ✗ | ✗ |
| Hoàn toàn offline | ✓ | ✓ | ✓ | ✓ |
| Runtime | Single Rust binary | Node | Python | Rust |

Khác biệt cốt lõi: chỉ cc-monitor đọc `~/.claude/sessions/{pid}.json` —
file chứa tên `/rename`, PID, cwd. Các tool khác chỉ đọc
`~/.claude/projects/` (transcript-only) nên không có tên thật, không
phân biệt được session đang chạy vs đã đóng.

---

## Tính năng

### CLI (`cc-monitor`)

- **Session table** với cột UID / NAME / REPO / STATUS / TOTAL / SUBS /
  COST / MODEL.
- **Sort** theo total / cost / updated / name; **filter** theo repo
  basename.
- **Subagent drill-down**: `--subagents <uuid-prefix>` → tách
  agent-* transcripts, mỗi cái 1 dòng.
- **Block view**: `--blocks` → 5h billing windows với reset countdown.
- **JSON output** (`--json`) để pipe / tích hợp.
- **Incremental cache** ~5ms warm parse, đầu chạy 80–500ms.

### Menubar app (macOS)

- **Tray title** luôn hiện `$cost · $burn/hr · N live` ngay trên menubar.
- **Sessions tab** — card list, mỗi card:
  - Chấm màu trạng thái (xanh active / vàng idle / xám inactive).
  - Name + cost USD.
  - Thanh progress **context window %** với màu cảnh báo theo ngưỡng
    (xanh <50% → vàng <75% → cam <90% → đỏ ≥90%).
  - Tự append `#<uid-prefix>` khi 2+ session trùng tên.
  - Click expand → grid chi tiết (UID, PID, cwd, model, breakdown 4
    loại tokens, subagents, updated relative).
  - Default chỉ hiện active+idle; toggle "Show N inactive".
- **Blocks tab**:
  - **Active block card** nổi bật ở top: cost lớn, badge "ACTIVE 5H
    BLOCK", reset countdown, `$burn/hr · est. block $proj`,
    window/msgs/tokens/model.
  - **Recent blocks** = 5 block gần nhất, toggle show all. Gap blocks
    ẩn hoàn toàn.
- **Settings tab**: chỉnh budget threshold USD per 5h block.
- **Filesystem watcher** — debounce 2s, popover auto-refresh, tray title
  cập nhật.
- **Auto-hide popover** khi mất focus (debounce 200ms — notification
  transient không nuốt mất popover).
- **Notifications**:
  - **Reactive**: cost vượt threshold → "Current 5h block at $X.XX".
  - **Proactive**: projection > threshold trước khi cost vượt →
    "Block trending to $XX by reset" — 1 lần/block.

---

## Cài đặt

### Yêu cầu

- macOS Apple Silicon (đã test). Linux/Windows CLI có thể chạy nhưng
  menubar app chỉ build macOS.
- Rust 1.85+ (edition 2024). Quản lý qua [mise](https://mise.jdx.dev):
  ```sh
  mise use -g rust@latest
  ```

### CLI

```sh
git clone https://github.com/Jorkendy/Claude-Code-Monitor.git
cd Claude-Code-Monitor
cargo install --path .
```

Binary cài tại `~/.cargo/bin/cc-monitor`.

### Menubar app

Yêu cầu thêm: [pnpm](https://pnpm.io) (`brew install pnpm` hoặc `npm i -g pnpm`).

**Cài để dùng thường xuyên (production, vào `/Applications`)**

```sh
cd app
pnpm install
pnpm tauri build
```

Output: `app/src-tauri/target/release/bundle/dmg/cc-monitor_0.1.0_aarch64.dmg`
và `.../bundle/macos/cc-monitor.app`.

Cài:

1. Mở file `.dmg` → drag `cc-monitor.app` vào `/Applications`.
2. (Hoặc copy thẳng) `cp -R app/src-tauri/target/release/bundle/macos/cc-monitor.app /Applications/`.
3. Mở từ Spotlight / Launchpad — lần đầu macOS hỏi "open from untrusted
   developer" → Right-click app → Open → Open (vì build chưa ký notarize).
4. Icon "C" hiện trên menubar. Click để mở popover.

Đặt mở cùng đăng nhập: System Settings → General → Login Items → thêm
`cc-monitor.app`.

Gỡ: `rm -rf /Applications/cc-monitor.app` (settings file ở
`~/Library/Application Support/com.ccmonitor.app/` xoá thủ công nếu muốn).

**Chạy dev (hot reload, không cài vào Applications)**

```sh
cd app
pnpm install
pnpm tauri dev
```

Chỉ chạy khi terminal còn mở. Đóng terminal → app tắt.

---

## Dùng CLI

```sh
cc-monitor                          # bảng mặc định, sort total desc
cc-monitor --sort cost              # sort theo cost USD
cc-monitor --sort updated           # sort theo updatedAt
cc-monitor --sort name              # sort alphabet
cc-monitor --repo Work              # filter theo basename của cwd
cc-monitor --include-cache-read     # cộng cache_read vào TOTAL
cc-monitor --full                   # số đầy đủ thay vì 55.7K
cc-monitor --json                   # JSON cho integration
cc-monitor --subagents <UUID-PREFIX> # drill-down subagent
cc-monitor --blocks                 # 5h billing blocks
cc-monitor --root /path/to/.claude  # override $HOME/.claude
```

`--subagents` chấp nhận UUID prefix (vd `48db5a8f`).

---

## Dùng menubar app

- Click icon "C" trên menubar → popover.
- Click ra ngoài → popover ẩn (200ms debounce).
- Right-click icon → menu Quit.
- Tab Sessions: xem live sessions, click card để chi tiết.
- Tab Blocks: xem block 5h hiện tại + lịch sử gần.
- Tab Settings: chỉnh budget threshold.

Settings lưu tại `~/Library/Application Support/com.ccmonitor.app/settings.json`.

---

## Quy ước tính toán

### TOTAL

`TOTAL = input + output + cache_creation` (mặc định loại `cache_read`).
Thêm `--include-cache-read` để cộng `cache_read`.

### Cost

Cost = `(input·$in + output·$out + cache_creation·$cwrite + cache_read·$cread) / 1_000_000`.

- Pricing per-million-token shipped sẵn cho Opus 4.x / Sonnet 4.x /
  Haiku 4.x (snapshot 2026-06).
- Model không có trong pricing table → `N/A`.
- `<synthetic>` (compaction marker) bị skip ở cả model tracking và cost.

### Context window %

`context % = (input + cache_read + cache_creation) của assistant turn
mới nhất / context_limit của model`. Context limit mặc định 200K (mọi
Claude model hiện tại).

Ngưỡng màu trên thanh progress:
- < 50%: xanh
- 50–75%: vàng
- 75–90%: cam
- ≥ 90%: đỏ

Inactive session ẩn thanh context (sau compaction trigger thường về
100% nhưng đã end → vô nghĩa). Vẫn xem được trong expand detail.

### Burn rate

`burn_rate = sum(cost of events in last 10 min) × 3600 / span_seconds`,
với `span_seconds ≥ 60` để 1 event lẻ không tạo rate giả khổng lồ.
Tray ẩn `$/hr` khi burn < $0.5/hr.

### Projection

`projection = current_block_cost + burn_rate × (block_end_ms - now_ms)`.

Proactive alert bắn khi `projection > threshold && cost < threshold`,
1 lần/block (re-arm khi block mới).

---

## Override pricing

Tạo `~/.config/cc-monitor/pricing.toml`:

```toml
[models.claude-opus-4-8]
input = 5.0
output = 25.0
cache_write = 6.25
cache_read = 0.50

[models.your-custom-model]
input = 2.0
output = 10.0
cache_write = 2.5
cache_read = 0.20
```

Override merge vào default table, ưu tiên file user.

---

## Cache (incremental)

Cache file: `~/.claude/.cc-monitor-cache.json`.

Lưu `{size, mtime_ms, byte_offset, tokens, model, latest_context_tokens,
latest_ts_ms}` cho mỗi JSONL.

- size + mtime giống cache → dùng tổng đã cache, không đọc file.
- File chỉ thêm dòng → seek tới `byte_offset` cũ, parse phần thêm, cộng dồn.
- File shrank / mới → đọc lại toàn bộ.

Parser chỉ advance `byte_offset` khi line kết thúc bằng `\n` để tránh
double-count khi resume vào lúc file đang được ghi dở.

Thay đổi schema cache (vd thêm field) → tự đọc lại nhờ `#[serde(default)]`.
Nếu muốn force re-parse: `rm ~/.claude/.cc-monitor-cache.json`.

---

## Giới hạn (quan trọng)

- **Cost chỉ là ước tính**:
  - Per-token rate. Nếu bạn dùng Pro/Max subscription, hoá đơn thật
    **không bằng** số này (subscription cố định, không tính per-token).
    Cost trong cc-monitor là "nếu trả per-token API".
  - `cache_creation_input_tokens` gộp cả 5-minute và 1-hour cache
    writes. Pricing dùng giá 5m (rẻ hơn). Session dùng nhiều 1h cache
    → cost thực cao hơn hiển thị.
  - Subagent tokens tính theo model của session cha; nếu subagent dùng
    model khác (vd Haiku trong session Opus), cost hơi sai.
- **Cowork / server-side session không hiện**: chỉ session có log local
  tại `~/.claude/projects/`.
- **Repo của session inactive đoán từ slug**: `~/.claude/projects/{slug}`
  encode mọi `/` thành `-`, không khôi phục được khi segment chứa `-`.
  Active session dùng `cwd` từ `sessions/{pid}.json` (authoritative).
- **Partial trailing line bỏ qua**: dòng cuối chưa có `\n` (Claude Code
  chưa flush) bỏ count cho đến lần chạy sau — tránh double-count, đổi
  lại trễ vài giây với session đang chạy.
- **Context limit hardcode 200K**: opt-in 1M Opus chưa có model id riêng
  trong data, nên giữ 200K cho an toàn.
- **Burn rate window 10 phút**: task mới start chưa đầy 1 phút sẽ chưa
  có burn rate ý nghĩa.

---

## Cấu trúc nguồn dữ liệu (đã verify)

```
~/.claude/
  sessions/{pid}.json                                       # name, pid, cwd, status, updatedAt
  projects/{slug}/{sessionId}.jsonl                         # transcript chính
  projects/{slug}/{sessionId}/subagents/agent-*.jsonl       # subagent transcripts
  projects/{slug}/{sessionId}/subagents/agent-*.meta.json   # agentType, description
```

Schema mỗi event trong JSONL:
- `.timestamp` — ISO 8601 string.
- `.message.usage.input_tokens`
- `.message.usage.output_tokens`
- `.message.usage.cache_creation_input_tokens`
- `.message.usage.cache_read_input_tokens`
- `.message.model` — vd `claude-opus-4-7`, hoặc `<synthetic>` (compaction
  marker, skip).
- `.message.usage.iterations[]` — lặp counter cho retry/compaction,
  **không** cộng để tránh double-count.

---

## Module layout

```
src/
  lib.rs        # module exports cho cả CLI + Tauri
  main.rs       # thin entry
  api.rs        # high-level API: list_sessions, list_blocks_enriched
  cli.rs        # clap args, run() orchestration
  model.rs      # Tokens, LiveStatus, SessionRow, UsageEvent
  scanner.rs    # walk 4 nguồn
  parser.rs     # streaming JSONL, drill .message.usage
  joiner.rs    # merge theo sessionId
  liveness.rs   # libc::kill(pid, 0)
  pricing.rs    # default table + TOML override + cost calc
  blocks.rs     # 5h block detection (port ccusage algorithm)
  cache.rs      # incremental file cache
  renderer.rs   # text table + JSON + subagent drill + block render

app/
  src/             # SvelteKit frontend
    routes/+page.svelte  # tabs UI
  src-tauri/
    src/
      lib.rs       # Tauri commands, tray, notifications, watcher hook
      watcher.rs   # notify-rs filesystem watcher
    icons/tray/    # custom tray icons (template-rendered "C")
```

---

## Safety

- Read-only tuyệt đối với mọi file trong `~/.claude/` **TRỪ**
  `~/.claude/.cc-monitor-cache.json` (file của tool).
- Không gửi data ra mạng — hoàn toàn local.
- Không in token / secret.
- Không update git config.
- Settings file của app: `~/Library/Application Support/com.ccmonitor.app/settings.json`.

---

## Roadmap

Đề xuất sắp tới (xem [issues](https://github.com/Jorkendy/Claude-Code-Monitor/issues)):

- Per-session burn rate (biết session nào đang tốn khi tray cảnh báo).
- Compaction stats (đếm `<synthetic>` per session + token wasted).
- Per-repo rollup tab.
- Daily/weekly sparkline trends.
- CSV export.
- Linux + Windows support cho menubar app.

---

## License

MIT.
