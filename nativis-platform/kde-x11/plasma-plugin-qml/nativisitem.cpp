#include "nativisitem.h"
#include <QSGSimpleTextureNode>
#include <QQuickWindow>
#include <QImage>
#include <QDebug>
#include <QDateTime>
#include <QElapsedTimer>
#include <QOpenGLContext>
#include <QOpenGLFunctions>
#include <QOpenGLShaderProgram>
#include <QSGGeometryNode>
#include <QSGGeometry>
#include <QSGMaterial>
#include <QSGMaterialShader>
#include <QRunnable>

// ───────────────────────────────────────────────────────────────────────────
// Nv12Material + Nv12Shader — Fase 6
//
// Custom QSGMaterial that holds two GL textures (Y + UV) and renders
// NV12→RGB color conversion entirely on the GPU via a GLSL fragment shader.
// ───────────────────────────────────────────────────────────────────────────

class Nv12Shader : public QSGMaterialShader
{
public:
    Nv12Shader() {}

    char const *const *attributeNames() const override {
        static char const *const names[] = { "qt_Vertex", "qt_MultiTexCoord0", nullptr };
        return names;
    }

    const char *vertexShader() const override {
        return
            "uniform highp mat4 qt_Matrix;\n"
            "attribute highp vec4 qt_Vertex;\n"
            "attribute highp vec2 qt_MultiTexCoord0;\n"
            "varying highp vec2 texCoord;\n"
            "void main() {\n"
            "    texCoord = qt_MultiTexCoord0;\n"
            "    gl_Position = qt_Matrix * qt_Vertex;\n"
            "}\n";
    }

    // NV12 → RGB via BT.601 matrix
    // BT.601 is correct for SDR content. For HD/4K broadcast sources,
    // BT.709 coefficients would be: r = y + 1.5748*v, g = y - 0.1873*u - 0.4681*v, b = y + 1.8556*u
    const char *fragmentShader() const override {
        return
            "uniform sampler2D yTex;\n"
            "uniform sampler2D uvTex;\n"
            "uniform lowp float qt_Opacity;\n"
            "varying highp vec2 texCoord;\n"
            "void main() {\n"
            "    highp float y  = texture2D(yTex, texCoord).r;\n"
            "    highp vec2  uv = texture2D(uvTex, texCoord).rg - vec2(0.5, 0.5);\n"
            "    highp float r  = y + 1.402 * uv.y;\n"
            "    highp float g  = y - 0.344136 * uv.x - 0.714136 * uv.y;\n"
            "    highp float b  = y + 1.772 * uv.x;\n"
            "    gl_FragColor = vec4(r, g, b, 1.0) * qt_Opacity;\n"
            "}\n";
    }

    void initialize() override {
        QSGMaterialShader::initialize();
        m_matrixId  = program()->uniformLocation("qt_Matrix");
        m_opacityId = program()->uniformLocation("qt_Opacity");
        m_yTexId    = program()->uniformLocation("yTex");
        m_uvTexId   = program()->uniformLocation("uvTex");
    }

    void updateState(const RenderState &state, QSGMaterial *newMaterial, QSGMaterial *oldMaterial) override;

private:
    int m_matrixId  = -1;
    int m_opacityId = -1;
    int m_yTexId    = -1;
    int m_uvTexId   = -1;
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

    QSGMaterialShader *createShader() const override {
        return new Nv12Shader;
    }

    GLuint yTexture  = 0;
    GLuint uvTexture = 0;
};

void Nv12Shader::updateState(const RenderState &state, QSGMaterial *newMaterial, QSGMaterial * /* oldMaterial */)
{
    auto *mat = static_cast<Nv12Material *>(newMaterial);
    QOpenGLFunctions *f = QOpenGLContext::currentContext()->functions();

    if (state.isMatrixDirty())
        program()->setUniformValue(m_matrixId, state.combinedMatrix());

    if (state.isOpacityDirty())
        program()->setUniformValue(m_opacityId, state.opacity());

    // Bind Y texture to unit 0
    f->glActiveTexture(GL_TEXTURE0);
    f->glBindTexture(GL_TEXTURE_2D, mat->yTexture);
    program()->setUniformValue(m_yTexId, 0);

    // Bind UV texture to unit 1
    f->glActiveTexture(GL_TEXTURE1);
    f->glBindTexture(GL_TEXTURE_2D, mat->uvTexture);
    program()->setUniformValue(m_uvTexId, 1);

    // Reset active texture
    f->glActiveTexture(GL_TEXTURE0);
}

// ───────────────────────────────────────────────────────────────────────────
// NativisItem
// ───────────────────────────────────────────────────────────────────────────

NativisItem::NativisItem(QQuickItem *parent)
    : QQuickItem(parent)
{
    setFlag(ItemHasContents, true);
    m_runtimeCtx = nativis_create();
    const uint32_t abiVer = nativis_version();
    qDebug() << "Nativis Runtime Initialized. ABI Version:" << abiVer;
    if (abiVer != NATIVIS_PROTOCOL_VERSION) {
        qWarning() << "Nativis ABI version mismatch! Expected" << NATIVIS_PROTOCOL_VERSION << "got" << abiVer;
    }
}

class TextureCleanupJob : public QRunnable
{
public:
    TextureCleanupJob(GLuint yTex, GLuint uvTex, QSGTexture* tex)
        : m_y(yTex), m_uv(uvTex), m_tex(tex) {}

    void run() override {
        QOpenGLContext *glCtx = QOpenGLContext::currentContext();
        if (glCtx) {
            QOpenGLFunctions *f = glCtx->functions();
            if (m_y)  f->glDeleteTextures(1, &m_y);
            if (m_uv) f->glDeleteTextures(1, &m_uv);
        }
        delete m_tex;
    }
private:
    GLuint m_y = 0;
    GLuint m_uv = 0;
    QSGTexture *m_tex = nullptr;
};

void NativisItem::scheduleOrDeleteTextures(QQuickWindow *win)
{
    if (!m_yTex && !m_uvTex && !m_texture) {
        return;
    }
    if (win) {
        win->scheduleRenderJob(
            new TextureCleanupJob(m_yTex, m_uvTex, m_texture),
            QQuickWindow::BeforeSynchronizingStage
        );
    } else {
        QOpenGLContext *glCtx = QOpenGLContext::currentContext();
        if (glCtx) {
            QOpenGLFunctions *f = glCtx->functions();
            if (m_yTex)  f->glDeleteTextures(1, &m_yTex);
            if (m_uvTex) f->glDeleteTextures(1, &m_uvTex);
            delete m_texture;
        }
    }
    m_texture = nullptr;
    m_yTex = 0;
    m_uvTex = 0;
    m_nv12W = 0;
    m_nv12H = 0;
    m_texW = 0;
    m_texH = 0;
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
    // Phase 1: stop watcher before destroying runtime context
    if (m_watcher) {
        m_watcher->stop();
        while (m_watcher->isRunning()) {
            if (m_watcher->wait(50)) {
                break;
            }
            if (m_runtimeCtx) {
                nativis_wake(m_runtimeCtx);
            }
        }
        delete m_watcher;
        m_watcher = nullptr;
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
        // Phase 1: disconnect old watcher if window changes
        if (m_watcher) {
            m_watcher->stop();
            while (m_watcher->isRunning()) {
                if (m_watcher->wait(50)) {
                    break;
                }
                if (m_runtimeCtx) {
                    nativis_wake(m_runtimeCtx);
                }
            }
            delete m_watcher;
            m_watcher = nullptr;
        }

        if (m_attachedWindow && m_attachedWindow != value.window) {
            scheduleOrDeleteTextures(m_attachedWindow);
        }
        m_attachedWindow = value.window;

        if (value.window) {
            if (m_runtimeCtx) {
                nativis_resume_wait(m_runtimeCtx);
            }
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
    static const bool forceRgba = qEnvironmentVariableIntValue("NATIVIS_DEBUG_FORCE_RGBA") != 0;
    static const bool timingDebug = qEnvironmentVariableIntValue("NATIVIS_DEBUG_FRAME_TIMING") != 0;

    QElapsedTimer timingTimer;
    if (timingDebug) {
        timingTimer.start();
    }

    // ── Geometry ───────────────────────────────────────────────────────────
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

    // ── Acquire frame from runtime ─────────────────────────────────────────
    if (!nativis_begin_frame(m_runtimeCtx, w, h)) {
        return oldNode; // runtime not ready yet
    }

    struct NativisFrameGuard {
        void *ctx;
        ~NativisFrameGuard() {
            if (ctx) nativis_end_frame(ctx);
        }
    } frameGuard{m_runtimeCtx};

    // ── Fase 7: Frame-id / epoch / geometry gating ─────────────────────────
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
        return oldNode; // no new frame — skip upload
    }

    // ── Detect format ──────────────────────────────────────────────────────
    uint32_t format = nativis_get_format(m_runtimeCtx);
    if (forceRgba && format == NATIVIS_FORMAT_NV12) {
        format = NATIVIS_FORMAT_RGBA8888;
    }

    // ── NV12 path (Fase 6) ─────────────────────────────────────────────────
    if (format == NATIVIS_FORMAT_NV12) {
        uint32_t yStride = 0, yW = 0, yH = 0;
        uint8_t* yData = nativis_get_plane(m_runtimeCtx, 0, &yStride, &yW, &yH);

        uint32_t uvStride = 0, uvW = 0, uvH = 0;
        uint8_t* uvData = nativis_get_plane(m_runtimeCtx, 1, &uvStride, &uvW, &uvH);

        nativis_render(m_runtimeCtx);

        if (!yData || !uvData || yW == 0 || yH == 0) {
            qDebug() << "[nativis-debug] updatePaintNode: plane data not ready, frameId=" << currentFrameId
                      << "yData=" << (void*)yData << "uvData=" << (void*)uvData
                      << "t=" << QDateTime::currentMSecsSinceEpoch();
            return oldNode;
        }

        int realW = static_cast<int>(yW);
        int realH = static_cast<int>(yH);

        QOpenGLContext *glCtx = QOpenGLContext::currentContext();
        if (!glCtx) {
            qDebug() << "[nativis-debug] updatePaintNode: glCtx==null, frameId=" << currentFrameId
                      << "t=" << QDateTime::currentMSecsSinceEpoch();
            return oldNode;
        }

        const bool wasNv12Active = m_nv12Active;
        QOpenGLFunctions *f = glCtx->functions();

        struct GlStateGuard {
            QOpenGLFunctions *f;
            QQuickWindow *win;
            GLint prevActiveTex = GL_TEXTURE0;
            GLint prevTex = 0;
            GlStateGuard(QOpenGLFunctions *funcs, QQuickWindow *w) : f(funcs), win(w) {
                f->glGetIntegerv(GL_ACTIVE_TEXTURE, &prevActiveTex);
                f->glActiveTexture(GL_TEXTURE0);
                f->glGetIntegerv(GL_TEXTURE_BINDING_2D, &prevTex);
            }
            ~GlStateGuard() {
                f->glActiveTexture(GL_TEXTURE0);
                f->glBindTexture(GL_TEXTURE_2D, prevTex);
                f->glActiveTexture(prevActiveTex);
                win->resetOpenGLState();
            }
        } glGuard(f, win);

        // ── Allocate or reallocate GL textures on resolution change ──
        bool needsRealloc = (m_yTex == 0) || (realW != m_nv12W) || (realH != m_nv12H);
        if (needsRealloc) {
            if (m_yTex)  f->glDeleteTextures(1, &m_yTex);
            if (m_uvTex) f->glDeleteTextures(1, &m_uvTex);

            // Y texture: GL_R8 (1 byte per pixel, full resolution)
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

            // UV texture: GL_RG8 (2 bytes per pixel, half resolution)
            int uvWidth  = static_cast<int>(uvW);
            int uvHeight = static_cast<int>(uvH);
            f->glGenTextures(1, &m_uvTex);
            f->glBindTexture(GL_TEXTURE_2D, m_uvTex);
            f->glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
            f->glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
            f->glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
            f->glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
            f->glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
            f->glPixelStorei(GL_UNPACK_ROW_LENGTH, uvStride / 2);
            f->glTexImage2D(GL_TEXTURE_2D, 0, GL_RG8, uvWidth, uvHeight, 0,
                           GL_RG, GL_UNSIGNED_BYTE, uvData);

            f->glPixelStorei(GL_UNPACK_ROW_LENGTH, 0);
            f->glPixelStorei(GL_UNPACK_ALIGNMENT, 4);

            m_nv12W = realW;
            m_nv12H = realH;
            m_nv12Active = true;
        } else {
            f->glBindTexture(GL_TEXTURE_2D, m_yTex);
            f->glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
            f->glPixelStorei(GL_UNPACK_ROW_LENGTH, yStride);
            f->glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, realW, realH,
                              GL_RED, GL_UNSIGNED_BYTE, yData);

            int uvWidth  = static_cast<int>(uvW);
            int uvHeight = static_cast<int>(uvH);
            f->glBindTexture(GL_TEXTURE_2D, m_uvTex);
            f->glPixelStorei(GL_UNPACK_ROW_LENGTH, uvStride / 2);
            f->glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, uvWidth, uvHeight,
                              GL_RG, GL_UNSIGNED_BYTE, uvData);

            f->glPixelStorei(GL_UNPACK_ROW_LENGTH, 0);
            f->glPixelStorei(GL_UNPACK_ALIGNMENT, 4);
        }

        // ── Build / reuse scene graph node with NV12 material ──────────
        QSGGeometryNode *geoNode = static_cast<QSGGeometryNode *>(oldNode);
        Nv12Material *material = nullptr;

        if (!geoNode || !wasNv12Active) {
            delete oldNode;

            geoNode = new QSGGeometryNode;
            auto *geometry = new QSGGeometry(QSGGeometry::defaultAttributes_TexturedPoint2D(), 4);
            geometry->setDrawingMode(GL_TRIANGLE_STRIP);
            geoNode->setGeometry(geometry);
            geoNode->setFlag(QSGNode::OwnsGeometry);

            material = new Nv12Material;
            geoNode->setMaterial(material);
            geoNode->setFlag(QSGNode::OwnsMaterial);
        } else {
            material = static_cast<Nv12Material *>(geoNode->material());
        }

        material->yTexture  = m_yTex;
        material->uvTexture = m_uvTex;

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

    // ── RGBA fallback path (image_backend, or format not NV12) ────────────
    const bool wasNv12ActiveRgba = m_nv12Active;
    m_nv12Active = false;

    uint8_t* pixels = nativis_get_pixels(m_runtimeCtx);
    nativis_render(m_runtimeCtx);

    int realW = nativis_get_width(m_runtimeCtx);
    int realH = nativis_get_height(m_runtimeCtx);
    if (realW <= 0) realW = w;
    if (realH <= 0) realH = h;

    if (!pixels) return oldNode;

    // ── Build / reuse scene graph node ────────────────────────────────────
    QSGSimpleTextureNode *node = wasNv12ActiveRgba ? nullptr : static_cast<QSGSimpleTextureNode *>(oldNode);
    if (!node) {
        if (wasNv12ActiveRgba) {
            delete oldNode;
        }
        node = new QSGSimpleTextureNode();
        node->setFiltering(QSGTexture::Linear);
    }

    // ── Phase 2: persistent texture ───────────────────────────────────────
    bool needsNewTexture = (m_texture == nullptr)
                        || (realW != m_texW)
                        || (realH != m_texH);

    if (needsNewTexture) {
        delete m_texture;
        m_texture = nullptr;

        QImage img(pixels, realW, realH, realW * 4, QImage::Format_RGBA8888);
        m_texture = win->createTextureFromImage(img);
        m_texW = realW;
        m_texH = realH;

        node->setTexture(m_texture);
        node->setOwnsTexture(false);
    } else {
        QOpenGLContext *glCtx = QOpenGLContext::currentContext();
        if (glCtx) {
            QOpenGLFunctions *f = glCtx->functions();
            GLint prevActiveTex = GL_TEXTURE0;
            GLint prevTex = 0;
            f->glGetIntegerv(GL_ACTIVE_TEXTURE, &prevActiveTex);
            f->glActiveTexture(GL_TEXTURE0);
            f->glGetIntegerv(GL_TEXTURE_BINDING_2D, &prevTex);

            GLuint texId = static_cast<GLuint>(m_texture->textureId());
            f->glBindTexture(GL_TEXTURE_2D, texId);
            f->glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0,
                               realW, realH,
                               GL_RGBA, GL_UNSIGNED_BYTE,
                               pixels);
            node->markDirty(QSGNode::DirtyMaterial);

            f->glBindTexture(GL_TEXTURE_2D, prevTex);
            f->glActiveTexture(prevActiveTex);
            win->resetOpenGLState();
        }
        if (node->texture() != m_texture) {
            node->setTexture(m_texture);
            node->setOwnsTexture(false);
        }
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