#include "coreclient.h"

#include "paths.h"

#include <QLoggingCategory>

#include <unistd.h>

Q_LOGGING_CATEGORY(lcCore, "tvshell.core")

namespace tvshell {

namespace {
QString envOrEmpty(const char *name)
{
    // Note the missing `.constData()`: qgetenv returns a temporary QByteArray,
    // so a pointer into it dangles the moment this expression ends.
    return QString::fromLocal8Bit(qgetenv(name));
}
} // namespace

CoreClient::CoreClient(QObject *parent)
    : QObject(parent)
{
    m_socketPath = QString::fromStdString(defaultCoreSocketPath(
            envOrEmpty("TV_SHELL_CORE_SOCK").toStdString(), static_cast<std::uint32_t>(getuid())));

    connect(&m_socket, &QLocalSocket::readyRead, this, &CoreClient::onReadyRead);
    connect(&m_socket, &QLocalSocket::errorOccurred, this, &CoreClient::onErrorOccurred);
    connect(&m_socket, &QLocalSocket::disconnected, this, &CoreClient::onDisconnected);
    connect(&m_socket, &QLocalSocket::connected, this, [this] {
        setLastError(QString());
        Q_EMIT connectedChanged();
    });
}

CoreClient::~CoreClient()
{
    // Tear the socket down explicitly, and unwire it first. Qt destroys a
    // connected QLocalSocket member by running its close path from inside the
    // destructor, at which point this object is already half-destroyed and its
    // slots must not run.
    m_socket.disconnect(this);
    m_socket.abort();
}

void CoreClient::setSocketPath(const QString &path)
{
    if (m_socketPath == path)
        return;
    m_socketPath = path;
    Q_EMIT socketPathChanged();
}

bool CoreClient::isConnected() const
{
    return m_socket.state() == QLocalSocket::ConnectedState;
}

void CoreClient::connectToCore()
{
    if (m_socket.state() != QLocalSocket::UnconnectedState)
        return;
    if (m_socketPath.isEmpty()) {
        setLastError(QStringLiteral("no socket path"));
        return;
    }
    m_socket.connectToServer(m_socketPath);
}

bool CoreClient::request(const QString &command)
{
    // K2. Checked BEFORE the connection state so a malformed command is
    // reported as malformed whether or not the core happens to be up — the
    // caller's bug does not change with the weather.
    for (const QChar c : command) {
        if (c.category() == QChar::Other_Control) {
            Q_EMIT requestRefused(command, QStringLiteral("control character in command"));
            return false;
        }
    }
    // K3. The wire line is the command plus one newline, so the command itself
    // must leave room for that byte.
    const QByteArray line = command.toUtf8() + '\n';
    if (line.size() > MaxLine) {
        Q_EMIT requestRefused(command, QStringLiteral("command exceeds the 4096-byte line limit"));
        return false;
    }
    if (!isConnected()) {
        Q_EMIT requestRefused(command, QStringLiteral("not connected"));
        return false;
    }

    // Enqueue BEFORE writing. The reverse order has a real hole: a synchronous
    // readyRead delivered from inside write() would find an empty queue and drop
    // a reply that has a command.
    m_pending.enqueue(command);
    m_socket.write(line);
    return true;
}

void CoreClient::onReadyRead()
{
    m_buffer += m_socket.readAll();
    int nl;
    while ((nl = m_buffer.indexOf('\n')) >= 0) {
        const QByteArray line = m_buffer.left(nl);
        m_buffer.remove(0, nl + 1);
        if (m_pending.isEmpty()) {
            // Only reachable after K4 cleared the queue, or from a core that
            // sent an unsolicited line. Either way it answers no command we
            // know, and guessing which would desync everything after it.
            qCWarning(lcCore, "unsolicited reply line dropped");
            continue;
        }
        const QString command = m_pending.dequeue();
        Q_EMIT replyReceived(command, QString::fromUtf8(line));
    }
    // A partial line longer than a whole line can never complete: the core never
    // sends one, so this is a desynced or hostile peer. Dropping the connection
    // is the only recovery that restores the FIFO invariant.
    if (m_buffer.size() > MaxLine) {
        setLastError(QStringLiteral("oversized reply line"));
        m_socket.abort();
        onDisconnected();
    }
}

void CoreClient::onErrorOccurred(QLocalSocket::LocalSocketError error)
{
    Q_UNUSED(error);
    setLastError(m_socket.errorString());
}

void CoreClient::onDisconnected()
{
    // K4. Both halves matter: a stale command would mis-pair with the first
    // reply after a reconnect, and a stale partial line would prepend itself to
    // the new connection's first reply.
    const bool had = !m_pending.isEmpty();
    m_pending.clear();
    m_buffer.clear();
    if (had)
        qCInfo(lcCore, "core disconnected; dropped in-flight commands");
    Q_EMIT connectedChanged();
}

void CoreClient::setLastError(const QString &error)
{
    if (m_lastError == error)
        return;
    m_lastError = error;
    Q_EMIT lastErrorChanged();
}

} // namespace tvshell
