// `CoreClient` — the shell's ONLY channel to tv-shell-core.
//
// WHY THIS IS A TYPE AND NOT A SHELL-OUT
//
// v1's shell has 50 `Process {` sites: every read of system state is a forked
// command whose failure mode is a blank string. v2 has one socket and no
// `Process` anywhere. If a screen needs something, it either comes down this
// socket or the core does not publish it — and "the core does not publish it" is
// a finding to report, not a licence to fork `hyprctl`.
//
// THE PROTOCOL, AND THE ONE RULE THAT MAKES IT SAFE
//
// `core/src/protocol.rs`: newline-delimited text, one command line in, one reply
// line out, 4096 bytes maximum, replies in the order the commands were sent.
// There are no request ids. So pairing a reply to its command is entirely a
// matter of keeping a FIFO and never letting it desync, and every rule below
// exists to protect that FIFO:
//
//   K1  Replies pair with commands in send order.
//   K2  A command containing a control character is REFUSED, never sent. An
//       embedded newline would be read by the core as two commands, and the
//       second reply would pair with the next command forever after. (The core
//       sanitizes what it sends us for the same reason; this is that rule's
//       other half, which only the client can enforce.)
//   K3  A command at or over MAX_LINE is REFUSED, never truncated. A truncated
//       command is a different, possibly valid, command.
//   K4  A disconnect CLEARS the queue. A reply that arrives after a reconnect
//       belongs to no command we still remember, and pairing it with one would
//       be worse than dropping it.
//
// Every one of those is asserted in tests/tst_coreclient.cpp against a real
// QLocalServer speaking the real framing — not a mock of this class.
//
// WHAT IT DOES NOT DO
//
// It does not poll. `core/src/protocol.rs` states there is deliberately no event
// stream yet, so there is nothing to subscribe to; the shell requests a snapshot
// at the moments that can have changed it (startup, its own command completing,
// the base surface becoming visible) and is otherwise silent. A timer here would
// be inventing liveness the core does not offer.
#pragma once

#include <QLocalSocket>
#include <QObject>
#include <QQueue>
#include <QtQml/qqmlregistration.h>

namespace tvshell {

class CoreClient : public QObject
{
    Q_OBJECT
    QML_ELEMENT

    // Resolved once at construction from the environment (see paths.h). Settable
    // so a test can point it at a scratch socket without touching the process
    // environment.
    Q_PROPERTY(QString socketPath READ socketPath WRITE setSocketPath NOTIFY socketPathChanged)
    Q_PROPERTY(bool connected READ isConnected NOTIFY connectedChanged)
    // The last transport-level failure, "" when there has not been one since the
    // last successful connect. A *protocol* error (`error:...`) is not one of
    // these — it arrives as an ordinary reply, because the core answered.
    Q_PROPERTY(QString lastError READ lastError NOTIFY lastErrorChanged)

public:
    // Matches core/src/protocol.rs MAX_LINE.
    static constexpr int MaxLine = 4096;

    explicit CoreClient(QObject *parent = nullptr);
    ~CoreClient() override;

    QString socketPath() const { return m_socketPath; }
    void setSocketPath(const QString &path);

    bool isConnected() const;
    QString lastError() const { return m_lastError; }

    // Dial the core. Idempotent while already connected or connecting.
    Q_INVOKABLE void connectToCore();

    // Queue one command. Returns false — and emits nothing, and puts nothing on
    // the wire — when the command violates K2/K3 or there is no connection.
    // The bool return is what lets a caller distinguish "refused" from "sent and
    // awaiting a reply"; a void signature would make a refusal look like a slow
    // core forever.
    Q_INVOKABLE bool request(const QString &command);

Q_SIGNALS:
    void socketPathChanged();
    void connectedChanged();
    void lastErrorChanged();
    // One per reply, carrying the command it answers (K1).
    void replyReceived(const QString &command, const QString &reply);
    // A command that was refused before reaching the wire, with why. Separate
    // from `lastError` because it is a bug in the CALLER, not a transport fault.
    void requestRefused(const QString &command, const QString &reason);

private:
    void onReadyRead();
    void onErrorOccurred(QLocalSocket::LocalSocketError error);
    void onDisconnected();
    void setLastError(const QString &error);

    QLocalSocket m_socket;
    QString m_socketPath;
    QString m_lastError;
    // Commands sent and not yet answered, oldest first. THE invariant of this
    // class: its head is the command the next reply line answers.
    QQueue<QString> m_pending;
    QByteArray m_buffer;
};

} // namespace tvshell
