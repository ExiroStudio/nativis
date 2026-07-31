#include "nativisitem.h"
#include <QSGSimpleTextureNode>
#include <QQuickWindow>
#include <QImage>
#include <QDebug>
#include <QElapsedTimer>
#include <QOpenGLContext>
#include <QOpenGLFunctions>

// ── Phase 0: instrumentation knob ──────────────────────────────────────────
// Set to 0 to compile out all timing/logging with zero overhead.
#define NATIVIS_INSTRUMENTATION 1
static constexpr int kLogEveryNFrames = 300; // ~5 s at 60 FPS

#if NATIVIS_INSTRUMENTATION
#  define NATIVIS_TIMER_START(t)  QElapsedTimer t; t.start()
#  define NATIVIS_TIMER_US(t)     (t.nsecsElapsed() / 1000)
#else
#  define NATIVIS_TIMER_START(t)  do {} while(0)
#  define NATIVIS_TIMER_US(t)     0LL
#endif
// ───────────────────────────────────────────────────────────────────────────

extern "C" {
    void* nativis_create();
    void nativis_destroy(void* ctx);
    bool nativis_begin_frame(void* ctx, int width, int height);
    uint8_t* nativis_get_pixels(void* ctx);
    int nativis_get_width(void* ctx);
    int nativis_get_height(void* ctx);
    void nativis_render(void* ctx);
    void nativis_end_frame(void* ctx);
    uint32_t nativis_version();
    uint64_t nativis_get_frame_id(void* ctx);
}

// ───────────────────────────────────────────────────────────────────────────
// NativisItem
// ───────────────────────────────────────────────────────────────────────────

NativisItem::NativisItem(QQuickItem *parent)
    : QQuickItem(parent)
{
    setFlag(ItemHasContents, true);
    m_runtimeCtx = nativis_create();
    qDebug() << "Nativis Runtime Initialized. ABI Version:" << nativis_version();
}

NativisItem::~NativisItem()
{
    // Phase 1: stop watcher before destroying runtime context
    if (m_watcher) {
        m_watcher->stop();
        m_watcher->wait();
        delete m_watcher;
        m_watcher = nullptr;
    }

    // Phase 2: persistent texture is owned by us (setOwnsTexture(false))
    delete m_texture;
    m_texture = nullptr;

    if (m_runtimeCtx) {
        nativis_destroy(m_runtimeCtx);
        m_runtimeCtx = nullptr;
    }
}

void NativisItem::itemChange(ItemChange change, const ItemChangeData &value)
{
    if (change == ItemSceneChange) {
        // Phase 1: disconnect old watcher if window changes
        if (m_watcher) {
            m_watcher->stop();
            m_watcher->wait();
            delete m_watcher;
            m_watcher = nullptr;
        }

        if (value.window) {
            // Phase 1: start watcher — drives rendering only on new frames.
            // frameSwapped is intentionally NOT connected.
            m_watcher = new FrameWatcher(m_runtimeCtx, this);
            connect(m_watcher, &FrameWatcher::newFrameAvailable,
                    this, [this](quint64) {
                        // QueuedConnection: crosses thread boundary safely,
                        // schedules a single update() per frame, not per vsync.
                        update();
                    }, Qt::QueuedConnection);
            m_watcher->start();
        }
    }
    QQuickItem::itemChange(change, value);
}

QSGNode *NativisItem::updatePaintNode(QSGNode *oldNode, UpdatePaintNodeData *)
{
    // ── Phase 0: total frame timer ─────────────────────────────────────────
    NATIVIS_TIMER_START(totalTimer);

    // ── Geometry ───────────────────────────────────────────────────────────
    int w = qMax(1, static_cast<int>(width()));
    int h = qMax(1, static_cast<int>(height()));

    // ── Acquire frame from runtime ─────────────────────────────────────────
    if (!nativis_begin_frame(m_runtimeCtx, w, h)) {
        return oldNode; // runtime not ready yet
    }

    uint8_t* pixels = nativis_get_pixels(m_runtimeCtx);
    nativis_render(m_runtimeCtx);
    nativis_end_frame(m_runtimeCtx);

    int realW = nativis_get_width(m_runtimeCtx);
    int realH = nativis_get_height(m_runtimeCtx);
    if (realW <= 0) realW = w;
    if (realH <= 0) realH = h;

    if (!pixels) return oldNode;

    // ── Build / reuse scene graph node ────────────────────────────────────
    QSGSimpleTextureNode *node = static_cast<QSGSimpleTextureNode *>(oldNode);
    if (!node) {
        node = new QSGSimpleTextureNode();
        node->setFiltering(QSGTexture::Linear);
    }

    // ── Phase 2: persistent texture ───────────────────────────────────────
    //
    // Evidence for this change:
    //   qt.scenegraph.time.texture: plain texture uploaded in: 4–7ms (2560x1707)
    //   createTextureFromImage() called every frame → glGenTextures + glDeleteTextures each time.
    //
    // Strategy:
    //   • Allocate texture once via createTextureFromImage (first frame or resolution change).
    //   • On subsequent frames, upload pixels directly with glTexSubImage2D.
    //   • glTexSubImage2D reuses existing GPU memory — no allocation, no destruction.

    QQuickWindow *win = window();
    if (!win) return node;

    bool needsNewTexture = (m_texture == nullptr)
                        || (realW != m_texW)
                        || (realH != m_texH);

    if (needsNewTexture) {
        // ── Phase 0: allocation timer ──────────────────────────────────────
        NATIVIS_TIMER_START(allocTimer);

        // Deallocate old texture when resolution changes
        delete m_texture;
        m_texture = nullptr;

        QImage img(pixels, realW, realH, realW * 4, QImage::Format_RGBA8888);
        // setOwnsTexture(false) below — we manage lifetime explicitly
        m_texture = win->createTextureFromImage(img);
        m_texW = realW;
        m_texH = realH;

        node->setTexture(m_texture);
        node->setOwnsTexture(false); // NativisItem owns m_texture, not the node

        qint64 allocUs = NATIVIS_TIMER_US(allocTimer);
        m_totalAllocUs += allocUs;

        qDebug().nospace() << "[NATIVIS] Texture (re)allocated: "
                           << realW << "x" << realH
                           << "  alloc=" << allocUs << "µs";
    } else {
        // ── Phase 0: upload timer ──────────────────────────────────────────
        NATIVIS_TIMER_START(uploadTimer);

        // Reuse existing GPU texture — upload new pixels only.
        // glTexSubImage2D does NOT allocate; it writes into existing GPU memory.
        QOpenGLContext *glCtx = QOpenGLContext::currentContext();
        if (glCtx) {
            QOpenGLFunctions *f = glCtx->functions();
            // Bind our texture's underlying GL object
            GLuint texId = static_cast<GLuint>(m_texture->textureId());
            f->glBindTexture(GL_TEXTURE_2D, texId);
            f->glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0,
                               realW, realH,
                               GL_RGBA, GL_UNSIGNED_BYTE,
                               pixels);
        }

        qint64 uploadUs = NATIVIS_TIMER_US(uploadTimer);
        m_totalUploadUs += uploadUs;
    }

    // ── Rect ──────────────────────────────────────────────────────────────
    QRectF rect = boundingRect();
    if (rect.width() <= 1.0 || rect.height() <= 1.0) {
        if (parentItem() && parentItem()->width() > 1.0)
            rect = QRectF(0, 0, parentItem()->width(), parentItem()->height());
        else if (win->width() > 1)
            rect = QRectF(0, 0, win->width(), win->height());
        else
            rect = QRectF(0, 0, realW, realH);
    }
    node->setRect(rect);

    // ── Phase 0: log summary every N frames ───────────────────────────────
    m_frameCount++;
    qint64 frameUs = NATIVIS_TIMER_US(totalTimer);
    m_totalFrameUs += frameUs;

#if NATIVIS_INSTRUMENTATION
    if (m_frameCount % kLogEveryNFrames == 0) {
        double avgAlloc  = static_cast<double>(m_totalAllocUs)  / kLogEveryNFrames;
        double avgUpload = static_cast<double>(m_totalUploadUs) / kLogEveryNFrames;
        double avgFrame  = static_cast<double>(m_totalFrameUs)  / kLogEveryNFrames;
        qDebug().nospace()
            << "[NATIVIS METRICS] frame=" << m_frameCount
            << "  avg_alloc="  << avgAlloc  << "µs"
            << "  avg_upload=" << avgUpload << "µs"
            << "  avg_total="  << avgFrame  << "µs";
        m_totalAllocUs = m_totalUploadUs = m_totalFrameUs = 0;
    }
#endif

    return node;
}
