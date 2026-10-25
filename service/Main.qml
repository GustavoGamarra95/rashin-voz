pragma ComponentBehavior: Bound

import QtQuick
import Quickshell.Io

Item {
    id: svc

    property var pluginApi
    readonly property var settings: pluginApi ? pluginApi.pluginSettings : null
    readonly property bool speakReplies: settings ? (settings.speakReplies ?? true) : true

    property string state: "idle"
    property string heard: ""
    property string reply: ""
    property string activity: ""
    property string error: ""
    property bool online: false

    readonly property bool listening: state === "listening"
    readonly property bool busy: state === "transcribing" || state === "thinking"
    readonly property bool speaking: state === "speaking"

    function command(line) {
        if (engine.running)
            engine.write(line + "\n");
    }

    function toggle() {
        svc.error = "";
        command("toggle");
    }

    function silence() {
        command("silence");
    }

    function say(text) {
        var t = String(text).trim();
        if (t.length === 0 || svc.busy)
            return;
        svc.error = "";
        svc.heard = t;
        svc.reply = "";
        command("say " + t.replace(/\n/g, " "));
    }

    function handle(line) {
        var e;
        try {
            e = JSON.parse(String(line));
        } catch (err) {
            return;
        }
        if (e.ev === "state") {
            svc.state = e.state;
            if (e.state !== "thinking")
                svc.activity = "";
        } else if (e.ev === "heard") {
            svc.heard = e.text;
            svc.reply = "";
        } else if (e.ev === "reply") {
            svc.reply = e.text;
        } else if (e.ev === "activity") {
            svc.activity = e.text;
        } else if (e.ev === "error") {
            svc.error = e.text;
        }
    }

    onSpeakRepliesChanged: command("speak " + (speakReplies ? "on" : "off"))

    Process {
        id: engine
        command: ["rashin-voz", "serve"]
        running: true
        stdinEnabled: true
        stdout: SplitParser {
            splitMarker: "\n"
            onRead: (line) => svc.handle(line)
        }
        onStarted: {
            svc.online = true;
            svc.command("speak " + (svc.speakReplies ? "on" : "off"));
        }
        onExited: {
            svc.online = false;
            svc.state = "idle";
            restart.start();
        }
    }

    Timer {
        id: restart
        interval: 3000
        onTriggered: engine.running = true
    }
}
