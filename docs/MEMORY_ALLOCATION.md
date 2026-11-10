# Temuan Penggunaan RAM Nativis — Ringkasan Percakapan 1 & 2

- **Tanggal analisis:** 2026-11-10
- **Repo:** `ExiroStudio/nativis`, branch `main`, commit `519ec11` (clone shallow, read-only)
- **Keluhan awal:** RAM video 200–400 MB pada kondisi tertentu; RAM gambar 120–200 MB.

---

## 1. Kesimpulan singkat

1. **Gambar:** ada kebocoran memori nyata. `ImageBackend::close()` tidak pernah membebaskan buffer RGBA di `ResourceManager`, sehingga tiap ganti gambar 4K menambah ±32 MB permanen.
2. **Video:** RAM linear terhadap resolusi. Penyebab utamanya decoder software (frame threading 4 thread) ditambah ±5 salinan frame yang hidup bersamaan di sisi repo. Jalur hardware juga belum zero-copy.
3. **FFmpeg tidak berjalan saat menampilkan gambar.** Yang terlihat sebagai "FFmpeg masih jalan" adalah sisa heap yang tidak dikembalikan glibc ke OS.
4. **Single binary bukan penyebab RAM tinggi.** Yang menurunkan RAM adalah **pemisahan proses** (decoder sebagai worker yang bisa dimatikan), dan itu tetap bisa dilakukan dalam satu executable.

---

## 2. Percakapan 1 — Diagnosa sumber RAM

### 2.1 Jalur gambar (diukur)
Harness memakai `ImageBackend` dan `ShmSink` asli dari repo, gambar JPG 4K, ganti wallpaper 5 kali.

| Kondisi | RSS setelah 1 / 2 / 3 / 4 / 5 kali set |
|---|---|
| Kode sekarang | 90 → 122 → 177 → 209 → **241 MB** |
| Jika `close()` membebaskan buffer | 90 → 90 → 90 → 90 → **90 MB** |

Penyebab:
- **Kebocoran:** `ImageBackend::close()` hanya mengosongkan field internal; handle `CpuBuffer` tidak di-`free`. ±32 MB bocor per ganti gambar 4K.
- **Lantai ±90 MB untuk satu gambar 4K:**
  - SHM 31 MB (wajar; region dibuat `3840*2160*4 + 4096` byte).
  - `CpuBuffer` 33 MB yang terus hidup padahal isinya sudah disalin ke SHM.
  - Sisa buffer sementara saat decode (`to_rgba8()` menyalin penuh).
- Perkiraan: 120 MB ≈ lantai + basis daemon; 200 MB ≈ setelah beberapa kali ganti wallpaper.

### 2.2 Jalur video
Catatan: `video_backend` tidak bisa dibuild di lingkungan analisis (header libav tidak ada), jadi decoder software diukur dengan ffmpeg CLI sebagai proxy.

**Decoder software (proxy, angka kasar termasuk kode library ±44 MB):**

| Klip | 1 thread | 4 thread (setting repo) |
|---|---|---|
| 1080p H.264 | 71 MB | 99 MB |
| 4K H.264 | 115 MB | 217 MB |
| 4K HEVC 10-bit | 277 MB | 417 MB |

Lihat bagian 4: angka ini dikoreksi di Percakapan 2 (anon saja).

**Salinan frame di sisi repo (dari kode + simulasi):**
- Satu frame NV12 4K = 12,4 MB.
- Frame yang hidup bersamaan: yang sedang dibuat decoder, 2 di channel `bounded(2)`, `cached_frame`, dan yang terdaftar di `ResourceManager`; ditambah 1 salinan di SHM.
- Simulasi pola ini stabil di **73 MB** (4K).
- Jalur hardware tidak zero-copy: readback GPU→RAM (`av_hwframe_transfer_data`), lalu `Arc::from(slice)` menyalin lagi, lalu salinan ke SHM.
- Konten 10-bit memicu konversi swscale ke NV12 per frame (buffer ekstra).

**"Kondisi tertentu" 200–400 MB** kemungkinan besar adalah mode software: GPU decode gagal lalu `Auto` jatuh ke software secara diam-diam, atau konten 4K/10-bit.

**Risiko lain (dari kode, belum diukur):** IPC `SetWallpaper` memanggil `open()` backend baru (decoder thread langsung jalan) sebelum backend lama di-`close()`, sehingga dua decoder hidup bersamaan sesaat.

**Temuan minor:** tidak ada `shm_unlink` di repo, jadi `/nativis_shm` tetap ada di `/dev/shm` setelah daemon berhenti.

### 2.3 Dugaan yang gugur
Fragmentasi glibc akibat alokasi di thread decoder dan `free` di thread runtime **bukan** penyebab pertumbuhan: simulasi stabil dan pool buffer memberi angka sama (73 MB). Yang menentukan adalah jumlah salinan yang hidup.

Data simulasi (600 frame 4K NV12):

| Varian | RSS akhir |
|---|---|
| Alokasi per frame, glibc default | 73 MB |
| `MALLOC_ARENA_MAX=1` | 81 MB |
| `MALLOC_MMAP_THRESHOLD_=1MB` | 49 MB (puncak 65) |
| Pool buffer | 73 MB |

---

## 3. Percakapan 2 — Linearitas, FFmpeg, dan single binary

### 3.1 RAM linear terhadap resolusi (anon saja, decode software H.264 8-bit)

| Resolusi | Megapiksel | 1 thread | 4 thread |
|---|---|---|---|
| 720p | 0,92 | 19 MB | 36 MB |
| 1080p | 2,07 | 30 MB | 60 MB |
| 1440p | 3,69 | 40 MB | 89 MB |
| 4K | 8,29 | 78 MB | 183 MB |

- Fit linear: **1 thread ≈ 12 MB + 7,9 MB/MP**; **4 thread ≈ 18 MB + 19,9 MB/MP**.
- Setara dengan decoder menahan ±5,5 frame (1 thread) atau ±14 frame (4 thread). Rumus: `RAM ≈ frame_ditahan × W × H × 1,5` byte (8-bit; 10-bit kira-kira dua kali).
- Cara mengendalikan:
  - Jumlah thread decode ditentukan dari resolusi (frame threading menggandakan frame yang ditahan).
  - Hardware decode memindahkan state decoder ke GPU, sehingga RAM proses tidak lagi ikut skala resolusi.
  - Jumlah frame in-flight dibuat tetap (`bounded(1)`, tanpa `cached_frame`).
  - Salinan keluaran diperkecil ke resolusi monitor (video 4K di layar 1080p → salinan 4× lebih kecil; state decoder tetap berukuran sumber).
- **Belum diuji:** slice threading sebagai pengganti frame threading.

### 3.2 Apakah FFmpeg jalan saat menampilkan gambar?
**Tidak.**
- `ffmpeg::init()` hanya dipanggil di `decoder_thread` (`plugins/video_backend/src/lib.rs:515`).
- `PluginManager::open` hanya memanggil `VideoBackend::new()` untuk mengecek `supports()`; itu tidak menyentuh FFmpeg.
- Harness gambar (tanpa FFmpeg) mulai dari 2 MB.

Biaya FFmpeg yang termuat tapi idle (proxy ffmpeg dinamis Ubuntu, lebih berat dari link statis minimal):
- anon **9 MB**
- file-backed (kode library) **34 MB** — dibagi antar-proses dan bisa dibuang OS.

**Sisa heap:** setelah `free`, glibc menyimpan memori. Di harness, anon tetap 56 MB setelah buffer dibebaskan; `malloc_trim(0)` menurunkannya ke 0 (RSS 90 → 34 MB, SHM 31 MB tetap). Jadi `malloc_trim(0)` setelah `close()` adalah perbaikan murah.

### 3.3 Single binary vs banyak file
- Format paket tidak mengubah memori anon yang mendominasi (50–400 MB). Static vs dinamis hanya menggeser halaman kode file-backed.
- Yang menurunkan RAM adalah **pemisahan proses**:
  - Daemon (gambar + SHM + IPC) tetap kecil.
  - Decoder berjalan di proses worker yang hanya hidup saat ada video; saat ganti ke gambar atau pause lama, worker dimatikan dan seluruh 100–400 MB kembali ke OS.
  - Worker menulis langsung ke region SHM yang sudah ada, sehingga salinan `PlanarBuffer`/`Arc` di daemon hilang.
  - Tetap bisa satu executable: pola `ensure_daemon_running` (menjalankan `current_exe()` dengan argumen) bisa dipakai untuk mode `--decode-worker`.
- Banyak file masuk akal hanya untuk alasan lain: kepatuhan lisensi LGPL (FFmpeg dinamis), executable lebih kecil, atau lazy-load. Harganya: ketidakcocokan versi library antar-distro di Linux (bertentangan dengan Rule 5 distro-agnostic) dan penempatan DLL di Windows.

---

## 4. Koreksi angka antar percakapan

| Hal | Percakapan 1 | Percakapan 2 (dipakai) |
|---|---|---|
| 4K H.264, 4 thread | 217 MB | **183 MB** (anon saja) |
| 4K H.264, 1 thread | 115 MB | **78 MB** (anon saja) |
| Basis proses ffmpeg | "±70 MB" | ±44 MB, sebagian besar kode library (file-backed) |

Penyebabnya: `/usr/local/bin/ffmpeg` ternyata launcher ke ffmpeg Ubuntu dinamis dengan puluhan library, sehingga `ru_maxrss` ikut menghitung kode library. Pengukuran kedua memisahkan `RssAnon` dan `RssFile` lewat `/proc`. Angka HEVC 10-bit (277/417 MB) belum diukur ulang dalam bentuk anon saja.

---

## 5. Rencana perbaikan (urut dari dampak terbesar)

1. **`ImageBackend`:** simpan clone `ResourceManager` di `open()` dan `free` handle di `close()` (seperti `VideoBackend`). Setelah submit, bebaskan `CpuBuffer`; pakai `into_rgba8()` atau perkecil gambar ke resolusi monitor. Panggil `malloc_trim(0)` setelah ganti wallpaper (khusus Linux/glibc).
2. **Video, kurangi frame in-flight dan thread:** `bounded(2)` → `bounded(1)`, buang `cached_frame`, tentukan thread decode dari resolusi. Sementara: `NATIVIS_DECODE_THREADS=1`.
3. **Urutan ganti wallpaper:** tutup backend lama sebelum membuka yang baru agar dua decoder tidak hidup bersamaan.
4. **Cek fallback software:** jalankan dengan `RUST_LOG=info` dan cari `[NATIVIS DECODE] Opened ... mode=` (`software` vs `vaapi`).
5. **Proses worker decoder** yang menulis langsung ke SHM (tetap satu executable, mode `--decode-worker`).
6. **Jangka panjang:** zero-copy (DMA-BUF di Linux; D3D11VA di Windows, sejalan dengan celah zero-copy pada review rencana Windows).
7. **Opsional:** `shm_unlink` saat daemon berhenti. Multi-file hanya bila ada alasan lisensi/lazy-load.

---

## 6. Batasan pengukuran

- Video diukur dengan ffmpeg CLI sebagai proxy, bukan `video_backend` asli (tidak bisa dibuild di lingkungan analisis).
- Klip uji dibuat dengan x264 `ultrafast` (referensi minimal, tanpa B-frame); video nyata kemungkinan memakai lebih banyak memori.
- Harness gambar tidak mencakup basis daemon, FFmpeg yang di-bundle, maupun sisi `plasmashell` (plugin Qt).
- Angka salinan frame (±70 MB) berasal dari simulasi pola alokasi, bukan pengukuran proses asli.
- Daemon utuh belum terukur.

## 7. Verifikasi di mesin sendiri

Pembagian memori proses daemon (anon / file-backed / shared):

```bash
grep -E "Rss(Anon|File|Shmem)" /proc/$(pgrep -f 'nativis.*--no-gui' | head -1)/status
```

Cek apakah decoder memakai hardware atau software:

```bash
RUST_LOG=info nativis --no-gui <video> 2>&1 | grep "NATIVIS DECODE"
```

Pertanyaan yang masih terbuka: apakah angka 200–400 MB dibaca dari proses `nativis` (daemon) atau `plasmashell`? Jawabannya menentukan apakah perbaikan di daemon saja sudah cukup.