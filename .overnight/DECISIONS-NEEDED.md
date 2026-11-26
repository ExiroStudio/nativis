# Decisions Needed (Overnight Defaults Applied)

| ID | Topic | Owner Decision Needed | Overnight Default Applied | Rationale & Fallback |
|---|---|---|---|---|
| W-HANDLE | ID3D11Device channel across plugins | Channel for sharing D3D11Device | Channel via ResourceManager / open context without modifying core D1-D7 ABI | Fallback to CPU-readback |
| W-RECREATE | Renderer recreated (e.g. Explorer restart) | Handling lost device handle | Fallback to CPU readback until backend reopened (+ warn!) | Safe, prevents stale pointer use |
| W-DEFAULT-ON | Zero-copy default state | Default enable zero-copy | OFF by default; opt-in via env var `NATIVIS_D3D11_ZERO_COPY=1` | Safe default without runtime Windows hardware test |
| L-VAAPI-SHIP | VAAPI distribution | Bundle VAAPI in baseline binary | Feature flag `vaapi` opt-in, runtime probe; not in baseline bundle-ffmpeg | Prevents hard DT_NEEDED dynamic link breakage |
| L-DMABUF | DMA-BUF zero-copy Linux | Wayland/X11 dmabuf import | Postponed (out of scope for overnight) | Baseline SHM works reliably |
| L-SEQLOCK | SHM Tearing | Header seqlock version bump | Bump SHM version to 2 with seqlock; update consumers | Avoid torn frames on concurrent read/write |
| L-X11RB | Fullscreen pause on KDE X11 | Dependency on x11rb | Behind feature flag; report dependency tree cost | Keeps default lean |
| P-POOL | Frame pool slot count | Increase planar slot count from 3 | Measured before increasing; avoid gratuitous memory increase | Memory efficiency (D5) |
