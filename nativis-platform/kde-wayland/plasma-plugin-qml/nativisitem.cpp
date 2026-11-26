#include "nativisitem.h"
#include <QSGSimpleTextureNode>
#include <QQuickWindow>
#include <QImage>
#include <QDebug>
#include <QDateTime>
#include <QElapsedTimer>
#include <QOpenGLContext>
#include <QOpenGLFunctions>
#include <QSGGeometryNode>
#include <QSGGeometry>
#include <QSGMaterial>
#include <QSGMaterialShader>
#include <QRunnable>
#include <QtQuick/qsgtexture_platform.h>

// ───────────────────────────────────────────────────────────────────────────
// Nv12Material + Nv12Shader (Qt6 RHI / QSGMaterialShader)
// ───────────────────────────────────────────────────────────────────────────

class Nv12Material;

class Nv12Shader : public QSGMaterialShader
{
public:
    Nv12Shader() {
        setShaderFileName(VertexStage, QStringLiteral(":/shaders/nv12.vert.qsb"));
        setShaderFileName(FragmentStage, QStringLiteral(":/shaders/nv12.frag.qsb"));
    }

    bool updateUniformData(RenderState &state, QSGMaterial *, QSGMaterial *) override {
        QByteArray *buf = state.uniformData();
        if (state.isMatrixDirty()) {
            const QMatrix4x4 m = state.combinedMatrix();
            memcpy(buf->data(), m.constData(), 64);
        }
        if (state.isOpacityDirty()) {
            const float opacity = state.opacity();
            memcpy(buf->data() + 64, &opacity, 4);
        }
        return true;
    }

    void updateSampledImage(RenderState &, int binding, QSGTexture **texture,
                            QSGMaterial *newMaterial, QSGMaterial *) override;
};

class Nv12Material : public QSGMaterial
{
public:
    Nv12Material() {
        setFlag(Blending, false);
    }

    QSGMaterialType *type() const override {
        static QSGMaterialType theType;
        return &theType;
    }

    QSGMaterialShader *createShader(QSGRendererInterface::RenderMode) const override {
        return new Nv12Shader;
    }

    QSGTexture *yTexture  = nullptr;
    QSGTexture *uvTexture = nullptr;
};

void Nv12Shader::updateSampledImage(RenderState &, int binding, QSGTexture **texture,
                                    QSGMaterial *newMaterial, QSGMaterial *)
{
    auto *mat = static_cast<Nv12Material *>(newMaterial);
    if (binding == 1) {
        *texture = mat->yTexture;
    } else if (binding == 2) {
        *texture = mat->uvTexture;
    }
}

// ───────────────────────────────────────────────────────────────────────────
// NativisItem
// ───────────────────────────────────────────────────────────────────────────

class TextureCleanupJob : public QRunnable
{
public:
    TextureCleanupJob(GLuint yTex, GLuint uvTex, GLuint rgbaTex, QSGTexture* t1, QSGTexture* t2, QSGTexture* t3)
        : m_y(yTex), m_uv(uvTex), m_rgba(rgbaTex), m_t1(t1), m_t2(t2), m_t3(t3) {}

    void run() override {
        QOpenGLContext *glCtx = QOpenGLContext::currentContext();
        if (glCtx) {
            QOpenGLFunctions *f = glCtx->functions();
            if (m_y)    f->glDeleteTextures(1, &m_y);
            if (m_uv)   f->glDeleteTextures(1, &m_uv);
            if (m_rgba) f->glDeleteTextures(1, &m_rgba);
        }
        delete m_t1;
        delete m_t2;
        delete m_t3;
    }
private:
    GLuint m_y = 0;
    GLuint m_uv = 0;
    GLuint m_rgba = 0;
    QSGTexture *m_t1 = nullptr;
    QSGTexture *m_t2 = nullptr;
    QSGTexture *m_t3 = nullptr;
};

NativisItem::NativisItem(QQuickItem *parent)
    : QQuickItem(parent)
{
    setFlag(ItemHasContents, true);
    m_runtimeCtx = nativis_create();
    const uint32_t abiVer = nativis_version();
    qDebug() << "Nativis Wayland Runtime Initialized. ABI Version:" << abiVer;
    if (abiVer != NATIVIS_PROTOCOL_VERSION) {
        qWarning() << "Nativis ABI version mismatch! Expected" << NATIVIS_PROTOCOL_VERSION << "got" << abiVer;
    }
}

void NativisItem::scheduleOrDeleteTextures(QQuickWindow *win)
{
    if (!m_yTex && !m_uvTex && !m_rgbaTex && !m_yQSG && !m_uvQSG && !m_rgbaQSG) {
        return;
    }
    if (win) {
        win->scheduleRenderJob(
            new TextureCleanupJob(m_yTex, m_uvTex, m_rgbaTex, m_yQSG, m_uvQSG, m_rgbaQSG),
            QQuickWindow::BeforeSynchronizingStage
        );
    } else {
        QOpenGLContext *glCtx = QOpenGLContext::currentContext();
        if (glCtx) {
            QOpenGLFunctions *f = glCtx->functions();
            if (m_yTex)    f->glDeleteTextures(1, &m_yTex);
            if (m_uvTex)   f->glDeleteTextures(1, &m_uvTex);
            if (m_rgbaTex) f->glDeleteTextures(1, &m_rgbaTex);
            delete m_yQSG;
            delete m_uvQSG;
            delete m_rgbaQSG;
        }
        // If neither win nor current OpenGL context exists, do not delete QSGTexture on GUI thread.
    }
    m_yTex = 0;
    m_uvTex = 0;
    m_rgbaTex = 0;
    m_yQSG = nullptr;
    m_uvQSG = nullptr;
    m_rgbaQSG = nullptr;
    m_nv12W = 0;
    m_nv12H = 0;
    m_rgbaW = 0;
    m_rgbaH = 0;
    m_nv12Active = false;
    m_lastUploadedFrameId = UINT64_MAX;
    m_lastUploadedEpoch = 0;
    m_lastRectW = 0;
    m_lastRectH = 0;
}

void NativisItem::releaseResources()
{
    scheduleOrDeleteTextures(window() ? window() : m_attachedWindow);
}

NativisItem::~NativisItem()
{
    if (m_watcherThread) {
        m_watcherRunning.store(false, std::memory_order_seq_cst);
        if (m_runtimeCtx) {
            nativis_wake(m_runtimeCtx);
        }
        while (m_watcherThread->isRunning()) {
            if (m_watcherThread->wait(50)) {
                break;
            }
            if (m_runtimeCtx) {
                nativis_wake(m_runtimeCtx);
            }
        }
        delete m_watcherThread;
        m_watcherThread = nullptr;
    }

    scheduleOrDeleteTextures(window() ? window() : m_attachedWindow);
    m_attachedWindow = nullptr;

    if (m_runtimeCtx) {
        nativis_destroy(m_runtimeCtx);
        m_runtimeCtx = nullptr;
    }
}

void NativisItem::itemChange(ItemChange change, const ItemChangeData &value)
{
    if (change == ItemSceneChange) {
        if (m_watcherThread) {
            m_watcherRunning.store(false, std::memory_order_seq_cst);
            if (m_runtimeCtx) {
                nativis_wake(m_runtimeCtx);
            }
            while (m_watcherThread->isRunning()) {
                if (m_watcherThread->wait(50)) {
                    break;
                }
                if (m_runtimeCtx) {
                    nativis_wake(m_runtimeCtx);
                }
            }
            delete m_watcherThread;
            m_watcherThread = nullptr;
        }

        if (m_attachedWindow && m_attachedWindow != value.window) {
            scheduleOrDeleteTextures(m_attachedWindow);
        }
        m_attachedWindow = value.window;

        if (value.window) {
            if (m_runtimeCtx) {
                nativis_resume_wait(m_runtimeCtx);
            }
            m_watcherRunning.store(true, std::memory_order_seq_cst);
            m_watcherThread = QThread::create([this]() {
                quint64 lastSeen = 0;
                quint64 lastEpoch = 0;
                while (m_watcherRunning.load(std::memory_order_seq_cst)) {
                    void* ctx = m_runtimeCtx;
                    if (!ctx) break;
                    quint64 current = nativis_wait_frame(ctx, lastSeen, 1000);
                    quint64 epoch = nativis_get_epoch(ctx);
                    if (!m_watcherRunning.load(std::memory_order_seq_cst)) {
                        break;
                    }
                    if (current != 0 && (current != lastSeen || (epoch != 0 && epoch != lastEpoch))) {
                        lastSeen = current;
                        if (epoch != 0) lastEpoch = epoch;
                        QMetaObject::invokeMethod(this, &QQuickItem::update, Qt::QueuedConnection);
                    }
                }
            });
            m_watcherThread->start();
        }
    }
    QQuickItem::itemChange(change, value);
}

QSGNode *NativisItem::updatePaintNode(QSGNode *oldNode, UpdatePaintNodeData *)
{
    static const bool forceRgba = qEnvironmentVariableIntValue("NATIVIS_DEBUG_FORCE_RGBA") != 0;
    static const bool timingDebug = qEnvironmentVariableIntValue("NATIVIS_DEBUG_FRAME_TIMING") != 0;

    QElapsedTimer timingTimer;
    if (timingDebug) {
        timingTimer.start();
    }

    int w = qMax(1, static_cast<int>(width()));
    int h = qMax(1, static_cast<int>(height()));

    QQuickWindow *win = window();
    if (!win) {
        return oldNode;
    }

    auto resolveTargetRect = [&](int fallbackW, int fallbackH) -> QRectF {
        QRectF rect = boundingRect();
        if (rect.width() <= 1.0 || rect.height() <= 1.0) {
            if (parentItem() && parentItem()->width() > 1.0)
                rect = QRectF(0, 0, parentItem()->width(), parentItem()->height());
            else if (win->width() > 1)
                rect = QRectF(0, 0, win->width(), win->height());
            else
                rect = QRectF(0, 0, fallbackW, fallbackH);
        }
        return rect;
    };

    if (!nativis_begin_frame(m_runtimeCtx, w, h)) {
        return oldNode;
    }

    struct NativisFrameGuard {
        void *ctx;
        ~NativisFrameGuard() {
            if (ctx) nativis_end_frame(ctx);
        }
    } frameGuard{m_runtimeCtx};

    uint64_t currentFrameId = nativis_get_frame_id(m_runtimeCtx);
    uint64_t currentEpoch = nativis_get_epoch(m_runtimeCtx);
    QRectF currentRect = resolveTargetRect(w, h);

    if (currentFrameId == m_lastUploadedFrameId
        && currentEpoch == m_lastUploadedEpoch
        && oldNode != nullptr) {
        if (currentRect.width() != m_lastRectW || currentRect.height() != m_lastRectH) {
            if (m_nv12Active) {
                auto *geoNode = static_cast<QSGGeometryNode *>(oldNode);
                QSGGeometry::updateTexturedRectGeometry(geoNode->geometry(), currentRect, QRectF(0, 0, 1, 1));
                geoNode->markDirty(QSGNode::DirtyGeometry);
            } else {
                auto *texNode = static_cast<QSGSimpleTextureNode *>(oldNode);
                texNode->setRect(currentRect);
            }
            m_lastRectW = currentRect.width();
            m_lastRectH = currentRect.height();
        }
        return oldNode;
    }

    uint32_t format = nativis_get_format(m_runtimeCtx);
    if (forceRgba && format == NATIVIS_FORMAT_NV12) {
        format = NATIVIS_FORMAT_RGBA8888;
    }

    QOpenGLContext *glCtx = QOpenGLContext::currentContext();
    if (!glCtx) {
        static bool warnedNoGl = false;
        if (!warnedNoGl) {
            qWarning() << "NativisItem: QOpenGLContext::currentContext() is null in updatePaintNode; skipping GPU upload";
            warnedNoGl = true;
        }
        return oldNode;
    }
    QOpenGLFunctions *f = glCtx->functions();

    struct ExternalGlGuard {
        QOpenGLFunctions *f;
        QQuickWindow *win;
        GLint prevActiveTex = GL_TEXTURE0;
        GLint prevTex = 0;

        ExternalGlGuard(QOpenGLFunctions *funcs, QQuickWindow *w) : f(funcs), win(w) {
            f->glGetIntegerv(GL_ACTIVE_TEXTURE, &prevActiveTex);
            f->glActiveTexture(GL_TEXTURE0);
            f->glGetIntegerv(GL_TEXTURE_BINDING_2D, &prevTex);
            win->beginExternalCommands();
        }
        ~ExternalGlGuard() {
            f->glActiveTexture(GL_TEXTURE0);
            f->glBindTexture(GL_TEXTURE_2D, prevTex);
            f->glActiveTexture(prevActiveTex);
            win->endExternalCommands();
        }
    } glGuard(f, win);

    // ── NV12 path (GPU hardware shader conversion) ─────────────────────────
    if (format == NATIVIS_FORMAT_NV12) {
        uint32_t yStride = 0, yW = 0, yH = 0;
        uint8_t* yData = nativis_get_plane(m_runtimeCtx, 0, &yStride, &yW, &yH);

        uint32_t uvStride = 0, uvW = 0, uvH = 0;
        uint8_t* uvData = nativis_get_plane(m_runtimeCtx, 1, &uvStride, &uvW, &uvH);

        nativis_render(m_runtimeCtx);

        if (!yData || !uvData || yW == 0 || yH == 0) {
            return oldNode;
        }

        int realW = static_cast<int>(yW);
        int realH = static_cast<int>(yH);
        int uvWidth = static_cast<int>(uvW);
        int uvHeight = static_cast<int>(uvH);

        const bool wasNv12Active = m_nv12Active;
        bool needsRealloc = (m_yTex == 0) || (realW != m_nv12W) || (realH != m_nv12H);

        if (needsRealloc) {
            if (m_yTex)  f->glDeleteTextures(1, &m_yTex);
            if (m_uvTex) f->glDeleteTextures(1, &m_uvTex);
            delete m_yQSG;
            delete m_uvQSG;

            // Y texture (GL_R8, full resolution)
            f->glGenTextures(1, &m_yTex);
            f->glBindTexture(GL_TEXTURE_2D, m_yTex);
            f->glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
            f->glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
            f->glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
            f->glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
            f->glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
            f->glPixelStorei(GL_UNPACK_ROW_LENGTH, yStride);
            f->glTexImage2D(GL_TEXTURE_2D, 0, GL_R8, realW, realH, 0,
                            GL_RED, GL_UNSIGNED_BYTE, yData);

            // UV texture (GL_RG8, half resolution)
            f->glGenTextures(1, &m_uvTex);
            f->glBindTexture(GL_TEXTURE_2D, m_uvTex);
            f->glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
            f->glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
            f->glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
            f->glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
            f->glPixelStorei(GL_UNPACK_ROW_LENGTH, uvStride / 2);
            f->glTexImage2D(GL_TEXTURE_2D, 0, GL_RG8, uvWidth, uvHeight, 0,
                            GL_RG, GL_UNSIGNED_BYTE, uvData);

            f->glPixelStorei(GL_UNPACK_ROW_LENGTH, 0);
            f->glPixelStorei(GL_UNPACK_ALIGNMENT, 4);

            m_yQSG = QNativeInterface::QSGOpenGLTexture::fromNative(m_yTex, win, QSize(realW, realH));
            m_uvQSG = QNativeInterface::QSGOpenGLTexture::fromNative(m_uvTex, win, QSize(uvWidth, uvHeight));

            m_nv12W = realW;
            m_nv12H = realH;
        } else {
            // Fast sub-image update directly into persistent textures
            f->glBindTexture(GL_TEXTURE_2D, m_yTex);
            f->glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
            f->glPixelStorei(GL_UNPACK_ROW_LENGTH, yStride);
            f->glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, realW, realH,
                              GL_RED, GL_UNSIGNED_BYTE, yData);

            f->glBindTexture(GL_TEXTURE_2D, m_uvTex);
            f->glPixelStorei(GL_UNPACK_ROW_LENGTH, uvStride / 2);
            f->glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, uvWidth, uvHeight,
                              GL_RG, GL_UNSIGNED_BYTE, uvData);

            f->glPixelStorei(GL_UNPACK_ROW_LENGTH, 0);
            f->glPixelStorei(GL_UNPACK_ALIGNMENT, 4);
        }
        m_nv12Active = true;

        QSGGeometryNode *geoNode = static_cast<QSGGeometryNode *>(oldNode);
        Nv12Material *material = nullptr;

        if (!geoNode || !wasNv12Active) {
            delete oldNode;
            geoNode = new QSGGeometryNode;
            auto *geometry = new QSGGeometry(QSGGeometry::defaultAttributes_TexturedPoint2D(), 4);
            geometry->setDrawingMode(QSGGeometry::DrawTriangleStrip);
            geoNode->setGeometry(geometry);
            geoNode->setFlag(QSGNode::OwnsGeometry);

            material = new Nv12Material;
            geoNode->setMaterial(material);
            geoNode->setFlag(QSGNode::OwnsMaterial);
        } else {
            material = static_cast<Nv12Material *>(geoNode->material());
        }

        material->yTexture  = m_yQSG;
        material->uvTexture = m_uvQSG;

        QRectF rect = resolveTargetRect(realW, realH);
        QSGGeometry::updateTexturedRectGeometry(geoNode->geometry(), rect, QRectF(0, 0, 1, 1));
        geoNode->markDirty(QSGNode::DirtyGeometry | QSGNode::DirtyMaterial);

        m_lastUploadedFrameId = currentFrameId;
        m_lastUploadedEpoch = currentEpoch;
        m_lastRectW = rect.width();
        m_lastRectH = rect.height();

        if (timingDebug) {
            static qint64 lastLog = 0;
            static int frameCount = 0;
            frameCount++;
            qint64 now = QDateTime::currentMSecsSinceEpoch();
            if (now - lastLog >= 1000) {
                qDebug() << "[NATIVIS TIMING] frameId=" << currentFrameId
                         << "paintUs=" << timingTimer.nsecsElapsed() / 1000
                         << "format=NV12"
                         << "fps=" << frameCount;
                frameCount = 0;
                lastLog = now;
            }
        }

        return geoNode;
    }

    // ── RGBA fallback path (image_backend, or non-planar format) ───────────
    const bool wasNv12ActiveRgba = m_nv12Active;
    m_nv12Active = false;

    uint8_t* pixels = nativis_get_pixels(m_runtimeCtx);
    nativis_render(m_runtimeCtx);

    int realW = nativis_get_width(m_runtimeCtx);
    int realH = nativis_get_height(m_runtimeCtx);
    if (realW <= 0) realW = w;
    if (realH <= 0) realH = h;

    if (!pixels) {
        return oldNode;
    }

    QSGSimpleTextureNode *node = wasNv12ActiveRgba ? nullptr : static_cast<QSGSimpleTextureNode *>(oldNode);
    if (!node) {
        if (wasNv12ActiveRgba) delete oldNode;
        node = new QSGSimpleTextureNode();
        node->setFiltering(QSGTexture::Linear);
    }

    bool needsNewTexture = (m_rgbaTex == 0) || (realW != m_rgbaW) || (realH != m_rgbaH);
    if (needsNewTexture) {
        if (m_rgbaTex) f->glDeleteTextures(1, &m_rgbaTex);
        delete m_rgbaQSG;

        f->glGenTextures(1, &m_rgbaTex);
        f->glBindTexture(GL_TEXTURE_2D, m_rgbaTex);
        f->glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
        f->glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
        f->glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
        f->glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
        f->glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, realW, realH, 0,
                        GL_RGBA, GL_UNSIGNED_BYTE, pixels);

        m_rgbaQSG = QNativeInterface::QSGOpenGLTexture::fromNative(
            m_rgbaTex, win, QSize(realW, realH), QQuickWindow::TextureHasAlphaChannel);
        m_rgbaW = realW;
        m_rgbaH = realH;

        node->setTexture(m_rgbaQSG);
        node->setOwnsTexture(false);
    } else {
        f->glBindTexture(GL_TEXTURE_2D, m_rgbaTex);
        f->glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, realW, realH,
                          GL_RGBA, GL_UNSIGNED_BYTE, pixels);
        if (node->texture() != m_rgbaQSG) {
            node->setTexture(m_rgbaQSG);
            node->setOwnsTexture(false);
        }
        node->markDirty(QSGNode::DirtyMaterial);
    }

    QRectF rect = resolveTargetRect(realW, realH);
    node->setRect(rect);

    m_lastUploadedFrameId = currentFrameId;
    m_lastUploadedEpoch = currentEpoch;
    m_lastRectW = rect.width();
    m_lastRectH = rect.height();

    if (timingDebug) {
        static qint64 lastLog = 0;
        static int frameCount = 0;
        frameCount++;
        qint64 now = QDateTime::currentMSecsSinceEpoch();
        if (now - lastLog >= 1000) {
            qDebug() << "[NATIVIS TIMING] frameId=" << currentFrameId
                     << "paintUs=" << timingTimer.nsecsElapsed() / 1000
                     << "format=RGBA"
                     << "fps=" << frameCount;
            frameCount = 0;
            lastLog = now;
        }
    }

    return node;
}
