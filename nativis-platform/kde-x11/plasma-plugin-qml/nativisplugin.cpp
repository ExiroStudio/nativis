#include "nativisplugin.h"
#include "nativisitem.h"
#include <qqml.h>

void NativisPlugin::registerTypes(const char *uri)
{
    Q_ASSERT(uri == QLatin1String("org.nativis"));
    qmlRegisterType<NativisItem>(uri, 1, 0, "NativisItem");
}
