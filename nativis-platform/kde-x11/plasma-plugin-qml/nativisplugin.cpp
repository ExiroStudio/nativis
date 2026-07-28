#include "nativisplugin.h"
#include "nativisitem.h"
#include <qqml.h>

void NativisPlugin::registerTypes(const char *uri)
{
    qmlRegisterType<NativisItem>(uri, 1, 0, "NativisItem");
}
