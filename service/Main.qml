pragma ComponentBehavior: Bound

import QtQuick
import Quickshell.Io

Item {
    id: svc

    property var pluginApi
    readonly property var settings: pluginApi ? pluginApi.pluginSettings : null
    readonly property bool speakReplies: settings ? (settings.speakReplies ?? true) : true
    readonly property string language: settings ? (settings.language ?? "auto") : "auto"
    readonly property bool wakeWord: settings ? (settings.wakeWord ?? true) : true
    property string activeLanguage: ""
    property bool armed: false

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
        } else if (e.ev === "armed") {
            svc.armed = e.on;
        } else if (e.ev === "wake") {
            svc.error = "";
        } else if (e.ev === "lang") {
            svc.activeLanguage = e.lang;
        } else if (e.ev === "error") {
            svc.error = e.text;
        }
    }

    onSpeakRepliesChanged: command("speak " + (speakReplies ? "on" : "off"))
    onLanguageChanged: command("lang " + language)
    onWakeWordChanged: command("wake " + (wakeWord ? "on" : "off"))

    Process {
        id: engine
        command: ["rashin-voz", "serve", "--tts", svc.pluginApi ? svc.pluginApi.pluginDir + "/bin/rashin-voz-tts" : ""]
        running: !!svc.pluginApi
        stdinEnabled: true
        stdout: SplitParser {
            splitMarker: "\n"
            onRead: (line) => svc.handle(line)
        }
        onStarted: {
            svc.online = true;
            svc.command("speak " + (svc.speakReplies ? "on" : "off"));
            svc.command("lang " + svc.language);
            svc.command("wake " + (svc.wakeWord ? "on" : "off"));
        }
        onExited: {
            svc.online = false;
            svc.armed = false;
            svc.state = "idle";
            restart.start();
        }
    }

    Timer {
        id: restart
        interval: 3000
        onTriggered: engine.running = !!svc.pluginApi
    }
}
