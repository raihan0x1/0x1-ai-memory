# Host agent: T3 Code dan Orca untuk `0x1-ai-memory` (keputusan Q10)

> Riset 27 November 2026, sekitar 19.20–19.50 WIB (12.20–12.50 UTC). Checkout `v0.3.5` pada `4f0bda7445082b815026cb096bc89dc607d73301`. Daemon PID 15985 (`~/.0x1-memory/bin/0x1-memory serve`, versi `0.3.5`) mulai 10.01 WIB dan binary-nya memuat string SQL watcher. Aplikasi terpasang: **T3 Code (Beta) `0.1.13`** (`com.t3tools.t3code`, `t3 v0.1.13`) dan **Orca `1.4.279`** (`com.stablyai.orca`). Source resmi yang dibaca: [`pingdotgg/t3code@dc2c7db`](https://github.com/pingdotgg/t3code/tree/dc2c7db0b6c47afb8c07cc9a12d204ab225b2b8e) dan [`stablyai/orca@75627d5`](https://github.com/stablyai/orca/tree/75627d50329bdd9a031152de6d3836b82527dd44), keduanya MIT. Bundle T3 diekstrak dari `app.asar` (`apps/server/dist/*.mjs`). `~/.0x1-memory/memory.db` dan `~/.t3/userdata/statev2.sqlite` (557 MB + WAL 23 MB) disalin ke scratchpad lalu dibuka read-only. Tidak ada config atau DB live yang diubah.

## Ringkasan

| Pertanyaan | Jawaban singkat |
| --- | --- |
| Kenapa watcher T3 tidak pernah menulis? | **Watcher hidup, tetapi rusak.** Watermark terus maju (`_meta:t3_watermark = 2026-11-27T12:21:04.460Z`, sama dengan pesan T3 terbaru), jadi baris memang dibaca lalu dibuang semua. Penyebabnya: `search_text` bernilai NULL di **12.611 dari 12.611** pesan. Kolom itu hanya diisi untuk pesan yang berisi citation link, sedangkan teks ada di `payload_json.text`. Semua baris kena `continue` di [`t3_watcher.rs:121-124`](../../src/t3_watcher.rs#L121). |
| Ada bug lain? | **Ada, dan masih tersembunyi.** Dedup membandingkan ID komposit T3 (`provider-session:provider-instance:claudeAgent:thread:…`) dengan UUID native di `sessions.external_session_id`. Hasilnya **0 cocok dari 34 thread**. Kalau `search_text` terisi, misalnya karena ada satu pesan dengan citation, setiap thread Claude/Codex di T3 akan tercatat **dobel** sebagai `t3-code`. |
| Hook native jalan di dalam T3? | **Claude: ya** (16/16 thread cocok dengan sesi `claude-code`, dan sesi riset ini menerima injeksi SessionStart/UserPromptSubmit serta tool `mcp__0x1-memory__*`). **Codex: ya** (4/4 thread top-level. 12 sisanya berupa `thread:subagent:*` yang memakai `session_id` milik parent). **Antigravity: tidak.** T3 menjalankan `agy acp --gemini-dir <profil privat>`, dan plugin `0x1-memory` tidak di-link ke profil itu. |
| Ada beban kerja untuk watcher? | **Nol sejak daemon ada.** Antigravity di T3 terakhir aktif 17 November. Sesi pertama di `memory.db` baru muncul 25 November 08.56 UTC. Sejak itu T3 hanya menjalankan `claudeAgent` dan `codex`, dan keduanya sudah tertangkap hook native. |
| Orca? | **Hook native ikut jalan.** Mode Terminal menjalankan CLI asli di worktree, dan instalasi baru default ke Terminal. Mode chat untuk Claude memakai Agent SDK dengan `settingSources: ['user','project','local']`. Orca sendiri bergantung pada hook user-level: ia memasang hook status di `~/.claude/settings.json` dan `~/.gemini/config/hooks.json`. Ada dua celah: **chat Antigravity dengan auth non-Google**, yang memakai `GEMINI_HOME` milik Orca, dan akun tambahan Orca, yang belum diverifikasi. |
| Host perlu dicatat sebagai provenance? | **Ya, dan murah.** Proses hook sudah mewarisi `CLAUDE_CODE_ENTRYPOINT=t3-code`/`orca`, `T3CODE_THREAD_ID`, dan `ORCA_PANE_KEY`. Cukup baca env di `run_hook_fast`, tanpa watcher dan tanpa I/O file. |
| **Rekomendasi Q10** | **Hapus watcher dan client `t3-code`.** Hook native sudah menangani Claude/Codex di T3 dan di Orca. Kalau baris handoff dibutuhkan, tambahkan label *host* dari env di shim hook sebagai pekerjaan terpisah. Yang hilang hanya capture teoretis Antigravity/Cursor/Grok/… yang dijalankan di dalam T3, dan capture itu belum pernah berhasil. |

## Verifikasi runtime: korelasi `statev2.sqlite` ↔ `memory.db`

Native ID tiap thread T3 ada di `orchestration_v2_projection_provider_threads.payload_json → $.nativeThreadRef.nativeId`, bukan di kolom `provider_session_id` atau `provider_thread_id`. Join di bawah memakai salinan kedua DB (`ATTACH … mode=ro`).

| Pengukuran (sejak 2026-11-25 08.00 UTC) | Hasil |
| --- | --- |
| Thread T3 per provider (sepanjang waktu) | `claudeAgent` 354, `codex` 44, `antigravity*` 102 (7 instance). Provider lain belum pernah dipakai. |
| Aktivitas Antigravity di T3 | 12–17 November (provider thread terakhir `2026-11-17T09:54Z`). **0/58** native ID ditemukan di `memory.db`, wajar karena daemon belum ada saat itu. |
| `claudeAgent` → `sessions.external_session_id` lewat `nativeId` | **16/16 cocok**, semuanya `source_client = claude-code`. |
| `codex` lewat `nativeId` | **4/16 cocok**. Ke-12 sisanya `thread:subagent:*`. Rollout subagent `01a21470…` punya `session_id = 01a2146f…` (milik parent), jadi hook subagent tercatat di sesi parent. Ini bukan celah T3. |
| Kunci dedup watcher (`provider_session_id`/`provider_thread_id`) | **0/34 cocok** (18 thread Claude, 16 thread Codex). |
| Pesan sejak 25 November | 588 (485 assistant, 103 user), `search_text` non-kosong: **0** |
| `client_connections.t3-code` | `connected=1`, `last_heartbeat_at` NULL, catatan status “Read-only WAL watcher active…”. Status ini menyesatkan karena [`touch_client_activity`](../../src/t3_watcher.rs#L184) hanya dipanggil setelah insert berhasil. |
| Setting | `t3_watcher_enabled=true` (default di [`config.rs:97`](../../src/config.rs#L97)), polling setiap 12 detik ([`t3_watcher.rs:20`](../../src/t3_watcher.rs#L20)) atas DB 557 MB. |

Semantik `search_text` di source resmi T3: [`messageSearchIndex.ts:49`](https://github.com/pingdotgg/t3code/blob/dc2c7db0b6c47afb8c07cc9a12d204ab225b2b8e/apps/server/src/orchestration-v2/messageSearchIndex.ts#L49) mengindeks `COALESCE(search_text, json_extract(payload_json,'$.text'))`. [`readableSearchText`](https://github.com/pingdotgg/t3code/blob/dc2c7db0b6c47afb8c07cc9a12d204ab225b2b8e/apps/server/src/orchestration-v2/messageSearchIndex.ts#L60) mengembalikan `null` jika teks yang terbaca sama dengan teks asli. Migrasi [`081_ProjectionMessageSearchText.ts`](https://github.com/pingdotgg/t3code/blob/dc2c7db0b6c47afb8c07cc9a12d204ab225b2b8e/apps/server/src/persistence/Migrations/081_ProjectionMessageSearchText.ts) hanya mem-backfill pesan yang mengandung `t3-citation://`. Jadi kolom itu memang override yang jarang terisi, bukan salinan teks.

## 1. T3 Code: cara menjalankan tiap provider

Daftar driver bawaan ada di `BUILT_IN_DRIVERS` (`server-B8LwMtOy.mjs:77026`, region `src/provider/builtInDrivers.ts`): Codex, Claude, Cursor, Grok, Fx, OpenCode, Antigravity, Kiro, Pi, Muse, AcpRegistry. Kolom “Hook/MCP” hanya relevan untuk tiga client yang diintegrasikan 0x1.

| Provider | Cara T3 menjalankan | Hook native 0x1 | MCP user | Bukti |
| --- | --- | --- | --- | --- |
| **Claude** (`claudeAgent`) | Agent SDK `query()` yang men-spawn `claude`. Env memakai `CLAUDE_CODE_ENTRYPOINT="t3-code"` dan `T3CODE_THREAD_ID`. Var nesting host dihapus. | **Ya** | **Ya** (ditambah MCP `t3-code` milik T3) | `mergeClaudeProcessEnvironment` (`runtimeLayer-bHa_wuNB.mjs:29448-29462`), `AGENT_THREAD_ENV` (`:30099`). Runtime: 16/16 cocok. `ps eww` pada dua proses `claude` anak T3 menampilkan `CLAUDE_CODE_ENTRYPOINT=t3-code` dan `T3CODE_THREAD_ID=…`. Sesi riset ini juga menerima briefing SessionStart dan tool `mcp__0x1-memory__*`. |
| **Codex** | `codex app-server -c thread_unload_delay_secs=0` dengan satu proses bersama per instance (`provider-session:provider-instance:codex:shared`) dan env `CODEX_INTERNAL_ORIGINATOR_OVERRIDE="T3 Code"` (`extendEnv: true`). | **Ya** | Ya, menurut [client-integrations §4.2](client-integrations.md) untuk 0.1.11. Belum dicek ulang pada runtime 0.1.13. | `codexAppServerArgs` (`runtimeLayer…mjs:65001`), `codexAppServerEnvironment` (`:65549`), `CODEX_CLIENT_NAME = "T3 Code"` (`:65285`). Runtime: 4/4 thread top-level cocok, dan rollout `01a2146f…`/`01a2170a…` mencatat `session_meta.originator = "T3 Code"`. **Catatan:** instance Codex sekarang `enabled: false` di `~/.t3/userdata/settings.json`. |
| **Antigravity** | `agy acp --gemini-dir <~/.t3/userdata/providers/antigravity/<sha256>> --app-data-dir antigravity-acp` dengan `extendEnv: false` dan `GEMINI_HOME` privat. Yang di-link hanya folder skill, `AGENTS.md`, `GEMINI.md`, dan `config/skills.json`. | **Tidak** | **Tidak** | `buildAntigravityAcpSpawnInput` (`server-B8LwMtOy.mjs:22457`), `antigravityUserConfigFiles` (`:22266`). Komentar source: *“Private GEMINI_HOME isolates credentials/hooks/MCP”* ([`antigravityAuthSupport.ts:263`](https://github.com/pingdotgg/t3code/blob/dc2c7db0b6c47afb8c07cc9a12d204ab225b2b8e/apps/server/src/provider/antigravityAuthSupport.ts#L263)). Di disk ada 10 profil. Tidak satu pun punya `hooks.json`. Profil yang dibuka berisi `mcp_config.json` 0 byte, tanpa `plugins/`. Plugin 0x1 berada di `~/.gemini/config/plugins/0x1-memory/` ([`connectors.rs:163`](../../src/connectors.rs#L163)). |
| Cursor | Worker Cursor Agent SDK (`packages/provider-cursor/src/server/CursorAgentSdkProcessRunner.ts`, `server…mjs:77051`) | n/a | tidak dicek | 0x1 belum punya integrasi Cursor |
| Grok | ACP `grok agent stdio` (`grokAcpSpawnArgs`, `runtimeLayer…mjs:41629`) | n/a | tidak dicek | — |
| Fx | ACP (`acp`, `provider-fx/src/server/acpSupport.ts`, `server…mjs:44346`) | n/a | tidak dicek | — |
| Kiro | ACP `acp --agent-engine=v3 --auth-method=cli` (`KIRO_ACP_ARGS`, `server…mjs:76110`) | n/a | tidak dicek | — |
| OpenCode | Server `opencode serve --service` (`provider-opencode/src/server/OpenCodeRuntime.ts`, `server…mjs:57214`) | n/a | tidak dicek | — |
| Pi | CLI `pi` dengan injeksi MCP lewat `--extension` (`provider-pi/src/server/mcpInjection.ts`, `runtimeLayer…mjs:91783`) | n/a | tidak dicek | — |
| Muse | SDK (`provider-muse/src/server/sdk.ts`, `server…mjs:68759`) | n/a | tidak dicek | — |
| ACP registry | Agent ACP generik: claude-acp, copilot, devin, droid, kimi, omp, opencode (`provider-acp-registry/src/server/*Acp.ts`) | n/a | tidak dicek | — |

## 2. T3: titik ekstensi dan stabilitas skema

| Pertanyaan | Temuan |
| --- | --- |
| Sistem hook atau plugin milik T3 untuk capture/injeksi? | **Tidak ada.** Area source 0.1.13 tidak punya modul hook/plugin user. Webhook yang ada hanya pemicu *masuk* untuk scheduled task (`src/scheduledTasks/webhook*.ts`). Injeksi tetap lewat hook client. T3 menambahkan instruksinya sendiri ke prompt sistem Claude dan ke `developer` item Codex (§4.1–4.2 di [client-integrations](client-integrations.md)). |
| API baca resmi | `t3 thread list\|show` (`t3 thread --help`, v0.1.13) membutuhkan server yang sedang jalan. Endpoint MCP “Outside agents” di `https://<env>/mcp` punya scope **Read only** untuk projects/threads dan juga bisa *monitor events*. Sign-in OAuth berlaku 30 hari ([`docs/user/outside-agents.md:3,31`](https://github.com/pingdotgg/t3code/blob/dc2c7db0b6c47afb8c07cc9a12d204ab225b2b8e/docs/user/outside-agents.md)). Keduanya didukung resmi, berbeda dengan membaca SQLite langsung. |
| Skema `statev2.sqlite` stabil atau terdokumentasi? | **Tidak.** Skema ini internal, tanpa dokumen user, dan sering berubah. Jumlah tabel naik dari 54 pada 0.1.11 ([client-integrations §4.4](client-integrations.md)) menjadi 69 pada 0.1.13. Bundle memuat migrasi sampai `119_WorkflowRunSubject`. Semantik kolom ikut berubah (`search_text` di atas). ID provider berbentuk komposit dan baru punya native ID setelah run (`native-thread:pending%3Arun…`). Kolom yang di-query watcher masih ada, tetapi isinya tidak lagi sesuai asumsi watcher. |
| Handoff antar provider di dalam T3 | Sudah ditangani T3 sendiri (“portable handoffs”, budget `T3CODE_CONTEXT_HANDOFF_TOKEN_CAP` default 16.000, [`docs/user/portable-handoffs.md:3,18`](https://github.com/pingdotgg/t3code/blob/dc2c7db0b6c47afb8c07cc9a12d204ab225b2b8e/docs/user/portable-handoffs.md)). Nilai tambah 0x1 ada pada handoff **lintas host**, misalnya dari T3 ke aplikasi Antigravity. |

## 3. Orca

Orca adalah aplikasi Electron untuk *“run a fleet of bots with your own subscription (e.g. Claude Code, Grok, Agy, Codex)”* ([repo](https://github.com/stablyai/orca), situs `orca.dev`). Unit kerjanya worktree. Tiap agent dibuka sebagai **terminal** (CLI asli di PTY) atau **structured chat**. Instalasi baru default ke Terminal ([`native-chat.mdx:8,14`](https://github.com/stablyai/orca/blob/75627d50329bdd9a031152de6d3836b82527dd44/docs/site/content/docs/agents/native-chat.mdx)). `orca agent launch --surface chat|terminal` bisa memilih per launch (`orca agent launch --help`).

| Agent | Terminal | Chat | Hook native dan MCP 0x1 |
| --- | --- | --- | --- |
| **Claude** | `claude` dengan cwd worktree dan memakai `~/.claude` ([`claude-code.mdx:11,15`](https://github.com/stablyai/orca/blob/75627d50329bdd9a031152de6d3836b82527dd44/docs/site/content/docs/agents/claude-code.mdx)) | Agent SDK, `systemPrompt` preset `claude_code`, `settingSources: ['user','project','local']` ([`claude-structured-launch-resolution.ts:78-79`](https://github.com/stablyai/orca/blob/75627d50329bdd9a031152de6d3836b82527dd44/src/main/claude/structured/launch/claude-structured-launch-resolution.ts#L78), [`claude-discovery-options.ts:6`](https://github.com/stablyai/orca/blob/75627d50329bdd9a031152de6d3836b82527dd44/src/main/claude/structured/launch/claude-discovery-options.ts#L6)), dengan `CLAUDE_CODE_ENTRYPOINT=orca` ([`claude-chat-entrypoint.ts:3-4`](https://github.com/stablyai/orca/blob/75627d50329bdd9a031152de6d3836b82527dd44/src/shared/agent-launch/startup/claude-chat-entrypoint.ts#L3)) | **Ya untuk keduanya** (statis). Dua record chat lokal memakai `accountHome = CLAUDE_CONFIG_DIR=/Users/nahiarhd/.claude`. |
| **Codex** | `codex` dengan cwd worktree ([`codex.mdx:15`](https://github.com/stablyai/orca/blob/75627d50329bdd9a031152de6d3836b82527dd44/docs/site/content/docs/agents/codex.mdx#L15)) | `codex app-server` (`src/main/codex/app-server/*`) | **System default: ya**, karena memakai `~/.codex` asli ([`codex.mdx:23`](https://github.com/stablyai/orca/blob/75627d50329bdd9a031152de6d3836b82527dd44/docs/site/content/docs/agents/codex.mdx#L23)). Pane lokal memakai `homeRoute: "real-home"`. **Akun tambahan:** `CODEX_HOME` sendiri di data Orca ([`codex.mdx:23,27`](https://github.com/stablyai/orca/blob/75627d50329bdd9a031152de6d3836b82527dd44/docs/site/content/docs/agents/codex.mdx#L27)). Belum dicek apakah `hooks.json` 0x1 ikut ke home itu. |
| **Antigravity** | `agy` asli yang memakai `~/.gemini` ([`antigravity.mdx:6`](https://github.com/stablyai/orca/blob/75627d50329bdd9a031152de6d3836b82527dd44/docs/site/content/docs/agents/antigravity.mdx#L6)) | ACP `agy`. Chat dengan metode **selain akun Google** berjalan di `--gemini-dir` milik Orca. Di sana skill di-link dan rules disalin, tetapi plugins/hooks/MCP tidak ([`antigravity-agy-orca-home.ts:1-2,32,55-59,100-102`](https://github.com/stablyai/orca/blob/75627d50329bdd9a031152de6d3836b82527dd44/src/main/antigravity/antigravity-agy-orca-home.ts#L1)) | **Terminal: ya** (Orca memasang hook statusnya di `~/.gemini/config/hooks.json`, artinya ia mengandalkan config user). **Chat non-Google: tidak.** |
| Lainnya | Cursor, Grok, Kiro, OpenCode, Droid, Copilot, dan lainnya lewat ACP dialect (`src/main/acp/acp-dialects/*`) | — | Di luar cakupan 0x1 |

| Bukti lokal Orca | Nilai |
| --- | --- |
| `orca agent hooks status --json` | `claude: installed` (`~/.claude/settings.json`), `antigravity: installed` (`~/.gemini/config/hooks.json`), `codex: not_installed` (`~/.codex/hooks.json`) |
| Koeksistensi hook | Entri `0x1-memory hook claude …` (6 event) tetap ada berdampingan dengan hook Orca (16 event, script `~/.orca/agent-hooks/claude-hook.sh`). Saat Orca dipasang pukul 13.20, `~/.gemini/config/hooks.json` ditulis ulang, tetapi plugin 0x1 tidak tersentuh. `codex-real-home-hooks/hooks.json.pre-orca` identik dengan `~/.codex/hooks.json` (10 entri). |
| Pemakaian | `orca-stats.json`: `totalAgentsSpawned: 0`. `agent-session-domain.db` berisi 2 record chat Claude dengan `journal_chat_state.last_sequence = 0`, artinya belum ada turn. **Korelasi runtime Orca tidak bisa dilakukan.** |
| Titik ekstensi | Sistem plugin (`orca plugin create\|install\|dev emit\|dev hook …`), `orca agent hooks status\|on\|off`, `orca agent read`, dan `orca search` untuk full-text sesi agent (`orca --help`). Endpoint hook ditulis ke `{userData}/agent-hooks/endpoint.env` ([`hooks-memory.mdx`](https://github.com/stablyai/orca/blob/75627d50329bdd9a031152de6d3836b82527dd44/docs/site/content/docs/agents/hooks-memory.mdx)). Semua ini tidak dibutuhkan untuk capture karena CLI-nya sudah memanggil hook 0x1. |

## 4. Host sebagai provenance terpisah dari client

**Nilainya nyata, dan host perlu jadi atribut, bukan client.** `source_client` tetap `claude-code`/`codex`/`antigravity`. Host (T3, Orca, terminal, Claude Desktop) menjawab pertanyaan “sesi itu bisa dilanjutkan di mana”. Itu yang dibutuhkan baris seperti *“lanjutan dari Claude Code via T3, 14 menit lalu”* dan contoh motivasi pengguna tentang handoff T3 → Antigravity. Distribusi `entrypoint` di semua transcript Claude lokal menunjukkan host memang beragam: `claude-desktop` 110.915 baris, `t3-code` 32.254, `cli` 2.544, `sdk-cli` 198.

| Host + client | Sinyal termurah di proses hook | Status verifikasi |
| --- | --- | --- |
| T3 + Claude | `CLAUDE_CODE_ENTRYPOINT=t3-code`, `T3CODE_THREAD_ID=<thread T3>`, `T3CODE_DESKTOP_HOST_SCOPE`, `__CFBundleIdentifier=com.t3tools.t3code` | **Runtime.** Dicek di env sesi ini dan `ps eww` dua proses `claude`. Transcript juga mencatat `"entrypoint":"t3-code"`. |
| T3 + Codex | `CODEX_INTERNAL_ORIGINATOR_OVERRIDE="T3 Code"` pada app-server, ditambah env server T3 (`T3CODE_DESKTOP_HOST_SCOPE`, `__CFBundleIdentifier`) lewat `extendEnv: true`. Cadangannya baris pertama rollout di `transcript_path`: `originator = "T3 Code"`. | Rollout: **runtime** (2 sesi). Pewarisan env sampai ke proses hook: **statis**, karena Codex sedang nonaktif di T3. |
| Orca terminal | `ORCA_PANE_KEY`, `ORCA_TAB_ID`, `ORCA_WORKTREE_ID`, `ORCA_TERMINAL_HANDLE`, `ORCA_CLI_COMMAND` | **Statis.** Script hook Orca membaca variabel ini (`~/.orca/agent-hooks/claude-hook.sh:10-11`), begitu juga [`skills/orca-cli/SKILL.md:24,39`](https://github.com/stablyai/orca/blob/75627d50329bdd9a031152de6d3836b82527dd44/skills/orca-cli/SKILL.md). |
| Orca chat Claude | `CLAUDE_CODE_ENTRYPOINT=orca` | **Statis** (source di atas) |
| Terminal biasa | `TERM_PROGRAM` (contoh `ghostty` pada app-server Codex yang berjalan), entrypoint `cli`/`claude-desktop` | Runtime |

- **Payload hook tidak membawa host.** Shim [`run_hook_fast`](../../src/main.rs#L172) meneruskan stdin apa adanya. `raw_events.payload_json` yang tersimpan hanya berisi field bersih (`prompt`, `tool`/`input`/`output`, `assistant`).
- **Proses parent** sebenarnya bisa dipakai (anak `claude` milik T3 punya PPID 63349, proses server T3), tetapi rapuh. Env lebih murah dan sudah disediakan host secara sengaja.
- **Implementasi minimal (di luar Q10):** di `run_hook_fast`, baca 3–4 env di atas, tambahkan `&host=t3-code|orca&host_ref=<T3CODE_THREAD_ID>` ke URL, lalu simpan saat upsert sesi. Perubahan ini menyentuh skema `sessions`, jadi baca `docs/ARCHITECTURE.md` dulu. Tidak perlu watcher.

## 5. Rekomendasi Q10

| Opsi | Yang dikerjakan | Yang hilang | Risiko dan biaya |
| --- | --- | --- | --- |
| **A. Hapus (rekomendasi)** | Hapus `src/t3_watcher.rs`, `pub mod` di [`lib.rs:14`](../../src/lib.rs#L14), spawn di [`main.rs:359`](../../src/main.rs#L359), setting `t3_watcher_enabled` (`config.rs`), kartu `t3-code` (`connectors.rs`, `db.rs:1744,1926`), mapping `t3` beserta test di [`models.rs:374`](../../src/models.rs#L374), [`has_external_session`](../../src/db.rs#L448) (hanya dipakai watcher), referensi UI (5 file), dan dokumen (`ARCHITECTURE.md`, `README.md`, `npm/README.md`). Bersihkan baris `client_connections.t3-code` serta `_meta:t3_watermark`. | Capture *teoretis* Antigravity/Cursor/Grok/OpenCode/Pi/Kiro/Fx/Muse yang berjalan di dalam T3. Hari ini nilainya **nol**: capture itu belum pernah berhasil, dan provider tersebut tidak dipakai sejak 17 November. | Paling kecil. Polling 12 detik atas DB 557 MB hilang, status “watcher active” yang menyesatkan hilang, bug duplikasi laten ikut hilang. Claude/Codex di T3 dan Orca tetap tertangkap lewat hook. |
| B. Pertahankan dan perbaiki | Baca `payload_json.text`, dedup lewat `$.nativeThreadRef.nativeId`, batasi ke provider tanpa hook | Tidak ada | Bergantung pada skema internal yang terus bermigrasi. Hasilnya hanya pesan, tanpa event tool. Tidak ada injeksi, jadi Antigravity di T3 tetap tidak menerima briefing (memori satu arah). |
| C. Ganti dengan MCP “Outside agents” T3 | Klien MCP OAuth ke `/mcp` (read-only + events) | Tidak ada | API resmi, tetapi butuh OAuth (30 hari), server harus jalan, dan kodenya jauh lebih banyak. Baru layak jika pengguna mulai rutin memakai provider tanpa hook di T3. |
| D. Biarkan | — | — | Tidak memberi nilai, statusnya menyesatkan, dan bug dobel bisa aktif begitu ada satu pesan dengan citation. |

**Orca tidak butuh integrasi baru.** Terminal dan chat Claude/Codex dengan akun System default memakai config user, jadi hook dan MCP 0x1 ikut jalan. Label host opsional di §4 sudah mencakup “via Orca”.

### Yang mengubah rencana

- **Antigravity di dalam T3 tidak memicu hook atau MCP 0x1** (dikonfirmasi lagi pada 0.1.13). Jika memori Antigravity penting, jalankan Antigravity di luar T3: aplikasi Antigravity, `agy` di terminal, atau terminal Orca.
- **Chat Antigravity di Orca dengan auth non-Google** punya isolasi yang sama. Terminal Orca tidak terdampak.
- Watcher T3 tidak sekadar “tidak terpakai”. Ia **rusak** (`search_text`) dan **berpotensi mencatat dobel** (dedup ID komposit). Status “active” di halaman Connections tidak akurat.
- Instance Codex di T3 sekarang dinonaktifkan pengguna (`providerInstances.codex.enabled: false`).

## Belum dicek / batasan

- Belum ada agent yang dijalankan di Orca, dan riset ini sengaja tidak menjalankannya. Semua klaim Orca bersifat statis: source dan dokumen `main@75627d5`, ditambah bundle lokal 1.4.279, file hook, dan DB. Source `main` bisa lebih baru daripada build terpasang. Env `ORCA_*` di proses hook dan eksekusi hook 0x1 di Orca belum dibuktikan saat runtime.
- MCP user untuk Codex di T3 0.1.13, serta pewarisan `T3CODE_*`/`__CFBundleIdentifier` ke proses hook Codex, belum diverifikasi saat runtime karena Codex dinonaktifkan di T3. Thread Codex T3 terakhir tercatat 26 November.
- Antigravity di T3 tidak bisa dikorelasikan saat runtime karena terakhir dipakai sebelum daemon ada. Buktinya berasal dari source 0.1.13 dan isi profil privat di disk.
- Cursor, Grok, Fx, Kiro, OpenCode, Pi, Muse, dan registry ACP di T3: hanya mekanisme launch yang dibaca dari bundle. Pemuatan MCP user per provider tidak dicek karena 0x1 tidak punya integrasinya dan `statev2` menunjukkan provider ini belum pernah dipakai.
- Akun tambahan Orca (`CODEX_HOME` per akun, profil `CLAUDE_CONFIG_DIR`) belum diperiksa: apakah `hooks.json`/`settings.json` 0x1 ikut dibawa. Interaksi hook trust Codex saat Orca menambah hook ke `~/.codex/hooks.json` juga belum diuji.
- Salinan DB diambil sekitar 19.21 WIB dari file WAL yang masih aktif, jadi bisa sedikit tertinggal.
- Temuan sampingan yang belum diselidiki: `client_connections` berisi baris non-kanonik `codex-mcp-client`, `antigravity-client`, dan `go-http-client/1.1`.
