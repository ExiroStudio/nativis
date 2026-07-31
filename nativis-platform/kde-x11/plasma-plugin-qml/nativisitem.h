#ifndef NATIVISITEM_H
#define NATIVISITEM_H

#include <QQuickItem>
#include <QThread>
#include <QAtomicInteger>
#include <QtGlobal>

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
}

// ---------------------------------------------------------------------------
// FrameWatcher — Phase 1
//
// Runs on a dedicated thread. Polls nativis_get_frame_id() in a tight-but-
// yielding loop and emits newFrameAvailable() only when the id changes.
//
// Why a thread and not futex?
//   Runtime and plugin run in different processes. There is no shared address
//   space for a direct callback. A lightweight thread is the minimal cross-
//   process solution. msleep(4) gives ~240 Hz poll ceiling — well above any
//   realistic content frame rate — while burning near-zero CPU when idle.
// ---------------------------------------------------------------------------
class FrameWatcher : public QThread
{
    Q_OBJECT
public:
    explicit FrameWatcher(void* runtimeCtx, QObject* parent = nullptr)
        : QThread(parent), m_ctx(runtimeCtx), m_running(true) {}

    void stop() {
        m_running.store(false, std::memory_order_relaxed);
    }

signals:
    void newFrameAvailable(quint64 frameId);

protected:
    void run() override {
        quint64 lastSeen = 0;
        while (m_running.load(std::memory_order_relaxed)) {
            quint64 current = nativis_get_frame_id(m_ctx);
            if (current != lastSeen) {
                lastSeen = current;
                emit newFrameAvailable(current);
            }
            msleep(4); // ~240 Hz ceiling, negligible CPU when idle
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

class NativisItem : public QQuickItem
{
    Q_OBJECT
public:
    explicit NativisItem(QQuickItem *parent = nullptr);
    ~NativisItem() override;

protected:
    QSGNode *updatePaintNode(QSGNode *oldNode, UpdatePaintNodeData *) override;
    void itemChange(ItemChange change, const ItemChangeData &value) override;

private:
    void* m_runtimeCtx = nullptr;

    // Phase 0 — instrumentation
    int     m_frameCount      = 0;
    qint64  m_totalAllocUs    = 0;
    qint64  m_totalUploadUs   = 0;
    qint64  m_totalFrameUs    = 0;

    // Phase 1 — event-driven rendering
    FrameWatcher* m_watcher   = nullptr;

    // Phase 2 — persistent texture
    QSGTexture*   m_texture   = nullptr;
    int           m_texW      = 0;
    int           m_texH      = 0;
};

#endif // NATIVISITEM_H
