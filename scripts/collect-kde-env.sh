#!/usr/bin/env bash
# scripts/collect-kde-env.sh
# Diagnostic environment collector for Nativis KDE Plasma live wallpaper engine.
# Run this script and attach its output when reporting KDE rendering or compositor issues.

set -u

echo "============================================================"
echo " Nativis KDE Environment Diagnostic Report"
echo " Timestamp: $(date -u '+%Y-%m-%dT%H:%M:%SZ')"
echo "============================================================"

echo ""
echo "--- [1] Session & Windowing ---"
echo "XDG_SESSION_TYPE: ${XDG_SESSION_TYPE:-unknown}"
echo "XDG_CURRENT_DESKTOP: ${XDG_CURRENT_DESKTOP:-unknown}"
echo "WAYLAND_DISPLAY: ${WAYLAND_DISPLAY:-unset}"
echo "DISPLAY: ${DISPLAY:-unset}"

echo ""
echo "--- [2] Desktop Components & Versions ---"
if command -v plasmashell &>/dev/null; then
    plasmashell --version 2>&1
else
    echo "plasmashell: not found"
fi

if command -v kwin_wayland &>/dev/null; then
    kwin_wayland --version 2>&1
elif command -v kwin_x11 &>/dev/null; then
    kwin_x11 --version 2>&1
elif command -v kwin &>/dev/null; then
    kwin --version 2>&1
else
    echo "kwin: not found"
fi

echo ""
echo "--- [3] Qt Versions & Libraries ---"
if command -v pkg-config &>/dev/null; then
    echo "Qt6Quick: $(pkg-config --modversion Qt6Quick 2>&1 || echo 'not installed')"
    echo "Qt5Quick: $(pkg-config --modversion Qt5Quick 2>&1 || echo 'not installed')"
fi

echo ""
echo "--- [4] GPU & Hardware Acceleration ---"
if command -v glxinfo &>/dev/null; then
    echo "[OpenGL summary]"
    glxinfo -B 2>&1 || true
fi

if command -v vainfo &>/dev/null; then
    echo "[VA-API summary]"
    vainfo 2>&1 | head -n 30 || true
fi

if command -v lspci &>/dev/null; then
    echo "[VGA / 3D Controller Hardware]"
    lspci -k 2>/dev/null | grep -A 3 -E "(VGA|3D)" || true
fi

echo ""
echo "--- [5] KWin Compositor State ---"
QDBUS_BIN=""
if command -v qdbus6 &>/dev/null; then
    QDBUS_BIN="qdbus6"
elif command -v qdbus &>/dev/null; then
    QDBUS_BIN="qdbus"
fi

if [ -n "$QDBUS_BIN" ]; then
    echo "Querying KWin via $QDBUS_BIN..."
    # Query supportInformation (rate limited / summarized)
    $QDBUS_BIN org.kde.KWin /KWin org.kde.KWin.supportInformation 2>&1 | head -n 40 || echo "KWin D-Bus query returned non-zero"
else
    echo "qdbus/qdbus6 not found; skipping KWin D-Bus query."
fi

echo ""
echo "--- [6] Recommended Diagnostic Environment Flags ---"
echo "To test differential rendering matrices (E4):"
echo "  export NATIVIS_DEBUG_FORCE_RGBA=1       # Force RGBA path on video to isolate NV12 vs 60Hz update"
echo "  export NATIVIS_DEBUG_FRAME_TIMING=1     # Log per-frame paintUs and FPS (rate-limited to 1s)"
echo ""
echo "Qt SceneGraph diagnostics (safe):"
echo "  export QSG_INFO=1                       # Log Qt scene graph renderer backend and threading mode"
echo "  export QSG_RENDER_LOOP=basic            # Test single-threaded render loop (isolates thread sync bugs)"
echo "  export QSG_VISUALIZE=batches            # Visualize scene graph batching"
echo "============================================================"
