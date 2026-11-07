#ifndef NATIVISITEM_H
#define NATIVISITEM_H

#include <QQuickItem>
#include <QThread>
#include <QAtomicInteger>
#include <atomic>
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
    uint64_t nativis_get_frame_id(void* ctx);

    // Multi-plane FFI for NV12
    uint32_t nativis_get_format(void* ctx);
    uint32_t nativis_get_plane_count(void* ctx);
    uint8_t* nativis_get_plane(void* ctx, uint32_t index,
                               uint32_t* out_stride, uint32_t* out_width, uint32_t* out_height);
}

#define NATIVIS_FORMAT_RGBA8888 1
#define NATIVIS_FORMAT_NV12     2

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
    QThread* m_watcherThread = nullptr;
    std::atomic<bool> m_watcherRunning{false};

    // Persistent NV12 GPU Textures
    uint32_t    m_yTex = 0;
    uint32_t    m_uvTex = 0;
    QSGTexture* m_yQSG = nullptr;
    QSGTexture* m_uvQSG = nullptr;
    int         m_nv12W = 0;
    int         m_nv12H = 0;
    bool        m_nv12Active = false;

    // Persistent RGBA GPU Texture (fallback)
    uint32_t    m_rgbaTex = 0;
    QSGTexture* m_rgbaQSG = nullptr;
    int         m_rgbaW = 0;
    int         m_rgbaH = 0;

    // Frame-id gating (skip redundant uploads)
    uint64_t    m_lastUploadedFrameId = UINT64_MAX;
};

#endif // NATIVISITEM_H
