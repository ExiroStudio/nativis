#ifndef NATIVISITEM_H
#define NATIVISITEM_H

#include <QQuickItem>

extern "C" {
    uint32_t nativis_version();
    void* nativis_create();
    void nativis_destroy(void* ctx);
    bool nativis_begin_frame(void* ctx, int width, int height);
    uint8_t* nativis_get_pixels(void* ctx);
    void nativis_render(void* ctx);
    void nativis_end_frame(void* ctx);
}

class NativisItem : public QQuickItem
{
    Q_OBJECT
public:
    explicit NativisItem(QQuickItem *parent = nullptr);
    ~NativisItem() override;

protected:
    QSGNode *updatePaintNode(QSGNode *oldNode, UpdatePaintNodeData *updatePaintNodeData) override;
    void itemChange(ItemChange change, const ItemChangeData &value) override;

private slots:
    void triggerUpdate();

private:
    void* m_runtimeCtx;
    int m_frameCount;
};

#endif // NATIVISITEM_H
