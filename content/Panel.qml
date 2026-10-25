import QtQuick
import Ryoku.PluginKit
import Ryoku.PluginKit.Singletons

Item {
    id: root

    property var pluginApi
    property string density: "full"
    property real s: 1
    property real widthBudget: 360
    property bool active: false

    readonly property var service: pluginApi ? pluginApi.mainInstance : null
    readonly property string phase: service ? service.state : "idle"
    readonly property bool listening: phase === "listening"

    readonly property string status: {
        if (!root.service || !root.service.online)
            return "Iniciando…";
        if (root.service.error.length > 0)
            return root.service.error;
        switch (root.phase) {
        case "listening": return "Te escucho… pulsa para enviar";
        case "transcribing": return "Transcribiendo…";
        case "thinking": return root.service.activity.length > 0 ? "Rashin: " + root.service.activity : "Rashin está pensando…";
        case "speaking": return "Hablando… pulsa para interrumpir";
        default: return "Pulsa el micrófono y habla";
        }
    }

    implicitWidth: root.widthBudget
    implicitHeight: col.implicitHeight + 24 * root.s

    Column {
        id: col
        x: 12 * root.s
        y: 12 * root.s
        width: root.width - 24 * root.s
        spacing: 12 * root.s

        MicroLabel {
            label: root.service && root.service.activeLanguage.length > 0 ? "Rashin Voz · " + root.service.activeLanguage.toUpperCase() : "Rashin Voz"
            s: root.s
        }

        Item {
            width: parent.width
            height: 84 * root.s

            Rectangle {
                id: halo
                anchors.centerIn: mic
                width: mic.width
                height: width
                radius: width / 2
                color: "transparent"
                border.width: 2 * root.s
                border.color: Theme.verm
                visible: root.listening
                opacity: 0

                ParallelAnimation {
                    running: root.listening
                    loops: Animation.Infinite
                    NumberAnimation { target: halo; property: "scale"; from: 1; to: 1.45; duration: Motion.heat }
                    NumberAnimation { target: halo; property: "opacity"; from: 0.8; to: 0; duration: Motion.heat }
                }
            }

            Rectangle {
                id: mic
                anchors.centerIn: parent
                width: 64 * root.s
                height: width
                radius: width / 2
                color: root.listening ? Theme.verm : (micArea.containsMouse ? Theme.accent : Theme.tileBg)
                border.width: 1
                border.color: root.listening ? Theme.vermDeep : Theme.lineStrong

                Behavior on color { ColorAnimation { duration: Motion.fast } }

                GlyphIcon {
                    anchors.centerIn: parent
                    width: 28 * root.s
                    height: 28 * root.s
                    name: "mic"
                    stroke: 2
                    color: root.listening || micArea.containsMouse ? Theme.cardBot : Theme.bright
                }

                MouseArea {
                    id: micArea
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    enabled: root.service && root.service.online && root.phase !== "transcribing" && root.phase !== "thinking"
                    onClicked: {
                        if (root.phase === "speaking")
                            root.service.silence();
                        else
                            root.service.toggle();
                    }
                }
            }
        }

        Text {
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            text: root.status
            color: root.service && root.service.error.length > 0 ? Theme.vermLit : Theme.dim
            font.family: Theme.font
            font.pixelSize: 12 * root.s
            wrapMode: Text.WordWrap
        }

        Rectangle {
            width: parent.width
            height: 1
            color: Theme.hair
            visible: heardText.visible || replyText.visible
        }

        Text {
            id: heardText
            width: parent.width
            visible: root.service && root.service.heard.length > 0
            text: root.service ? "Tú: " + root.service.heard : ""
            color: Theme.dim
            font.family: Theme.font
            font.pixelSize: 13 * root.s
            wrapMode: Text.WordWrap
        }

        Flickable {
            width: parent.width
            height: Math.min(replyText.implicitHeight, 220 * root.s)
            visible: replyText.visible
            contentHeight: replyText.implicitHeight
            clip: true

            Text {
                id: replyText
                width: parent.width
                visible: root.service && root.service.reply.length > 0
                text: root.service ? root.service.reply : ""
                textFormat: Text.MarkdownText
                color: Theme.bright
                font.family: Theme.font
                font.pixelSize: 13 * root.s
                wrapMode: Text.WordWrap
            }
        }

        SearchField {
            id: field
            width: parent.width
            s: root.s
            placeholder: "O escribe aquí y pulsa Enter"
            onAccepted: {
                if (!root.service)
                    return;
                root.service.say(field.text);
                field.text = "";
            }
            onDismissed: if (root.pluginApi) root.pluginApi.closePanel()
        }
    }
}
