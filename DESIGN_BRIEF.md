# Tokenscope — Design Brief

Tài liệu này gửi cho designer (Claude Design hoặc người thật) để hoàn
thiện UI trước release. Mô tả sản phẩm, user, dữ liệu hiện có, IA hiện
tại, style direction, và điểm mở / điểm khóa.

---

## 1. Sản phẩm là gì

**Tokenscope** = local-first usage monitor cho Claude Code (CLI của
Anthropic). Đọc trực tiếp log local `~/.claude/`, không gửi data ra
mạng.

3 entry-point:
- **CLI** (`tokenscope`) — text table cho power user, không phạm vi
  redesign lần này.
- **Menubar popover** (macOS) — view nhanh, luôn 1 click chạm.
- **Dashboard window** — view sâu, mở khi cần quản lý nhiều session.

User chính:
- Lập trình viên dùng Claude Code Pro/Max (subscription) hoặc API,
  có nhiều session song song (thường 5–50 session lưu trong
  `~/.claude/`).
- Quan tâm: chi phí (per-block 5h, total), tốc độ tiêu (burn rate),
  context window từng session đầy chưa, quản lý dọn dẹp session cũ.

---

## 2. User goals — câu hỏi cần trả lời tức thì

Sắp xếp theo tần suất:

1. **"Block 5h hiện tại đã tiêu bao nhiêu? Còn bao lâu reset?"**
   → tray title + Blocks tab active card.
2. **"Đang burn $/hr nhanh không? Block này dự kiến tổng bao nhiêu?"**
   → tray title (`$X/hr`) + Blocks active card (`est. block $Y`).
3. **"Session nào đang active? Session nào sắp đầy context (cần
   /compact)?"** → Sessions tab popover, thanh context %.
4. **"Tôi đã chi bao nhiêu cho project X tuần này?"** → Dashboard
   tab Repos.
5. **"Session cũ chiếm chỗ — xoá những session inactive nào?"** →
   Dashboard Sessions, filter inactive, hard delete.
6. **"Trước khi vượt budget, báo cho tôi"** → proactive notification
   (đã có, không cần UI mới).

---

## 3. Information Architecture hiện tại

### Menubar popover (460 × 620)
Vai trò: **view nhanh, ưu tiên 1–3 thông tin quan trọng nhất**.

| Tab | Nội dung |
|---|---|
| Sessions | Card list, default chỉ active+idle. Mỗi card: status dot, name, cost, context % bar, repo · model · sub. Click card expand 2-col detail. |
| Blocks | Active block card (cost lớn, reset, burn, projection) + Recent blocks table (5 dòng). Gap blocks ẩn. |
| Settings | 1 trường input budget threshold + 2 ghi chú info (pricing path, data source). |

### Dashboard window (1000 × 720, resizable)
Vai trò: **view sâu, quản lý**.

| Tab | Nội dung |
|---|---|
| Sessions | Full table 10 cột, sortable. Filter: search box, status, repo, show-hidden toggle. Click name → expand inline detail. Per-row Hide / Delete actions. |
| Repos | Rollup table 6 cột: repo / sessions / live / tokens / total cost / top model. Sort by cost desc. |

### Tray title (macOS menubar bar)
Format: `$X.XX · $Y/hr · N live` (giấu `$/hr` nếu burn <0.5/hr).

---

## 4. Dữ liệu hiện có (full schema)

### Session
```ts
{
  session_id: string            // UUID v4
  name: string | null           // user-set qua /rename, có thể trùng
  cwd: string | null            // path tuyệt đối, vd /Users/x/Project
  pid: number | null            // process id nếu live
  status: "active" | "idle" | "inactive"
  tokens: {                     // tokens chính của session
    input, output,
    cache_creation, cache_read  // 4 loại — pricing khác nhau
  }
  subagent_tokens: { ... }      // cùng shape, gộp tất cả subagents
  subagent_count: number
  model: string | null          // vd "claude-opus-4-7"
  cost_usd: number | null       // null nếu model không có pricing
  updated_at_ms: number | null  // timestamp ms
  context_tokens: number        // input+cache_read+cache_creation của
                                // assistant turn mới nhất
  context_limit: number         // hiện hardcode 200_000
}
```

### Block (5h billing window)
```ts
{
  start_ms, end_ms              // wall clock
  is_active: boolean            // 1 block active tại mọi thời điểm
  is_gap: boolean               // khoảng idle giữa block (vô usage)
  tokens: { ...4 fields }       // sum trong window
  message_count: number
  models: string[]              // unique models trong window
  cost_usd: number
  burn_usd_per_hr: number       // rolling 10 phút, chỉ block active
  projected_block_usd: number   // cost + burn * remaining hours
}
```

### Repo rollup
```ts
{ repo, session_count, live_count, total_tokens, total_cost_usd, top_model }
```

### Settings
```ts
{ budget_window_usd: number }   // 0 = disable proactive alert
```

---

## 5. Style hiện tại

### Palette (dark only)
- Background: `#1a1a1a` (popover), `#141414` (dashboard).
- Surface hover/expand: `#232323`.
- Border / divider: `#2a2a2a`.
- Text primary: `#e0e0e0`.
- Text secondary: `#888`.
- Text muted: `#555`.

### Trạng thái live
- Active: `#4ade80` (xanh).
- Idle: `#facc15` (vàng).
- Inactive: `#555` (xám).

### Cảnh báo (gradient)
- Healthy: `#4ade80` (<50%).
- Warning: `#facc15` (50–75%).
- High: `#f97316` (75–90%).
- Critical: `#ef4444` (≥90%).

### Typography
- Sans: SF Pro Text (system fallback). Body 13px, headers 18–20px.
- Mono: SF Mono / Menlo cho UID, model id, path. 11px.
- `font-variant-numeric: tabular-nums` mọi nơi có số (cost, tokens,
  duration) để cột thẳng hàng.

### Tray icon
- Chữ "C" SF Pro Rounded weight 900, template-rendered (macOS tự tint
  trắng/đen theo dark/light menubar).

### Tone của ngôn ngữ
- Technical, ngắn gọn, anglo-Vietnamese mix là OK.
- Số luôn 2 chữ số thập phân cho USD (`$12.34`), `<$0.01` nếu nhỏ hơn.
- Token số: rút gọn `45K`, `1.2M` khi không cần chính xác; full
  `45,234` khi cần.
- Không emoji, không icon cute. Symbols allowed: `↻`, `⛶`, `▸`, `▾`,
  `▴`, `·`.

---

## 6. Mục tiêu redesign

Mỗi target nên balance giữa **information density** (user là dev,
chấp nhận đặc) và **clarity** (đừng nhồi nhét gây mệt mắt).

### Goal A — Popover "glanceable"
Popover mở thường 5–10 giây rồi đóng. Phải:
- Cho biết "block hiện tại $bao_nhiêu, burn nhanh không" trong < 1 giây.
- Cho biết "session nào đang sắp nguy hiểm" (đầy context, cost spike).
- Nếu UI hiện tại quá đặc/ít nổi bật, redesign cho dễ "scan".

### Goal B — Dashboard "scannable & actionable"
Dashboard mở khi user muốn deep dive. Phải:
- Tìm nhanh 1 session trong 50 sessions (search/filter rõ).
- So sánh cost giữa repos (sort, viz).
- Hành động hide/delete an toàn (confirm rõ, không destructive accident).

### Goal C — Style coherence
- Mọi window cùng feel (cùng palette, spacing scale, typography).
- Native macOS feel hơn (vd respect system accent color? Hoặc giữ
  tuyệt đối dark — designer chọn).

### Goal D — Tray icon
- Hiện đang là chữ "C" — generic, dễ nhầm với icon terminal. Redesign
  thành thứ unique hơn nhưng vẫn template-style (monochrome
  silhouette) — vd bar chart, gauge, logo riêng. Phải hiển thị tốt
  ở 22×22 và 44×44 (retina).

---

## 7. Ràng buộc kỹ thuật

- **Framework**: SvelteKit (Svelte 5 with runes) + Tauri 2.
- **Styling**: vanilla CSS trong `<style>` tag của `.svelte` file, không
  Tailwind, không UI library. Designer có thể propose thêm
  CSS variables / design tokens — chấp nhận refactor.
- **Window sizes** không cố định:
  - Popover hiện 460×620, có thể đổi nhưng không > 600 wide (không
    còn vibe "menubar").
  - Dashboard 1000×720 mặc định, min 720×480, max tuỳ ý.
- **macOS native chrome**:
  - Popover: không có titlebar, không decorations (vibe popover thật).
  - Dashboard: có titlebar mặc định macOS, không custom.
- **Theme**: hiện chỉ dark. Open: có cần light mode (theo system) không?
  Hiện ý kiến nghiêng "dark-only OK" vì là dev tool, đa số dùng dark.

---

## 8. Điểm mở / Điểm khóa

### Mở (designer được đề xuất)
- Bố cục popover (tabs vs single scroll vs khác).
- Bố cục dashboard (sidebar nav vs top tabs vs khác).
- Card vs row vs grid cho session list.
- Cách visualize context % (bar hiện tại, hay donut, gauge?).
- Cách visualize burn rate (text hiện tại, hay sparkline, gauge?).
- Tray icon mới.
- Palette tinh chỉnh (giữ dark, nhưng đổi accent / shade).
- Spacing / typography scale.
- Iconography (đang dùng symbol unicode, OK nếu đổi sang icon font
  như Lucide / Heroicons miễn license free).

### Khóa (không được đổi)
- Dữ liệu hiển thị: mọi field trong section 4 phải có chỗ hiển thị
  ở đâu đó (popover hoặc dashboard expand).
- "Glanceable" trong < 1s cho popover sessions list.
- Status dot (active/idle/inactive) phải phân biệt được cho người
  colorblind (current xanh/vàng/xám OK nhưng add shape hint nếu đổi
  palette).
- Tabular nums cho mọi số liệu.
- Không emoji.
- Không loại bỏ bất cứ tính năng nào (xem README.md cho danh sách
  features chi tiết).

---

## 9. Deliverable mong muốn

Designer trả lại 1 hoặc nhiều trong:

1. **Mockup PNG/Figma** cho từng screen (popover Sessions, popover
   Blocks, popover Settings, dashboard Sessions, dashboard Repos).
2. **CSS spec** với design tokens (palette, spacing scale, typography),
   để dev (engineer hoặc Claude code) implement.
3. **Tray icon** PNG đen trắng template-ready (22×22, 44×44, 66×66 —
   PIL script tạo file ở `app/src-tauri/icons/tray/`).
4. **Rationale ngắn** (1–2 đoạn) cho mỗi screen: vì sao chọn bố cục
   đó, alternative đã loại bỏ vì sao.

Không yêu cầu prototype clickable hay animation spec — đây là MVP
release, giữ scope tối thiểu.

---

## 10. Reference

- Repo: https://github.com/Jorkendy/Claude-Code-Monitor (tên repo
  sắp đổi sang `tokenscope`).
- README chi tiết feature: `README.md` cùng repo.
- Code frontend hiện tại: `app/src/routes/+page.svelte` (popover) và
  `app/src/routes/dashboard/+page.svelte` (dashboard).
- Inspirations (gợi ý, không bắt buộc theo):
  - **Bartender / iStat Menus** — macOS menubar tools, status density.
  - **Stripe Dashboard** — table density + filters + clean financial
    feel.
  - **Linear** — sidebar nav + keyboard-first, dark mode coherent.
  - **Raycast** — popover compact, density high nhưng vẫn breathable.

---

## 11. Câu hỏi designer có thể hỏi lại

Nếu cần clarification, các câu thường gặp:

- "Light mode cần không?" → Không bắt buộc; nếu propose, phải nguyên
  vẹn dark + light.
- "Có cần animation không?" → Chỉ subtle (hover, expand fade <150ms).
  Không animation kiểu trang chủ.
- "User có pay attention đến tray title không?" → Có. Đây là 1 trong
  số ít chỗ user thấy mà không cần click. Tray title là priority cao.
- "Có cần localize (đa ngôn ngữ) không?" → Không trong MVP.
- "Tokens là gì với user?" → User hiểu khái niệm (lập trình viên dùng
  Claude API). Không cần định nghĩa "input token", "cache" trong UI.
