# Review halaman Stream `0x1-ai-memory`

> Audit 27 November 2026, sekitar 13.05–13.20 WIB (06.05–06.20 UTC). Checkout `v0.3.5` pada `4f0bda7445082b815026cb096bc89dc607d73301`. Daemon di `http://localhost:10101/` melaporkan versi `0.3.5`. SHA-256 aset JavaScript dan CSS yang dilayani sama dengan `ui/dist/` lokal. Root `/` dirender sebagai tab **Stream**. Database dibaca dari salinan `.backup` read-only. [Routing tab](../../ui/src/App.tsx#L30-L38) · [Mount Stream](../../ui/src/App.tsx#L579-L592) · [Aset UI dari biner](../../src/server.rs#L841)

## Ringkasan

| Pertanyaan | Jawaban singkat |
| --- | --- |
| Berjalan? | **Ya.** Semua request awal mengembalikan 200 dengan median di bawah 9 ms. SSE mengalirkan event nyata dan ketiga panel menampilkan data lokal. |
| Benar? | **Sebagian.** Urutan Live event tail salah dan terbukti di browser. Filter klien tidak berlaku sama di tiga panel. Empty state menyuruh onboarding padahal arsip berisi 321 memori. Angka “active sessions” juga terlalu besar. |
| Optimal? | **Cukup untuk volume sekarang, tetapi boros.** Satu event SSE memicu sekitar 8 request. Pada tab yang hanya dibuka tercatat 90 request API dalam 218 detik. Salah satu GET menulis ke SQLite, dan query Sessions makin lambat seiring jumlah sesi. |
| Membantu? | **Sebagian.** Memori terbaru, jumlah recall, budget, dan antrean distilasi berguna. Tail didominasi event tool (75% `tool_use`). Tail juga belum menampilkan recall atau injeksi memori, padahal itu bukti utama bahwa memori benar-benar dipakai agent. |

**Arsitekturnya tepat untuk dashboard lokal.** Tiga panel memakai endpoint berpaging (10 memori, 6 sesi, dan 20 event), lalu SSE dipakai sebagai sinyal invalidasi. Commit `ee5f177` memindahkan browse memori ke `COUNT(*)` + `LIMIT/OFFSET` dalam satu transaksi SQL. Karena itu `Recent memories (N)` sekarang cocok dengan metrik **Active memories**. Panel ini juga memakai predikat status dan proyek yang sama dengan halaman Memories. Perbaikan yang dibutuhkan ada di cara UI menggabungkan data live, cakupan filter, dan frekuensi refetch. Halaman ini tidak perlu ditulis ulang. [Loader halaman](../../ui/src/pages/StreamPage.tsx#L231-L316) · [Browse SQL](../../src/db.rs#L1943-L1974) · [Predikat filter](../../src/db.rs#L1991-L2007) · [Metrik total](../../src/db.rs#L1685-L1689)

## Verifikasi runtime

### Browser dan SSE

| Alur | Hasil |
| --- | --- |
| Load awal | Header, empat metrik, 10 memori dalam grup `Today · 2026-11-27 UTC`, 6 sesi, dan 14 baris tail dari 20 event. Ada agregasi `Bash x5` dan `x2`. Pager menunjukkan 32, 12, dan 151 halaman. Tidak ada callout error. |
| Urutan tail | Setelah beberapa menit, sembilan baris pertama bertanggal 06:13:03–06:18:59 UTC. Baris berikutnya, `run_command x9`, bertanggal 06:19:42 UTC. Jadi event lama tampil di atas event baru. |
| Filter **T3 Code** | Muncul `Recent memories (0)` dan “No memories recorded yet… Open onboarding”. Sessions tetap `(67)` dan berisi sesi `antigravity`/`claude-code`. Tail berjudul `(0)`, tetapi menampilkan tiga baris `distill_ok`. API mengembalikan total 0 untuk `t3-code`, `ui`, dan `mcp`. |
| By Session | Ada enam grup. Dua pasang grup berlabel identik, misalnya dua grup `claude-code · 0x1-ai-memory (main)`, tanpa waktu mulai sebagai pembeda. |
| Cadence request | Dalam 218 detik setelah navigasi, Resource Timing mencatat 90 request API: `memories` 4, `sessions` 13, `raw-events` 13, dan enam endpoint shell masing-masing 10 kali. |
| SSE via `curl` 90 detik | Tercatat 7 event (5 `raw_event`, 1 `usage_recorded`, 1 `memory_created`), 3 keep-alive, dan 0 baris `id:`. Field `data` berukuran 9,3 kB, sekitar 1,3 kB per event. |

[Merge tail](../../ui/src/pages/StreamPage.tsx#L408-L442) · [Filter tail](../../ui/src/pages/StreamPage.tsx#L105-L128) · [Empty state](../../ui/src/pages/StreamPage.tsx#L731-L752) · [Grup sesi](../../ui/src/pages/StreamPage.tsx#L381-L406) · [SSE dan refresh](../../ui/src/App.tsx#L253-L315) · [Handler SSE](../../src/server.rs#L240-L272)

### Latensi dan payload

GET lewat loopback, 15 request berurutan setelah satu warm-up. Waktu diukur sampai body diterima penuh. Data pada saat itu: 3.004 raw event (25 Nov 09.01 sampai 27 Nov 06.11 UTC), 66 sesi, 336 memori (315 aktif), 296 baris usage, dan 13 proyek.

| Endpoint | Median | Maks | Body |
| --- | ---: | ---: | ---: |
| `/api/memories?limit=10` | 3,31 ms | 3,88 ms | 8,5 kB |
| `/api/memories` halaman 2 | 2,65 ms | 4,26 ms | 8,4 kB |
| `/api/sessions?limit=6` | **8,17 ms** | 10,97 ms | 2,2 kB |
| `/api/raw-events?limit=20` | 3,29 ms | 4,93 ms | 8,7 kB |
| `/api/raw-events` `kind=tools` | 5,27 ms | 7,69 ms | 8,6 kB |
| `/api/raw-events` `client=claude-code` | 4,46 ms | 9,25 ms | 8,5 kB |
| `/api/connections` | **6,74 ms** | 8,15 ms | 2,1 kB |
| `/api/summary` | 0,99 ms | 1,49 ms | 0,5 kB |
| `projects` / `settings` / `daemon` / `updates/status` | 0,64–1,28 ms | ≤1,76 ms | 0,4–3,7 kB |

Load awal memindahkan sekitar 37 kB JSON. Bundle 395,6 kB JS dan 42,3 kB CSS tanpa kompresi sudah dibahas di review Memories dan tidak diulang di sini. Di salinan DB, `EXPLAIN QUERY PLAN` query Sessions menunjukkan `CORRELATED SCALAR SUBQUERY` yang memakai `SCAN r USING COVERING INDEX idx_raw_events_pending` untuk `total_events`, `SCAN m` untuk hitungan memori, dan `USE TEMP B-TREE FOR ORDER BY`. Hitungan raw event berfilter memakai `SCAN r`. Usage harian memakai `SCAN usage_ledger`. [Review Memories: bundle](memories-page-review.md#latensi-dan-payload) · [SQL Sessions](../../src/db.rs#L479-L522) · [SQL raw event](../../src/db.rs#L617-L673) · [Summary](../../src/db.rs#L1667-L1706) · [SQLite EXPLAIN QUERY PLAN](https://www.sqlite.org/eqp.html)

## 1. Kebenaran data live

**P1 · Gabungkan tail live berdasarkan waktu, bukan dengan menaruh buffer di atas.** Di halaman 1, `liveTail` mengambil sampai 10 item dari buffer SSE (maksimal 80 item) yang ID-nya belum ada di 20 baris DB. Item itu lalu ditempel di depan baris DB. Raw event yang sudah tergeser keluar dari halaman 1 ikut memenuhi syarat tersebut, sehingga kembali naik ke posisi teratas. Event itu juga muncul lagi di halaman 2. Item `usage_recorded` dan `memory_*` punya ID sintetis `sse_*`, jadi selalu dianggap “ekstra” sampai keluar dari buffer. Akibatnya urutan kronologis rusak (lihat tabel browser), halaman 1 bisa berisi sampai 30 baris, dan judul `(N)` tidak cocok dengan isinya. `loadTail` sudah dipanggil ulang pada setiap tick SSE di halaman 1. Karena itu buffer cukup dipakai untuk item non-raw. Item buffer hanya boleh lebih baru dari `rawEvents[0].created_at`, lalu digabung dan diurutkan berdasarkan `timestamp`. Uji dengan 25 raw event baru setelah satu `usage_recorded`, dan pastikan urutan menurun serta tidak ada ID ganda antara halaman 1 dan 2. [Merge tail](../../ui/src/pages/StreamPage.tsx#L408-L440) · [Buffer SSE](../../ui/src/App.tsx#L280-L297) · [ID sintetis](../../ui/src/App.tsx#L102-L141)

**P1 · Hanya respons terbaru yang boleh mengisi panel.** `loadMemories`, `loadSessions`, dan `loadTail` tidak memakai `AbortController` maupun nomor urut. Setiap `raw_event` memicu `loadSessions` dan `loadTail` tanpa debounce. Saat agent sedang aktif, beberapa request untuk panel yang sama bisa berjalan bersamaan. Respons yang selesai terakhir menang, termasuk respons dari filter sebelumnya. Baris tail sebagian tertutupi karena `matchesTailFilters` menyaring ulang di klien, tetapi `tailTotal`, daftar memori, dan daftar sesi tidak ikut disaring. `api.searchMemories` sudah menerima `signal` sejak `ee5f177`, tetapi Stream belum memakainya. Temuan ini sama dengan P1 di review Memories. Pola perbaikannya juga sama: cleanup effect yang membatalkan request lama. Race ini diverifikasi dari source dan belum direproduksi dengan penundaan buatan. [Loader](../../ui/src/pages/StreamPage.tsx#L231-L304) · [Effect tick](../../ui/src/pages/StreamPage.tsx#L318-L359) · [Signal di api](../../ui/src/api.ts#L87-L90) · [Review Memories P1](memories-page-review.md#2-paging-dan-respons-ui) · [React: fetching data](https://react.dev/learn/synchronizing-with-effects#fetching-data)

**P2 · Satukan cakupan filter dan bedakan “kosong karena filter” dari “belum ada data”.** Filter klien dikirim ke memori dan tail, tetapi tidak ke Sessions. `SessionsQuery` memang tidak punya parameter klien. Item `distill` selalu lolos filter klien, dan item dengan `project_id` null lolos filter proyek. Pada data ini, opsi **T3 Code**, **MCP**, dan **Manual UI** menghasilkan 0 memori dan 0 event. Penyebabnya, event T3 tercatat atas nama driver Claude atau Codex. Empty state lalu menampilkan “No memories recorded yet” dengan tombol **Open onboarding**. Saat filter aktif, tampilkan “Tidak ada memori untuk filter ini” dengan aksi **Clear filters**, seperti di halaman Memories. Tambahkan `client` ke `/api/sessions` atau beri label bahwa panel Sessions tidak ikut difilter. Isi opsi klien dari data yang ada. Satu memori di arsip berlabel `go-http-client/1.1` karena fallback User-Agent MCP. [Opsi klien](../../ui/src/pages/StreamPage.tsx#L47-L55) · [Filter tail](../../ui/src/pages/StreamPage.tsx#L105-L114) · [Sessions query](../../src/server.rs#L465-L492) · [Fallback User-Agent](../../src/server.rs#L189-L196) · [Integrasi T3](client-integrations.md)

**P2 · Hitung sesi aktif dari aktivitas terakhir.** Status sesi hanya berubah menjadi `ended` saat hook `SessionEnd`. Antigravity hanya punya enam event hook dan tidak satu pun berupa `SessionEnd`. Akibatnya 44 sesi Antigravity tetap `active`. Dari 47 sesi “active”, 26 terakhir aktif lebih dari satu hari sebelum snapshot. Metrik **47 active sessions** dan tag `active` per baris jadi menyesatkan. Turunkan status dari `last_turn_at` dibanding ambang idle, atau tutup sesi lewat sweep worker. [End session](../../src/hooks.rs#L254-L256) · [Hitungan](../../src/db.rs#L1702-L1706) · [Event Antigravity](client-integrations.md#L17) · [Tag UI](../../ui/src/pages/StreamPage.tsx#L981-L983)

**P2 · Samakan arti chip `Distill` antara server dan UI.** Di server, `kind=distill` berarti `event_type = 'turn_stop'`, yaitu pesan akhir assistant. Hasil distilasi (`usage_recorded`, `memory_created`) hanya ada di buffer SSE. Hasil itu hilang setelah reload atau setelah 80 event berikutnya, sehingga riwayat “Distill” setelah refresh hanya berisi turn stop. Ambil hasil distilasi dari `usage_ledger` dan pembuatan memori, atau ganti label chip menjadi **Turn end** dan tambahkan chip terpisah untuk hasil distilasi. [Kind clause](../../src/db.rs#L628-L633) · [Filter klien](../../ui/src/pages/StreamPage.tsx#L119-L126) · [Ledger](../../src/db.rs#L153-L166)

**P3 · Urutkan “Recent memories” sesuai namanya.** Urutan default menaruh `pinned` lebih dulu, baru `updated_at DESC`. Pengelompokan **By Day** memakai urutan pertama kali muncul di `Map`. Memori lama yang di-pin, termasuk yang di-pin dari tombol **Pin** di halaman ini, akan berada di atas grup `Today` dengan tanggalnya sendiri. Snapshot ini tidak berisi memori pinned, jadi efeknya belum terlihat. Untuk Stream, kirim `sort=recent` tanpa prioritas pin, atau tampilkan pin di seksi terpisah. Beri waktu mulai pada label grup sesi agar dua sesi pada repo dan branch yang sama bisa dibedakan. [Order](../../src/db.rs#L1935-L1941) · [Grouping](../../ui/src/pages/StreamPage.tsx#L381-L406)

**P3 · Sinkronkan ulang setelah SSE tersambung kembali.** Server tidak mengirim `id:`, sehingga browser tidak punya `Last-Event-ID` untuk dilanjutkan. Event yang tertinggal karena `Lagged` dilewati tanpa jejak. `onopen` hanya mengubah indikator. Setelah daemon restart atau update, panel tetap basi sampai ada event berikutnya. Panggil ketiga loader dan `refresh()` pada `onopen` kedua dan seterusnya. [onopen](../../ui/src/App.tsx#L258-L262) · [Lagged](../../src/server.rs#L258-L266) · [Kapasitas 256](../../src/main.rs#L355) · [HTML: Last-Event-ID](https://html.spec.whatwg.org/multipage/server-sent-events.html#the-last-event-id-header) · [Tokio broadcast lagging](https://docs.rs/tokio/latest/tokio/sync/broadcast/index.html#lagging)

## 2. Biaya per event dan query

**P1 · Satu event SSE memicu terlalu banyak kerja.** Pada halaman 1, setiap `raw_event`, `usage_recorded`, atau `memory_created` langsung memicu `/api/sessions` dan `/api/raw-events`. Setelah jeda 250 ms, `refresh()` menjalankan enam GET shell: `summary`, `projects`, `settings`, `daemon`, `connections`, dan `updates/status`. Hasil terukur adalah 90 request dalam 218 detik pada tab yang hanya dibuka. `GET /api/connections` membaca konfigurasi klien dari disk, termasuk `~/.claude.json` sebesar sekitar 273 kB di mesin ini, lalu menulis empat UPSERT ke `client_connections` di setiap panggilan. Semua query memakai satu mutex koneksi SQLite yang juga dipakai hook. Selain itu tidak ada penanganan `visibilitychange`, sehingga tab di latar belakang tetap melakukan refetch. Langkah perbaikannya:

1. Kumpulkan tick per panel dengan debounce 250–500 ms.
2. Lewati refetch saat tail di-pause atau dokumen tersembunyi, lalu sinkronkan saat tab terlihat lagi.
3. Segarkan endpoint shell sesuai jenis event, sesuai P2 SSE di review Memories.
4. Jadikan `GET /api/connections` read-only. Pindahkan UPSERT ke jalur connect dan heartbeat.

[Tick dan refresh](../../ui/src/App.tsx#L207-L230) · [Listener](../../ui/src/App.tsx#L280-L297) · [inspect_all](../../src/connectors.rs#L85-L98) · [UPSERT](../../src/db.rs#L1759-L1771) · [Review Memories P2 SSE](memories-page-review.md#2-paging-dan-respons-ui) · [Page Visibility](https://www.w3.org/TR/page-visibility-2/)

**P2 · Bangun halaman Sessions sebelum menghitung agregat.** Query menghitung tiga subquery berkorelasi per sesi dan mengurutkan lewat temp B-tree. Kolom `total_events` tidak ditampilkan di UI, tetapi menjadi biaya terbesar. Di salinan DB, waktu query turun dari 1,71 ms menjadi 0,26 ms tanpa kolom itu. `memories.session_id` tidak punya indeks, dan tabel `sessions` tidak pernah dipangkas. Pada fixture sintetis dengan 12.000 sesi, 21.000 raw event, dan 30.000 memori, query sekarang butuh 32,4 ms. Memilih 6 sesi lebih dulu lewat CTE tanpa `total_events` menurunkannya ke 7,2 ms. Dengan indeks `memories(session_id)` dan `sessions(last_turn_at)`, waktunya di bawah 0,1 ms. Query ini berjalan pada setiap event dan berada di bawah mutex yang sama dengan hook. Pertahankan tes paging yang sudah ada, lalu tambahkan fixture ber-volume besar. [SQL Sessions](../../src/db.rs#L479-L522) · [Indeks sekarang](../../src/db.rs#L85-L120) · [Field UI](../../ui/src/pages/StreamPage.tsx#L986-L990) · [Tes paging](../../tests/integration.rs#L1512-L1567) · [SQLite query planner](https://www.sqlite.org/queryplanner.html)

**P3 · Ganti paging offset di tail dengan “Load older”.** Event baru menggeser `OFFSET`, sehingga halaman 2 dan seterusnya bergeser saat dibaca. Banner memang memperingatkan hal ini. Untuk feed live, 151 halaman event tool jarang berguna. Gunakan keyset `created_at < ?` dan tampilkan total kasar atau total per filter. Hitungan berfilter memakai `SCAN r` (0,5–0,6 ms pada 3.004 baris) dan dibatasi retensi 14 hari. Usage harian memakai `created_at LIKE 'YYYY-MM-DD%'` sehingga tidak bisa memakai `idx_usage_created`. Biayanya kini 0,02 ms dan ledger tidak pernah dipangkas. Predikat rentang `>= hari ini AND < besok` cukup. [Raw paging](../../src/db.rs#L634-L653) · [Pager tail](../../ui/src/pages/StreamPage.tsx#L1139-L1149) · [Usage LIKE](../../src/db.rs#L1673-L1681) · [SQLite scrolling window](https://www.sqlite.org/rowvalue.html#scrolling_window_queries) · [SQLite LIKE optimization](https://www.sqlite.org/optoverview.html#the_like_optimization)

## 3. Kegunaan bagi pengguna

**P2 · Tampilkan recall dan injeksi memori sebagai event.** Halaman ini menunjukkan apa yang direkam, tetapi belum menunjukkan kapan memori dipakai. Hook `UserPromptSubmit` mencatat recall ke `memory_recall_stats` dan menyuntikkan konteks, sedangkan briefing `SessionStart` juga menyuntikkan konteks. Keduanya tidak mengirim `StreamEvent`. Padahal 265 dari 315 memori aktif pernah di-recall (926 recall total), dan pola ini justru membuktikan memori lintas agent berfungsi. Kirim event `memory_recalled`/`briefing_injected` dengan klien, proyek, dan ID memori, lalu tambahkan chip **Recall** di tail. [Recall hook](../../src/hooks.rs#L134-L167) · [SessionStart](../../src/hooks.rs#L83-L100) · [Jenis event](../../src/models.rs#L305-L306)

**P2 · Kurangi noise tool di tail.** Dari 3.004 event, 74,9% adalah `tool_use`, 16,5% `tool_error`, 7,0% `prompt`, dan 1,6% `turn_stop`. Agregasi hanya menggabungkan tool sama yang berurutan dalam satu sesi, misalnya `run_command x9`. Perintah baca seperti `grep` atau `sed` lewat `Bash` tetap masuk karena filter read-only hanya mengenali nama tool. Jadikan prompt, error, hasil distilasi, dan recall sebagai tampilan default. Ringkas tool per turn dengan jumlah dan beberapa file atau perintah, lalu buka detail saat diklik. [Filter tool](../../src/hooks.rs#L187-L197) · [Agregasi](../../ui/src/pages/StreamPage.tsx#L83-L103)

**P3 · Beri alasan pada error tool Antigravity.** Ada 496 `tool_error` dari Antigravity (`run_command` 321, `edit_file` 158). Sebanyak 480 menyimpan `output` kosong. Kode memilih `tool_response`/`tool_output` sebelum `error`, sehingga pesan error tidak tersimpan jika field output ada tetapi kosong. Kemungkinan lain, `error` hanya berisi nilai `true`. Rekam satu payload hook mentah untuk memastikannya, lalu simpan teks error bila ada. Baris merah tanpa alasan sulit ditindaklanjuti. [Klasifikasi dan payload](../../src/hooks.rs#L199-L213) · [Deteksi error](../../src/hooks.rs#L263-L272)

**P3 · Rapikan detail tampilan.** Ada beberapa perbaikan kecil:

- Semua waktu konsisten berlabel UTC. Untuk pengguna di WIB, batas grup `Today` jatuh pada pukul 07.00 lokal. Opsi waktu lokal dengan UTC di tooltip akan lebih mudah dibaca.
- `Daily spend` tampil dua kali, di topbar dan di metrik.
- Istilah `zero silent fallback` dan `Zero-LLM safe mode` perlu penjelasan singkat. Review Engine sudah membahas label ini.
- Sebanyak 16 payload menjadi JSON tidak valid setelah pemotongan head/tail 6.000 karakter di `insert_raw_event`. Preview baris itu jatuh kembali ke nama tool saja. Potong per field sebelum serialisasi.

[Format UTC](../../ui/src/api.ts#L191-L234) · [Metrik](../../ui/src/pages/StreamPage.tsx#L543-L619) · [Scrub payload](../../src/db.rs#L539) · [Preview](../../src/db.rs#L2401-L2440) · [Review Engine](engine-settings-review.md#urutan-pengerjaan)

**P3 · Tangani error aksi dan konfirmasi perubahan privasi.** `handlePrivacyChange` dan `handleTogglePin` tidak menangkap error. Jika gagal, terjadi unhandled rejection tanpa umpan balik. Selector privasi berada di toolbar filter dan langsung berlaku. Pilihan `Paused` menghentikan capture untuk proyek itu. Beri status sukses atau gagal, dan minta konfirmasi untuk `Paused`. [Handler](../../ui/src/pages/StreamPage.tsx#L485-L496) · [Toolbar](../../ui/src/pages/StreamPage.tsx#L668-L679) · [Paused di hook](../../src/hooks.rs#L116)

## 4. Privasi

**P2 · Kirim preview saja ke browser dan sediakan mode samaran.** Guard Host/Origin membatasi akses ke localhost, dan payload sudah melewati `redact_secrets` sebelum disimpan. Ada tiga celah yang tersisa:

- **SSE mengirim seluruh `payload_json`.** Isinya mencakup output tool, rata-rata 1,0 kB dan maksimal 5.993 karakter. UI hanya memakai 120 karakter.
- **Tail menampilkan prompt dan perintah apa adanya.** Pada audit ini tail memperlihatkan prompt yang berisi pesan chat pihak ketiga dengan nama orang. Redaksi hanya menangani pola rahasia dan tag `<private>`.
- **Ada string berbentuk kredensial yang lolos redaksi.** Di salinan DB ada 5 payload dengan pola yang tidak tercakup aturan: header `X-API-Key:`, userinfo URL `postgresql://user:pass@`, dan `*_TOKEN=` di luar `ACCESS_TOKEN`/`AUTH_TOKEN`. Satu di antaranya tampak placeholder. Tidak ada yang masuk preview 120 karakter, tetapi semuanya tersimpan dan ikut terkirim lewat SSE. Nilainya tidak diperiksa lebih jauh.

Saran perbaikan:

- Kirim `preview` hasil server lewat SSE.
- Tambahkan toggle untuk menyamarkan prompt saat berbagi layar.
- Perluas aturan redaksi dengan tes untuk ketiga pola di atas.

[Guard](../../src/server.rs#L150-L173) · [Event raw](../../src/hooks.rs#L116-L131) · [Payload tool](../../src/hooks.rs#L206-L227) · [Aturan redaksi](../../src/scrub.rs#L12-L76) · [Preview klien](../../ui/src/App.tsx#L40-L64)

**P3 · Host font secara lokal.** Setiap load dashboard lokal ini menghubungi `api.fontshare.com` dan `fonts.googleapis.com`. Kedua host tercatat di Resource Timing. Font juga gagal dimuat saat offline. Untuk produk yang menyebut dirinya local-first, sertakan font di `ui/dist`. [index.html](../../ui/index.html#L13-L19) · [Tujuan local-first](../ARCHITECTURE.md#L3) · [Google Fonts privacy FAQ](https://developers.google.com/fonts/faq/privacy)

## Urutan pengerjaan

1. Perbaiki merge tail: urut waktu, tanpa duplikat, dan judul yang sesuai isi. Satukan cakupan filter, tambahkan empty state khusus filter, dan pasang guard respons terbaru di tiga loader. Semua ini perubahan UI kecil di `StreamPage.tsx` dan bisa dicek dengan skenario 25 event setelah satu distilasi.
2. Kumpulkan refetch per panel, hentikan refetch saat pause atau tab tersembunyi, segarkan shell sesuai jenis event, dan jadikan `GET /api/connections` read-only. Ukur ulang jumlah request per menit dengan agent aktif.
3. Ubah query Sessions menjadi page-first tanpa `total_events`, tambahkan indeks `memories(session_id)`, dan turunkan status aktif dari `last_turn_at`. Tambahkan fixture besar ke tes integrasi.
4. Tambahkan event recall/injeksi dan hasil distilasi yang persisten, lalu jadikan tampilan sinyal sebagai default tail. Bersamaan dengan itu, kirim preview saja lewat SSE, perluas redaksi, dan host font secara lokal.

## Belum dicek / batasan

- Browser yang dipakai adalah Lightpanda headless, bukan Chrome. Indikator awal menampilkan `SSE offline` lalu berubah menjadi `SSE live`. Waktu event loop Lightpanda berbeda, sehingga ini tidak dihitung sebagai temuan. FCP, layout mobile 320/390 px, screen reader, dan keyboard belum diuji pada halaman ini.
- Race respons lama diverifikasi dari source dan belum direproduksi dengan penundaan jaringan buatan. Beban berkelanjutan dengan beberapa agent aktif belum diukur.
- Aksi tulis **Save memory**, **Pin**, **Privacy**, **Distill now**, dan **Flush pending** tidak dijalankan. Namun pengukuran `GET /api/connections` (16 kali lewat `curl`, ditambah refresh dari browser) ikut menjalankan UPSERT `client_connections` pada DB live. Ini sama dengan yang dilakukan UI. Tidak ada tulisan lain ke DB live. Kueri SQL dijalankan pada salinan `.backup` read-only dan fixture in-memory.
- Bentuk payload hook mentah Antigravity untuk `tool_error` belum ditangkap. Keaslian lima string berbentuk kredensial tidak diperiksa, dan nilainya sengaja tidak dicetak.
- Fixture skala memakai distribusi sintetis: 12.000 sesi, 21.000 event pada 500 sesi terbaru, dan 30.000 memori. Angkanya menunjukkan arah, bukan p95 produksi.
- Anchor dokumentasi eksternal (React, HTML SSE, Tokio, SQLite, W3C, Google Fonts) dikutip dari halaman resmi tanpa diambil ulang pada sesi ini.
