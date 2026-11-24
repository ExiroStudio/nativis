# Morning Report — Overnight Run (2026-11-23 / 2026-11-24)

## TL;DR
- **KDE Ghosting/Trail Bug (P0)**: Under investigation. Mutex held across blocking futex wait in SHM consumer (T4) identified and proven via deterministic test (E1). State leakage in Qt Quick SceneGraph (T3) and lifetime issues analyzed.
- **Cross-Repo Audit (P1)**: Baseline established (22 passed tests, clean clippy warnings identified). Full checklist execution underway.
- **Performance & GPU Decode (P2)**: Windows D3D11VA zero-copy conservative design mapped; Linux VAAPI runtime probe & SHM seqlock planned for strict platform parity.

## Git Commits
*(Will be populated per commit)*

## Platform Parity Matrix
*(Will be populated across Windows, KDE X11, KDE Wayland)*

## Checkpoints
- `checkpoint/p0`: Pending
- `checkpoint/p1`: Pending
- `checkpoint/p2`: Pending

## Status P0
- Proven defects: Pending test execution
- Open hypotheses: H1-H6
- Verification procedures: See scripts/ and E1-E6

## Unverified Aspects
*(List of components needing owner target hardware testing)*

## Parked Decisions
See `DECISIONS-NEEDED.md`.

## Unfixed Findings
See `FINDINGS.md`.

## Risks & Uncertainties
*(To be detailed)*
