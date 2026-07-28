#ifndef NATIVISPLUGIN_H
#define NATIVISPLUGIN_H

#include <QQmlExtensionPlugin>

class NativisPlugin : public QQmlExtensionPlugin
{
    Q_OBJECT
    Q_PLUGIN_METADATA(IID QQmlExtensionInterface_iid)

public:
    void registerTypes(const char *uri) override;
};

#endif // NATIVISPLUGIN_H
