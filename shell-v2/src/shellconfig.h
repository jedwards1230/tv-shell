// `ShellConfig` — reads the shell's catalog file and hands QML its raw text.
//
// WHY THE SHELL HAS A CATALOG AT ALL
//
// The core owns launching: `[[app]]` in `core.toml` carries the argv, the env,
// and the env_unset that decide whether an app maps a window. It does NOT
// publish that table — there is no `list-apps` verb in `core/src/protocol.rs` —
// so the shell has no way to ask what is launchable, and it must not read
// `core.toml` and become a second parser of the core's schema.
//
// So the split is by ownership rather than by convenience: the core knows how to
// START an app, the shell knows what an app is CALLED and what it looks like.
// The catalog holds only the second kind of fact, and launching is still
// `launch <appid>` — the class form — so the argv exists in exactly one place.
// This is recorded as a gap: a core `list-apps` would let the shell take ids
// from the core and keep this file purely cosmetic.
//
// WHY THE TEXT AND NOT A PARSED MODEL
//
// This class does I/O and nothing else. The parse, the validation and every
// decision about a malformed row live in `catalog.js`, which is pure and tested
// headlessly. A C++ parser here would be a second place where the catalog schema
// is known, and the untested one.
#pragma once

#include <QObject>
#include <QString>
#include <QtQml/qqmlregistration.h>

namespace tvshell {

class ShellConfig : public QObject
{
    Q_OBJECT
    QML_ELEMENT

    // Resolved at construction from the environment (paths.h). Writable so a
    // test can point it at a fixture.
    Q_PROPERTY(QString catalogPath READ catalogPath WRITE setCatalogPath NOTIFY catalogPathChanged)
    // The file's contents, or "" when there is no readable file. Refreshed by
    // reload().
    Q_PROPERTY(QString catalogText READ catalogText NOTIFY catalogTextChanged)
    // Why the last reload() produced no text, or "" when it produced text OR
    // when the file simply does not exist.
    //
    // A MISSING catalog is not an error: a box with no catalog is a box that has
    // not configured one yet, and the shell's answer to that is an empty screen
    // with an empty state, not a red banner. An UNREADABLE one (permissions, a
    // directory where a file should be) IS an error, because something is wrong
    // that the user can fix.
    Q_PROPERTY(QString lastError READ lastError NOTIFY lastErrorChanged)

public:
    explicit ShellConfig(QObject *parent = nullptr);

    QString catalogPath() const { return m_catalogPath; }
    void setCatalogPath(const QString &path);

    QString catalogText() const { return m_catalogText; }
    QString lastError() const { return m_lastError; }

    Q_INVOKABLE void reload();

Q_SIGNALS:
    void catalogPathChanged();
    void catalogTextChanged();
    void lastErrorChanged();

private:
    void setCatalogText(const QString &text);
    void setLastError(const QString &error);

    QString m_catalogPath;
    QString m_catalogText;
    QString m_lastError;
};

} // namespace tvshell
