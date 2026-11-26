import QtQuick 2.15
import org.kde.plasma.plasmoid 2.0
import "."

Item {
    id: root

    Rectangle {
        anchors.fill: parent
        color: "black"
    }

    NativisItem {
        id: nativis
        anchors.fill: parent
        width: root.width
        height: root.height
    }
}
