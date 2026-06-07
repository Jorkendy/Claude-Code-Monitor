# cc-monitor

CLI đọc log local của Claude Code và hiển thị danh sách session với
**tên do người dùng đặt (`/rename`)** thay vì UUID, kèm token 4 loại, status
live (PID check thật), subagent breakdown, và cost ước tính.

Read-only với mọi file trong `~/.claude/`. Hoàn toàn offline.

## Khác biệt với tool khác

Các tool hiện có (ccusage, cclog-cli, ai-token-monitor) chỉ đọc `~/.claude/projects/`
nên chỉ thấy UUID và không biết trạng thái live. `cc-monitor` đọc thêm
`~/.claude/sessions/{pid}.json` (chứa tên `/rename` + PID) và check process
sống bằng `kill(pid, 0)` → hiển thị được tên thật + `active`/`idle`/`inactive`.

## Build

```sh
cargo build --release
# binary tại target/release/cc-monitor
```

Hoặc `cargo install --path .` để cài vào `~/.cargo/bin/`.

Yêu cầu: Rust 1.85+ (edition 2024). macOS Apple Silicon đã test.

## Dùng

```sh
cc-monitor                          # bảng mặc định, sort total desc
cc-monitor --sort cost              # sort theo cost USD
cc-monitor --sort updated           # sort theo updatedAt
cc-monitor --sort name              # sort alphabet
cc-monitor --repo Work              # lọc theo basename của cwd
cc-monitor --include-cache-read     # cộng cache_read vào TOTAL
cc-monitor --full                   # số đầy đủ thay vì 55.7K
cc-monitor --json                   # JSON cho tích hợp / Tauri
cc-monitor --subagents <SESSION_ID> # drill-down subagent của 1 session
cc-monitor --root /path/to/.claude  # override (cũng đọc env CLAUDE_HOME)
```

## Quy ước Total

`TOTAL = input + output + cache_creation` (mặc định loại `cache_read` vì
cache_read rất lớn nhưng rẻ — giống tool khác). Thêm `--include-cache-read`
để cộng cả `cache_read`.

## Cost

- Pricing per-million-token shipped sẵn cho Opus 4.x / Sonnet 4.x / Haiku 4.x
  (snapshot 2026-06 từ `platform.claude.com/docs/en/about-claude/pricing`).
- Cost = `input·$in + output·$out + cache_creation·$cwrite + cache_read·$cread`,
  chia 1_000_000.
- Model không có trong pricing table → `N/A` (không đoán bừa).
- `<synthetic>` và session không có usage → `N/A`.

### Override pricing

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

## Cache (incremental)

Lần đầu chạy: ~80–500ms (tuỳ FS cache). Lần sau: **~5ms** (20× nhanh hơn).

Cache file: `~/.claude/.cc-monitor-cache.json` — file duy nhất tool này ghi
trong `~/.claude/`. Lưu `{size, mtime_ms, byte_offset, tokens, model}` cho
mỗi JSONL.

- size + mtime giống cache → dùng tổng đã cache, không đọc file.
- File chỉ thêm dòng → seek tới `byte_offset` cũ, parse phần thêm, cộng dồn.
- File mới / shrank → đọc lại toàn bộ.

Parser chỉ advance `byte_offset` khi line kết thúc bằng `\n` để tránh
double-count khi resume vào lúc file đang được ghi dở.

## Giới hạn (quan trọng)

- **Cowork / session server-side không hiện**: chỉ session có ghi log local
  vào `~/.claude/projects/` mới xuất hiện.
- **Cost chỉ là ước tính**:
  - Per-token rate. Nếu bạn dùng Pro/Max subscription, hoá đơn thật **không
    bằng** số này (subscription cố định, không tính per-token).
  - `cache_creation_input_tokens` gộp cả 5-minute và 1-hour cache writes.
    Pricing dùng giá 5m (rẻ hơn). Nếu session dùng nhiều 1h cache, cost
    thực cao hơn hiển thị.
  - Subagent token được tính theo model của session cha; nếu subagent dùng
    model khác (vd Haiku trong session Opus), cost hơi sai.
- **Repo của session inactive đoán từ slug**: slug `~/.claude/projects/{slug}/`
  encode mọi `/` thành `-`, không khôi phục được path chính xác khi
  segment chứa `-`. Cột Repo cho session inactive chỉ là segment cuối —
  vd `landing` thay vì `lam-phuong-landing`. Session active dùng `cwd`
  authoritative từ `sessions/{pid}.json` nên không bị.
- **Lock partial trailing line**: dòng cuối chưa có `\n` (Claude Code chưa
  flush) sẽ không được count cho đến lần chạy sau — đảm bảo không
  double-count, đổi lại trễ vài giây với session đang chạy.

## Cấu trúc nguồn dữ liệu (đã verify)

```
~/.claude/
  sessions/{pid}.json                                   # name, pid, cwd, status, updatedAt
  projects/{slug}/{sessionId}.jsonl                     # transcript (usage tại .message.usage)
  projects/{slug}/{sessionId}/subagents/agent-*.jsonl   # subagent transcript
  projects/{slug}/{sessionId}/subagents/agent-*.meta.json  # agentType, description
  history.jsonl                                         # fallback name (first prompt)
```

`usage.iterations[]` lặp counter cho retry/compaction — **không** cộng để
tránh double-count.

## Module layout

```
src/
  main.rs       # thin entry
  cli.rs        # clap args, run() orchestration
  model.rs      # Tokens, LiveStatus, SessionRow
  scanner.rs    # walk 4 nguồn
  parser.rs     # streaming JSONL, filter "usage", drill .message.usage
  joiner.rs     # merge theo sessionId, history fallback
  liveness.rs   # libc::kill(pid, 0)
  pricing.rs    # default table + TOML override + cost calc
  cache.rs      # incremental {size, mtime, offset, tokens, model}
  renderer.rs   # text table + JSON + subagent drill-down
```

## Roadmap (ngoài scope bản đầu)

- Giai đoạn 2: Tauri menu-bar app — core đã thiết kế để extract thành crate,
  Tauri backend (Rust) gọi `joiner::join` + render UI.
- File watcher real-time (hiện chạy theo lệnh, không daemon).
- Transcript drill-down sâu (hiện chỉ token tổng).
- Export CSV.

## Safety

- Read-only tuyệt đối với mọi file trong `~/.claude/` TRỪ
  `~/.claude/.cc-monitor-cache.json` (file của tool).
- Không gửi data ra mạng.
- Không in token/secret.
