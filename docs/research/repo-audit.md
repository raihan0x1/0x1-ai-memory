# Audit seluruh repo `0x1-ai-memory`

> Audit 27 November 2026. Checkout `v0.3.5` di `4f0bda7`. Audit ini hanya membaca: tidak ada kode, config, atau database yang diubah. DB live dibuka read-only (`file:~/.0x1-memory/memory.db?mode=ro`). `OX1_MEMORY_DISABLE_ONNX=1 cargo test` lulus dengan 41 tes (28 + 2 + 11 + 0). Format laporan mengikuti `ponytail-audit`: diurutkan dari yang paling penting, dan setiap temuan punya kasus gagal yang konkret.

**Apa yang dilakukan repo ini:** `0x1-memory` adalah satu biner Rust yang berjalan sebagai daemon lokal di `127.0.0.1:10101`. Daemon menerima event hook dari Claude Code, Codex, dan Antigravity, lalu menyimpannya di SQLite. Event itu kemudian diringkas oleh LLM menjadi "memori" yang disuntikkan lagi ke sesi agent berikutnya lewat briefing, recall, dan MCP. Biner yang sama juga menyajikan Web UI, endpoint MCP, dan stream SSE, memasang hook ke file config klien AI, dan bisa mengupdate dirinya sendiri.

**Beban yang diasumsikan:** satu developer di satu mesin, dengan beberapa agent (sesi) yang menulis event hook bersamaan lewat satu mutex koneksi SQLite. Pada beban ini, mutex dengan query pendek dan WAL tidak menjadi masalah. Karena itu, temuan di bawah berfokus pada kebenaran data, keamanan file, dan alur update.

**Istilah:** *distilasi* = meringkas event mentah menjadi memori. *Supersede* = memori baru menggantikan memori lama, lalu status memori lama menjadi `superseded`. *Hook* = perintah yang dijalankan klien AI di setiap event, misalnya `0x1-memory hook claude PreToolUse`.

## Harus diperbaiki

### 1. Distilasi menandai event "selesai" padahal event itu tidak pernah dikirim ke model

[`src/llm.rs:500-511`](../../src/llm.rs#L500-L511) · [`src/distill.rs:54-63`](../../src/distill.rs#L54-L63) · [`src/distill.rs:171`](../../src/distill.rs#L171)

- **Apa ini:** Setiap kali satu turn selesai, worker mengambil maksimal 40 event mentah dari sesi itu. Worker lalu membuat prompt dan menandai **ke-40 event** itu `distilled=1`.
- **Masalah:** Prompt hanya mengambil 30 event pertama. Selain itu, prompt berhenti begitu bloknya melewati 7.500 karakter. Setiap potongan event panjangnya sampai 900 karakter, jadi sering hanya sekitar 8 event yang benar-benar masuk prompt. Sisanya tetap ditandai selesai dan tidak pernah diringkas. Simulasi ulang batch pada DB live menunjukkan 309 dari 737 event (41,9%) tidak pernah masuk prompt. Pada 15 dari 76 turn (19,7%), `turn_stop` terakhir, yang memuat jawaban akhir agent, juga ikut terbuang.
- **Perbaikan:** Hitung panjang potongan dari jumlah event, misalnya `let per = (6_000 / events.len().max(1)).clamp(120, 900);`, lalu hapus `take(30)`. Dengan begitu, semua event yang diambil masuk prompt, termasuk event terakhir. Alternatif yang lebih kecil: tandai hanya ID yang benar-benar masuk prompt. Tambahkan satu tes: 40 event masing-masing 6 KB harus menghasilkan prompt yang memuat 40 header dan teks event terakhir.
- **Jika dilewati:** Hampir separuh kerja agent tidak pernah menjadi memori, padahal ini fungsi inti produk. Yang paling sering hilang justru kesimpulan di akhir turn yang panjang.

### 2. Launcher npm membatalkan self-update dan menurunkan daemon ke versi lama

[`npm/bin/0x1-memory.mjs:135-140`](../../npm/bin/0x1-memory.mjs#L135-L140) · [`src/updater.rs:697-703`](../../src/updater.rs#L697-L703) · [`src/daemon.rs:338-343`](../../src/daemon.rs#L338-L343)

- **Apa ini:** Launcher npm memasang biner di `~/.0x1-memory/bin` dan menulis stempel `.version`. Self-update di daemon menimpa biner yang sama dan menulis stempel versi baru.
- **Masalah:** Launcher hanya menerima stempel yang *persis sama* dengan versi paket npm. Misalnya, setelah self-update ke 0.3.6, perintah `npx 0x1-memory ...` dari paket 0.3.5 (dari cache npx atau dari versi yang dipin di config MCP) melihat `0.3.6 !== 0.3.5`. Launcher lalu mengunduh 0.3.5 dan menimpa biner. Setelah itu, `daemon_needs_upgrade` memakai `!=`, sehingga `start` menghentikan daemon 0.3.6 dan menjalankan 0.3.5.
- **Perbaikan:** Di launcher, terima stempel yang `>=` versi paket, cukup dengan membandingkan tiga angka semver. Di `daemon_needs_upgrade`, ganti daemon hanya jika versi yang sedang berjalan *lebih lama*.
- **Jika dilewati:** Update diam-diam mundur, UI terus menawarkan update yang sama, dan siklus update lalu turun versi berulang.

### 3. Penulisan config "atomik" membuat `~/.claude.json` bisa dibaca akun lain dan merusak symlink

[`src/config.rs:58-64`](../../src/config.rs#L58-L64) · [`src/daemon.rs:364`](../../src/daemon.rs#L364)

- **Apa ini:** `write_atomic_file` menulis file sementara, lalu memakai `rename` untuk menimpa config klien AI (`~/.claude.json`, `~/.claude/settings.json`, `~/.codex/*`, dan plugin Antigravity).
- **Masalah:** File baru dibuat mengikuti umask (644), dan izin file lama tidak disalin. Buktinya ada di mesin ini: backup `~/.claude.json` dan `settings.json` masih 600, sedangkan file aslinya sekarang 644. Home dir bermode 750 dengan grup `staff`, jadi akun lain di grup itu bisa membaca `~/.claude.json`. File itu berisi env server MCP, yang sering memuat API key, serta info akun. Selain itu, `rename` mengganti symlink dengan file biasa, sehingga dotfile yang dikelola stow/chezmoi diam-diam terlepas dari repo-nya. Hal yang sama berlaku untuk `memory.db` (644) di `~/.0x1-memory` (755): seluruh riwayat event agent bisa dibaca grup yang sama.
- **Perbaikan:** Resolve symlink dulu dengan `canonicalize`, lalu tulis file sementara di sebelah target. Salin `permissions()` file lama ke file sementara, dan pakai 0600 untuk file baru. Buat `~/.0x1-memory` dengan mode 0700 dan `memory.db` dengan 0600. User yang sudah terkena cukup menjalankan `chmod 600 ~/.claude.json ~/.claude/settings.json`.
- **Jika dilewati:** Rahasia di config klien AI dan isi database memori terbuka untuk akun lokal lain. Dotfile user juga rusak tanpa tanda apa pun.

### 4. Connect/disconnect menghapus hook milik user, dan backup bisa saling menimpa

[`src/connectors.rs:345-348`](../../src/connectors.rs#L345-L348) · [`384-387`](../../src/connectors.rs#L384-L387) · [`544-546`](../../src/connectors.rs#L544-L546) · [`583-586`](../../src/connectors.rs#L583-L586) · [backup `758-771`](../../src/connectors.rs#L758-L771) · [`573`](../../src/connectors.rs#L573)

- **Apa ini:** Saat connect, kode membuang hook 0x1-memory yang lama lalu menambahkan yang baru. Saat disconnect, kode membuang semuanya. Sebelum mengubah file, kode membuat backup.
- **Masalah:** Filter bekerja per *grup* hook (`{matcher, hooks:[...]}`) dengan aturan `item.to_string().contains("0x1-memory")` atau `contains("hook claude")`/`"hook codex"`. Akibatnya, jika user menaruh hook sendiri di grup yang sama, misalnya `audit.sh` di samping hook kita, atau perintahnya kebetulan memuat teks itu (`my-tool hook claude-notify`), seluruh grup ikut terhapus. Nama backup hanya presisi sampai detik. `~/.codex/hooks.json` dan `hooks.json` plugin Antigravity punya nama file yang sama, dan `setup` menghubungkan klien secara berurutan dalam detik yang sama, jadi backup kedua menimpa backup pertama. Connect lalu disconnect dalam satu detik juga menimpa backup asli dengan versi yang sudah diubah. Terakhir, `disconnect_codex` menulis `config.toml` dengan `std::fs::write` biasa, sehingga crash di tengah penulisan meninggalkan file yang terpotong.
- **Perbaikan:** Filter per perintah di dalam `hooks[]` dengan pola yang tepat (`"<bin>" hook <client> `), lalu buang grup yang menjadi kosong. Tambahkan milidetik dan nama klien di nama backup, dan jangan pernah menimpa file backup yang sudah ada. Pakai `write_atomic` di `disconnect_codex`. Tambahkan satu tes: grup yang berisi hook user dan hook kita, setelah disconnect, hook user harus tetap ada.
- **Jika dilewati:** Hook audit atau keamanan milik user hilang diam-diam, dan backup yang seharusnya bisa memulihkannya sudah tertimpa.

### 5. Supersede bisa menyembunyikan memori yang dipin, global, atau dari proyek lain

[`src/db.rs:780-787`](../../src/db.rs#L780-L787) · [`src/distill.rs:129-138`](../../src/distill.rs#L129-L138) · [`src/distill.rs:198`](../../src/distill.rs#L198) · [`src/llm.rs:486`](../../src/llm.rs#L486) · [briefing pinned `src/db.rs:1357`](../../src/db.rs#L1357)

- **Apa ini:** Prompt distilasi menampilkan 6 memori terbaru dari proyek itu kepada model, lalu meminta model mengisi `supersedes_id` jika memori baru menggantikan memori lama. `create_memory` lalu menjalankan `UPDATE memories SET status='superseded' ... WHERE id = ?1`.
- **Masalah:** Query itu tidak memeriksa status, scope, atau proyek. "Pinned" di sini adalah sebuah *status*, jadi jika model memilih ID memori yang dipin, status `pinned` tertimpa menjadi `superseded`. Memori itu lalu hilang dari briefing (`WHERE status = 'pinned'`). ID yang dikarang model, atau yang dikirim lewat MCP, juga bisa mengenai memori global atau milik proyek lain. Memori yang sudah `superseded` pun bisa ditimpa lagi `superseded_by`-nya, sehingga rantai lineage putus. Di data live hal ini belum terjadi (31 dari 32 supersede berasal dari distilasi, dan 0 lintas proyek), tetapi tidak ada yang mencegahnya.
- **Perbaikan:** Tambahkan `AND status = 'active' AND project_id IS ?4` di UPDATE. Di jalur distilasi, terima `supersedes_id` hanya jika ID itu ada di daftar `existing_recent`. Tambahkan satu tes: hasil distilasi yang menunjuk memori pinned tidak boleh mengubah status memori itu.
- **Jika dilewati:** Aturan yang sengaja dipin user bisa hilang dari briefing karena satu jawaban model, tanpa jejak yang jelas di UI.

## Sebaiknya diperbaiki

### 6. Provider CLI yang timeout tetap berjalan di belakang

[`src/llm.rs:356`](../../src/llm.rs#L356) · [`409`](../../src/llm.rs#L409) · [`444`](../../src/llm.rs#L444) · [smoke test `src/updater.rs:650-656`](../../src/updater.rs#L650-L656)

- **Apa ini:** Provider opsional `claude -p`, `codex exec`, dan `agy -p` dijalankan dengan `tokio::time::timeout(45 s, cmd.output())`.
- **Masalah:** Tokio tidak mematikan proses anak saat future-nya di-drop, kecuali `kill_on_drop(true)` diset. Contoh: jika `claude -p` macet karena prompt login atau jaringan, setelah 45 detik distilasi dianggap error, tetapi prosesnya tetap hidup dan terus memakai token. Setiap turn berikutnya bisa menambah satu proses yatim lagi. Smoke test updater (`--version`) punya masalah yang sama.
- **Perbaikan:** Tambahkan `cmd.kill_on_drop(true)` di keempat tempat itu. Sekalian, satukan tiga adapter CLI yang hampir identik menjadi satu helper `run_cli(cmd, prompt)`.
- **Jika dilewati:** Proses LLM menumpuk dan biayanya terus berjalan tanpa terlihat di ledger usage.

### 7. CORS menerima Origin dari port localhost mana pun

[`src/config.rs:475-488`](../../src/config.rs#L475-L488) · [`src/server.rs:59-61`](../../src/server.rs#L59-L61) · [`src/server.rs:151-173`](../../src/server.rs#L151-L173)

- **Apa ini:** `is_localhost_url` dipakai sebagai predikat CORS sekaligus sebagai guard Host/Origin. Fungsi ini menerima `127.0.0.1`, `localhost`, dan `[::1]` dengan port berapa pun.
- **Masalah:** Halaman apa pun yang tersaji di port lokal lain, misalnya dev server dependency yang terinfeksi di `localhost:3000`, preview HTML yang tidak tepercaya, atau notebook, bisa melakukan `fetch` ke `:10101` dan *membaca jawabannya*. Halaman itu bisa mengambil semua memori dan event, mengganti provider atau `base_url` LLM, atau memanggil connect yang menulis config klien. Endpoint `/mcp` juga tidak punya token. DNS rebinding sudah tertahan oleh guard Host, jadi celahnya hanya dari origin lokal.
- **Perbaikan:** Izinkan hanya Origin dengan port daemon itu sendiri, ditambah `5173` (dev server Vite) di build debug.
- **Jika dilewati:** Satu halaman lokal yang jahat cukup untuk mencuri seluruh riwayat agent dan mengarahkan ringkasan ke server luar.

### 8. `stop` mengirim SIGTERM ke PID yang tidak diverifikasi

[`src/daemon.rs:458-472`](../../src/daemon.rs#L458-L472)

- **Apa ini:** Untuk menghentikan daemon, kode membaca `daemon.pid`, mencocokkan port, lalu menjalankan `kill -TERM <pid>`.
- **Masalah:** Jika daemon pernah mati mendadak (kill -9, crash, atau reboot), file PID lama tetap tertinggal. Setelah reboot, PID itu bisa saja dipakai proses lain, misalnya editor user. `0x1-memory stop`, atau `start` yang memicu upgrade, lalu menghentikan proses yang salah.
- **Perbaikan:** Kirim sinyal hanya jika `/health` menjawab dengan PID yang sama, atau jika `ps -p <pid> -o comm=` memuat `0x1-memory`. Selain itu, cukup hapus file PID.
- **Jika dilewati:** Ada kemungkinan kecil tetapi nyata bahwa pekerjaan user di aplikasi lain tertutup tanpa peringatan.

### 9. Dua salinan kode start daemon, dan salinan untuk update tidak mendeteksi daemon baru yang langsung mati

[`src/daemon.rs:363-431`](../../src/daemon.rs#L363-L431) · [`src/updater.rs:708-752`](../../src/updater.rs#L708-L752) · [`src/updater.rs:530-542`](../../src/updater.rs#L530-L542) · [`src/daemon.rs:477`](../../src/daemon.rs#L477) vs [`506`](../../src/daemon.rs#L506)

- **Apa ini:** `spawn_background` dan `spawn_updated_daemon` sama-sama membuka log, men-detach proses, lalu menjalankan `serve --foreground`. `read_log_tail` dan `read_log_tail_async` juga dua salinan dari logika yang sama.
- **Masalah:** Hanya `spawn_background` yang menunggu `/health` dan menampilkan log jika proses keluar lebih awal. Di jalur update, jika biner baru lolos `--version` tetapi crash saat `serve` (misalnya migrasi DB gagal), `spawn_updated_daemon` tetap mengembalikan `Ok`. Daemon lama lalu mati 150 ms kemudian. Hasilnya, tidak ada daemon sama sekali, dan hook diam-diam berhenti merekam karena hook dirancang gagal secara diam (fail-open).
- **Perbaikan:** Buat satu helper `daemon_command(bin, port, extra_args)` untuk kedua jalur. Di jalur update, sebelum `shutdown_tx`, cek `child.try_wait()` selama sekitar 1 detik, lalu batalkan shutdown dan tampilkan error jika proses baru sudah mati. Simpan hanya versi async dari `read_log_tail`.
- **Jika dilewati:** Satu rilis yang rusak saat start bisa mematikan perekaman memori tanpa pesan apa pun, dan perbaikan di satu salinan tidak ikut sampai ke salinan lainnya.

## Bagus jika sempat

### 10. `mcp-stdio` menjalankan ulang tulisan secara lokal setelah timeout

[`src/main.rs:721-724`](../../src/main.rs#L721-L724)

- **Apa ini:** `mcp-stdio` meneruskan request ke daemon. Jika jawabannya bukan sukses, request dijalankan langsung di SQLite lokal.
- **Masalah:** Timeout 10 detik, error 500, dan 403 semuanya jatuh ke jalur lokal. Contoh: `memory_save` yang lambat karena model ONNX baru dimuat ternyata sudah selesai di daemon, lalu dijalankan sekali lagi secara lokal. Hasilnya, ada dua memori kembar.
- **Perbaikan:** Gunakan jalur lokal hanya untuk error koneksi (`err.is_connect()`). Untuk kasus lain, kembalikan error JSON-RPC.
- **Jika dilewati:** Memori dobel sesekali muncul di briefing dan memakan budget injeksi.

### 11. Mode privasi yang tidak dikenal diam-diam menjadi `full`

[`src/models.rs:346-352`](../../src/models.rs#L346-L352)

- **Apa ini:** `normalize_privacy_mode` mengubah teks bebas menjadi `local_only`, `paused`, atau `full`.
- **Masalah:** Salah ketik seperti `"localonly "` atau `"local_onyl"` dari API, CLI, atau skrip disimpan sebagai `full`, dan API tetap menjawab sukses. Akibatnya, proyek yang ingin dibuat lokal saja justru dikirim ke LLM luar.
- **Perbaikan:** Kembalikan `Option` dan tolak nilai yang tidak dikenal dengan HTTP 400.
- **Jika dilewati:** Proyek yang seharusnya lokal saja bisa bocor ke LLM luar tanpa diketahui user.

### 12. `memory_save` lewat MCP dengan proyek yang tidak dikenal menghasilkan memori yatim

[`src/mcp.rs:384-407`](../../src/mcp.rs#L384-L407) · [`src/models.rs:323-335`](../../src/models.rs#L323-L335)

- **Apa ini:** Fungsi ini mencoba mengubah argumen `project` dari MCP menjadi `proj_...`, entah dari path, ID, atau nama proyek.
- **Masalah:** Nama yang tidak dikenal dikembalikan apa adanya (misalnya `"my-proj"`), dan kolom `project_id` tidak punya foreign key. `scope:"project"` tanpa proyek juga tetap disimpan sebagai `project` dengan `project_id = NULL`. Memori seperti ini tidak pernah cocok dengan `project_id = ?` maupun `scope = 'global'`, jadi tidak pernah muncul di briefing. Di data live saat ini jumlahnya masih 0.
- **Perbaikan:** Kembalikan error jika nama proyek tidak ditemukan. Jika scope `project` tidak punya proyek, turunkan scope ke `global` atau tolak request-nya.
- **Jika dilewati:** Agent mengira sudah menyimpan aturan, padahal aturan itu tidak pernah muncul lagi.

### 13. Kode yang tidak dipakai dan cabang kembar

[`src/config.rs:263-275`](../../src/config.rs#L263-L275) · [`src/db.rs:470-477`](../../src/db.rs#L470-L477) · [`src/connectors.rs:796-798`](../../src/connectors.rs#L796-L798) · [`src/hooks.rs:305-331`](../../src/hooks.rs#L305-L331)

- **Apa ini:** `is_cli_provider`, `is_external_api`, dan `list_sessions` tidak punya pemanggil di `src/` maupun `tests/`. `connectors::write_atomic` hanya meneruskan panggilan ke `write_atomic_file`. Di `format_context_injection`, cabang `"codex"` dan cabang default menghasilkan JSON yang identik.
- **Masalah:** Kode ini tetap harus dibaca dan dipelihara, padahal tidak menambah perilaku apa pun. Contoh nyata: perbaikan format Codex harus diubah di dua cabang, padahal sebenarnya cukup satu.
- **Perbaikan:** Hapus ketiga fungsi itu dan wrapper `write_atomic`. Ubah cabang Codex menjadi `"codex" | _` dan simpan komentar "jangan pakai `systemMessage`" di sana. (`recent_usage` hanya dipakai tes, jadi biarkan.)
- **Jika dilewati:** Tidak ada yang rusak. Ini hanya beban baca yang tidak perlu.

## Sudah diketahui/direncanakan

Hal-hal yang tersentuh audit ini tetapi tidak dilaporkan ulang: nasib raw event saat distilasi error, budget habis, atau mode paused/local_only, termasuk pemakaian ulang key; GET connections yang menulis (UPSERT); query Sessions yang makin lambat; urutan tail dan filter di Stream; celah redaksi (`X-API-Key:`, userinfo URL, `*_TOKEN=`) beserta re-scrub; T3 watcher (`src/t3_watcher.rs`); normalisasi nama klien MCP; timestamp di memori yang disuntikkan dan blok Handoff; event recall/handoff/hasil distilasi sebagai `raw_events`; paging keyset "Load older"; dan semua isi `stream-page-review.md`, `memories-page-review.md`, serta `engine-settings-review.md`.

## Sudah dicek dan aman

- Guard Host/Origin menolak `127.0.0.1.evil.com` dan `Origin: null`, jadi DNS rebinding tertahan.
- File JSON/TOML klien yang rusak ditolak, bukan ditimpa.
- Updater memeriksa SHA-256 dari `SHA256SUMS` dan memvalidasi URL rilis. Link di catatan rilis hanya https, tanpa `innerHTML`.
- Userinfo di remote git dibuang sebelum disimpan (0 dari 13 proyek live memuat `@`).
- Setting retensi dan idle dibatasi rentangnya (1–365 hari, 15–3600 detik).
- Embedding ONNX berjalan di `spawn_blocking`, sehingga tidak memblokir runtime.
- `parse_extraction_json` memotong string di karakter `{`/`}` ASCII, jadi tidak bisa panic di batas UTF-8.
- Mutex SQLite hanya dipegang per query pendek. Pada beban satu developer dengan beberapa agent, tidak ada antrean yang terlihat.

## Urutan pengerjaan

1. **No. 1** (distilasi): ini paling berdampak pada nilai produk dan perbaikannya hanya beberapa baris.
2. **No. 3 dan 4** (penulisan config klien): kerjakan dalam satu PR karena sama-sama menyentuh `write_atomic_file`/`backup_file`, lalu sarankan `chmod 600` di catatan rilis.
3. **No. 2** (launcher npm dan `daemon_needs_upgrade`): rilis bersama agar update berikutnya tidak turun versi.
4. **No. 5** (guard supersede) dan **No. 6** (`kill_on_drop`): perubahan satu baris plus tes.
5. Sisanya sesuai waktu.

**Vonis:** Fondasinya sehat. Kerjakan No. 1–5 lebih dulu, dimulai dari distilasi yang membuang sekitar 42% event.

**Lean:** sekitar -130 baris, -0 dependensi.

**Belum dicek:** `src/t3_watcher.rs` (sudah direncanakan); detail `scrub.rs`; sebagian besar halaman UI selain Onboarding dan modal update; isi peringkat pencarian (sudah dibahas di review Memories); plist autostart; `install.sh` dan skrip rilis; path Windows; load test dengan agent sungguhan yang berjalan bersamaan; provider CLI yang tidak benar-benar dijalankan; dan penulisan config klien yang tidak dicoba sungguhan, karena kesimpulannya diambil dari membaca kode dan mode file yang ada.
