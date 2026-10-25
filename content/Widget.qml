pragma ComponentBehavior: Bound
import QtQuick
import Ryoku.PluginKit
import Ryoku.PluginKit.Singletons

Item {
    id: root

    property var pluginApi
    property var screen
    property bool active: false
    property string density: "glyph"
    property real s: 1
    property real widthBudget: 0
    property bool open: false
    property real reveal: 0
    property bool tabActive: false
    signal requestClose()

    readonly property var service: pluginApi ? pluginApi.mainInstance : null
    readonly property bool listening: service ? service.listening : false
    readonly property bool busy: service ? (service.busy || service.speaking) : false

    implicitWidth: icon.width
    implicitHeight: Math.max(icon.height, 18 * root.s)

    GlyphIcon {
        id: icon
        anchors.centerIn: parent
        width: 16 * root.s
        height: 16 * root.s
        name: root.service && !root.service.online ? "mic-off" : "mic"
        color: root.listening ? Theme.verm : (root.busy || root.open ? Theme.accent : Theme.dim)

        SequentialAnimation on opacity {
            running: root.listening || root.busy
            loops: Animation.Infinite
            onRunningChanged: if (!running) icon.opacity = 1
            NumberAnimation { to: 0.35; duration: Motion.shapeshift }
            NumberAnimation { to: 1; duration: Motion.shapeshift }
        }
    }

    MouseArea {
        anchors.fill: parent
        onClicked: {
            if (!root.pluginApi)
                return;
            if (root.open)
                root.requestClose();
            else if (root.pluginApi.togglePanel)
                root.pluginApi.togglePanel();
        }
    }
}
