import QtQuick 2.15
import org.kde.plasma.plasmoid 2.0
import org.nativis 1.0

WallpaperItem {
    id: root
    
    NativisItem {
        id: nativis
        anchors.fill: parent
        width: root.width
        height: root.height
    }
}
