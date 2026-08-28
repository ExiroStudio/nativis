    #include "nativisitem.h"
    #include <QSGSimpleTextureNode>
    #include <QQuickWindow>
    #include <QImage>
    #include <QDebug>
    #include <QElapsedTimer>
    #include <QOpenGLContext>
    #include <QOpenGLFunctions>
    #include <QOpenGLShaderProgram>
    #include <QSGGeometryNode>
    #include <QSGGeometry>
    #include <QSGMaterial>
    #include <QSGMaterialShader>

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

        // Fase 6: delete NV12 GL textures
        if (m_yTex || m_uvTex) {
            QOpenGLContext *glCtx = QOpenGLContext::currentContext();
            if (glCtx) {
                QOpenGLFunctions *f = glCtx->functions();
                if (m_yTex)  f->glDeleteTextures(1, &m_yTex);
                if (m_uvTex) f->glDeleteTextures(1, &m_uvTex);
            }
            m_yTex = 0;
            m_uvTex = 0;
        }

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
        // ── Geometry ───────────────────────────────────────────────────────────
        int w = qMax(1, static_cast<int>(width()));
        int h = qMax(1, static_cast<int>(height()));

        // ── Acquire frame from runtime ─────────────────────────────────────────
        if (!nativis_begin_frame(m_runtimeCtx, w, h)) {
            return oldNode; // runtime not ready yet
        }

        // ── Fase 7: Frame-id gating ────────────────────────────────────────────
        // Don't re-upload textures if the frame hasn't changed.
        // This prevents redundant glTexSubImage2D calls that waste GPU bandwidth.
        uint64_t currentFrameId = nativis_get_frame_id(m_runtimeCtx);
        if (currentFrameId == m_lastUploadedFrameId && oldNode != nullptr) {
            nativis_end_frame(m_runtimeCtx);
            return oldNode; // no new frame — skip upload
        }
        m_lastUploadedFrameId = currentFrameId;

        // ── Detect format ──────────────────────────────────────────────────────
        uint32_t format = nativis_get_format(m_runtimeCtx);

        QQuickWindow *win = window();
        if (!win) return oldNode;

        // ── NV12 path (Fase 6) ─────────────────────────────────────────────────
        if (format == NATIVIS_FORMAT_NV12) {
            uint32_t yStride = 0, yW = 0, yH = 0;
            uint8_t* yData = nativis_get_plane(m_runtimeCtx, 0, &yStride, &yW, &yH);

            uint32_t uvStride = 0, uvW = 0, uvH = 0;
            uint8_t* uvData = nativis_get_plane(m_runtimeCtx, 1, &uvStride, &uvW, &uvH);

            nativis_render(m_runtimeCtx);
            nativis_end_frame(m_runtimeCtx);

            if (!yData || !uvData || yW == 0 || yH == 0) return oldNode;

            int realW = static_cast<int>(yW);
            int realH = static_cast<int>(yH);

            QOpenGLContext *glCtx = QOpenGLContext::currentContext();
            if (!glCtx) return oldNode;
            QOpenGLFunctions *f = glCtx->functions();

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
                // GL_UNPACK_ALIGNMENT default is 4 bytes. RGBA was always safe (4 B/px),
                // but GL_RED (1 B/px) rows aren't guaranteed to land on a 4-byte boundary.
                // Without this, GL silently pads each row, desyncing every row after the first.
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
                f->glPixelStorei(GL_UNPACK_ALIGNMENT, 1); // same reasoning as Y: GL_RG is 2 B/px
                f->glPixelStorei(GL_UNPACK_ROW_LENGTH, uvStride / 2); // GL_RG = 2 bytes/pixel
                f->glTexImage2D(GL_TEXTURE_2D, 0, GL_RG8, uvWidth, uvHeight, 0,
                            GL_RG, GL_UNSIGNED_BYTE, uvData);

                f->glPixelStorei(GL_UNPACK_ROW_LENGTH, 0); // reset
                f->glPixelStorei(GL_UNPACK_ALIGNMENT, 4);  // restore GL default for other callers

                m_nv12W = realW;
                m_nv12H = realH;
                m_nv12Active = true;
            } else {
                // ── Fast path: reuse existing GL textures, upload only ──
                // Same GL_UNPACK_ALIGNMENT reasoning as the allocation branch above —
                // this path runs on nearly every frame, so it's the one that mattered most.
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

            if (!geoNode || !m_nv12Active) {
                // Delete old node entirely if switching from RGBA to NV12
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

            // Update geometry rect
            QRectF rect = boundingRect();
            if (rect.width() <= 1.0 || rect.height() <= 1.0) {
                if (parentItem() && parentItem()->width() > 1.0)
                    rect = QRectF(0, 0, parentItem()->width(), parentItem()->height());
                else if (win->width() > 1)
                    rect = QRectF(0, 0, win->width(), win->height());
                else
                    rect = QRectF(0, 0, realW, realH);
            }

            QSGGeometry::updateTexturedRectGeometry(geoNode->geometry(), rect, QRectF(0, 0, 1, 1));
            geoNode->markDirty(QSGNode::DirtyGeometry | QSGNode::DirtyMaterial);

            return geoNode;
        }

        // ── RGBA fallback path (image_backend, or format not NV12) ────────────
        m_nv12Active = false;

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
        bool needsNewTexture = (m_texture == nullptr)
                            || (realW != m_texW)
                            || (realH != m_texH);

        if (needsNewTexture) {
            // Deallocate old texture when resolution changes
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
                GLuint texId = static_cast<GLuint>(m_texture->textureId());
                f->glBindTexture(GL_TEXTURE_2D, texId);
                f->glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0,
                                realW, realH,
                                GL_RGBA, GL_UNSIGNED_BYTE,
                                pixels);
                node->markDirty(QSGNode::DirtyMaterial);
            }
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

        return node;
    }