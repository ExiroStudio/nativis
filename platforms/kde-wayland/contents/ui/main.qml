import QtQuick
import org.kde.plasma.plasmoid
import "."

WallpaperItem {
    id: root

    Rectangle {
        anchors.fill: parent
        color: "black"
    }

    NativisItem {
        id: nativis
        anchors.fill: parent
    }
}
