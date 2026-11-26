#ifndef NATIVISITEM_H
#define NATIVISITEM_H

#include <QQuickItem>
#include <QThread>
#include <QAtomicInteger>
#include <QtGlobal>
#include <QDebug>
#include <QDateTime>

extern "C" {
    uint32_t nativis_version();
    void*    nativis_create();
    void     nativis_destroy(void* ctx);
    bool     nativis_begin_frame(void* ctx, int width, int height);
    uint8_t* nativis_get_pixels(void* ctx);
    int      nativis_get_width(void* ctx);
    int      nativis_get_height(void* ctx);
    void     nativis_render(void* ctx);
    void     nativis_end_frame(void* ctx);
    uint64_t nativis_get_frame_id(void* ctx);  // Phase 1: frame change detection
    uint64_t nativis_get_epoch(void* ctx);
    uint64_t nativis_wait_frame(void* ctx, uint64_t last_seen_id, uint32_t timeout_ms); // Item 5: futex wait
    void     nativis_wake(void* ctx);
    void     nativis_resume_wait(void* ctx);

    // Fase 5: Generic plane-based FFI
    uint32_t nativis_get_format(void* ctx);
    uint32_t nativis_get_plane_count(void* ctx);
    uint8_t* nativis_get_plane(void* ctx, uint32_t index,
                               uint32_t* out_stride, uint32_t* out_width, uint32_t* out_height);
}

// Protocol constants (must match nativis-protocol)
#define NATIVIS_PROTOCOL_VERSION 4
#define NATIVIS_FORMAT_RGBA8888  1
#define NATIVIS_FORMAT_NV12      2
#define NATIVIS_FORMAT_P010      3

// ---------------------------------------------------------------------------
// FrameWatcher — Phase 1 & Item 5
//
// Runs on a dedicated thread. Blocks on nativis_wait_frame() (Linux futex on
// lower 32-bits of frame_id at SHM offset 8) and emits newFrameAvailable()
// only when the id or epoch changes.
// ---------------------------------------------------------------------------
class FrameWatcher : public QThread
{
    Q_OBJECT
public:
    explicit FrameWatcher(void* runtimeCtx, QObject* parent = nullptr)
        : QThread(parent), m_ctx(runtimeCtx), m_running(true) {}

    void stop() {
        m_running.store(false, std::memory_order_seq_cst);
        if (m_ctx) {
            nativis_wake(m_ctx);
        }
    }

signals:
    void newFrameAvailable(quint64 frameId);

protected:
    void run() override {
        quint64 lastSeen = 0;
        quint64 lastEpoch = 0;
        int debugEmitCount = 0; // TEMP: cap logging so video framerate doesn't flood journalctl

        while (m_running.load(std::memory_order_seq_cst)) {
            quint64 current = nativis_wait_frame(m_ctx, lastSeen, 1000);
            quint64 epoch = nativis_get_epoch(m_ctx);
            if (!m_running.load(std::memory_order_seq_cst)) {
                break;
            }
            if (current != 0 && (current != lastSeen || (epoch != 0 && epoch != lastEpoch))) {
                lastSeen = current;
                if (epoch != 0) lastEpoch = epoch;
                if (debugEmitCount < 20) {
                    qDebug() << "[nativis-debug] FrameWatcher emit newFrameAvailable, frameId=" << current
                              << "t=" << QDateTime::currentMSecsSinceEpoch();
                    ++debugEmitCount;
                }
                emit newFrameAvailable(current);
            }
        }
    }

private:
    void*                    m_ctx;
    std::atomic<bool>        m_running;
};

// ---------------------------------------------------------------------------
// NativisItem
// ---------------------------------------------------------------------------
class QSGTexture;
class QQuickWindow;

class NativisItem : public QQuickItem
{
    Q_OBJECT
public:
    explicit NativisItem(QQuickItem *parent = nullptr);
    ~NativisItem() override;

protected:
    QSGNode *updatePaintNode(QSGNode *oldNode, UpdatePaintNodeData *) override;
    void itemChange(ItemChange change, const ItemChangeData &value) override;
    void releaseResources() override;

private:
    void scheduleOrDeleteTextures(QQuickWindow *win);

    void* m_runtimeCtx = nullptr;

    // Phase 1 — event-driven rendering
    FrameWatcher* m_watcher        = nullptr;
    QQuickWindow* m_attachedWindow = nullptr;

    // Phase 2 — persistent texture (RGBA fallback path)
    QSGTexture*   m_texture   = nullptr;
    int           m_texW      = 0;
    int           m_texH      = 0;

    // Fase 6 — NV12 dual texture (uint32_t = GLuint, avoids GL header in .h)
    uint32_t      m_yTex      = 0;
    uint32_t      m_uvTex     = 0;
    int           m_nv12W     = 0;  // tracked Y-plane width for realloc detection
    int           m_nv12H     = 0;  // tracked Y-plane height for realloc detection
    bool          m_nv12Active = false; // true when NV12 pipeline is in use

    // Fase 7 — frame-id / epoch / geometry gating
    uint64_t      m_lastUploadedFrameId = UINT64_MAX;
    uint64_t      m_lastUploadedEpoch   = 0;
    qreal         m_lastRectW           = 0;
    qreal         m_lastRectH           = 0;

};

#endif // NATIVISITEM_H
