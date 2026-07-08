pragma Singleton
import QtQuick

Item {
    function matchesApp(app, client) {
        let cls = (client["class"] || "").toLowerCase();
        let initCls = (client["initialClass"] || "").toLowerCase();
        if (cls === "" && initCls === "")
            return false;

        // 1. StartupWMClass match (most reliable)
        let wmClass = (app.wmClass || "").toLowerCase();
        if (wmClass !== "") {
            if (cls === wmClass || initCls === wmClass)
                return true;
            let normWm = normalize(wmClass);
            if (normalize(cls) === normWm || normalize(initCls) === normWm)
                return true;
        }

        // 2. Exec basename match
        let execBase = execBasename(app.exec || "");
        if (execBase !== "") {
            if (cls === execBase || initCls === execBase)
                return true;
            if (normalize(cls) === normalize(execBase) || normalize(initCls) === normalize(execBase))
                return true;
        }

        // 3. App name match (exact only)
        let appName = (app.name || "").toLowerCase();
        if (appName !== "" && (cls === appName || initCls === appName))
            return true;

        // 4. Substring: class/initialClass within exec or vice versa
        if (execBase !== "") {
            if (cls !== "" && (execBase.indexOf(cls) >= 0 || cls.indexOf(execBase) >= 0))
                return true;
            if (initCls !== "" && (execBase.indexOf(initCls) >= 0 || initCls.indexOf(execBase) >= 0))
                return true;
        }

        return false;
    }

    // Public helpers (also used by HomeScreen's recent-model matching). Null-guarded
    // supersets of the former privates — behaviour-identical at every call site,
    // which already passes guarded (`|| ""`) strings.
    function execBasename(exec) {
        if (!exec)
            return "";
        let cmd = exec.split(/\s/)[0];
        return cmd.split("/").pop().toLowerCase();
    }

    function normalize(s) {
        return (s || "").toLowerCase().replace(/[-_.]/g, "");
    }
}
