#include "nativisitem.h"
#include <QSGSimpleTextureNode>
#include <QQuickWindow>
#include <QImage>
#include <QDebug>
#include <QElapsedTimer>

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
}

NativisItem::NativisItem(QQuickItem *parent)
    : QQuickItem(parent), m_frameCount(0)
{
    setFlag(ItemHasContents, true);
    m_runtimeCtx = nativis_create();
    qDebug() << "Nativis Runtime Initialized. ABI Version:" << nativis_version();
}

NativisItem::~NativisItem()
{
    if (m_runtimeCtx) {
        nativis_destroy(m_runtimeCtx);
        m_runtimeCtx = nullptr;
    }
}

void NativisItem::itemChange(ItemChange change, const ItemChangeData &value)
{
    if (change == ItemSceneChange && value.window) {
        // Tie rendering to vsync by updating on frame swapped
        connect(value.window, &QQuickWindow::frameSwapped, this, &NativisItem::triggerUpdate);
        triggerUpdate();
    }
    QQuickItem::itemChange(change, value);
}

void NativisItem::triggerUpdate()
{
    update(); // Request a redraw on the next Qt vsync cycle
}

QSGNode *NativisItem::updatePaintNode(QSGNode *oldNode, UpdatePaintNodeData *)
{
    QElapsedTimer totalTimer;
    totalTimer.start();
    
    QSGSimpleTextureNode *node = static_cast<QSGSimpleTextureNode *>(oldNode);
    if (!node) {
        node = new QSGSimpleTextureNode();
        node->setFiltering(QSGTexture::Linear);
    }

    int w = qMax(1, static_cast<int>(width()));
    int h = qMax(1, static_cast<int>(height()));

    QElapsedTimer renderTimer;
    renderTimer.start();
    
    // Abstract RenderTarget C ABI
    if (!nativis_begin_frame(m_runtimeCtx, w, h)) {
        return node;
    }
    
    uint8_t* pixels = nativis_get_pixels(m_runtimeCtx);
    nativis_render(m_runtimeCtx);
    nativis_end_frame(m_runtimeCtx);
    
    qint64 renderTime = renderTimer.elapsed();
    
    QElapsedTimer copyTimer;
    copyTimer.start();

    int realW = nativis_get_width(m_runtimeCtx);
    int realH = nativis_get_height(m_runtimeCtx);
    if (realW <= 0) realW = w;
    if (realH <= 0) realH = h;
    
    // CPU Copy with real dimensions
    QImage img(pixels, realW, realH, realW * 4, QImage::Format_RGBA8888);
    
    // Qt limits texture updates to full recreation for public API in QImage wrapping
    QQuickWindow *win = window();
    if (win) {
        QSGTexture *texture = win->createTextureFromImage(img, QQuickWindow::TextureCanUseAtlas);
        node->setTexture(texture);
        node->setOwnsTexture(true);

        QRectF rect = boundingRect();
        if (rect.width() <= 1.0 || rect.height() <= 1.0) {
            if (parentItem() && parentItem()->width() > 1.0) {
                rect = QRectF(0, 0, parentItem()->width(), parentItem()->height());
            } else if (win->width() > 1) {
                rect = QRectF(0, 0, win->width(), win->height());
            } else {
                rect = QRectF(0, 0, realW, realH);
            }
        }
        node->setRect(rect);
    }
    
    qint64 copyTime = copyTimer.elapsed();
    m_frameCount++;
    
    if (m_frameCount % 60 == 0) {
        qDebug().nospace() << "Frame #" << m_frameCount 
                           << " | Render Time: " << renderTime << " ms"
                           << " | Copy/Upload Time: " << copyTime << " ms"
                           << " | Total: " << totalTimer.elapsed() << " ms"
                           << " | Scene Graph Update: OK";
    }

    return node;
}
