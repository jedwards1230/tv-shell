// The core client and the path rules, against a REAL Unix socket.
//
// The fake core here speaks the actual framing from core/src/protocol.rs —
// newline-delimited lines, one reply per command, in order. It is not a mock of
// CoreClient; nothing in this file knows CoreClient's internals. So a change to
// the framing, or to the queue discipline, fails here rather than on a
// television.
//
// docs/V2_SHELL.md records which mutation of coreclient.cpp each case caught.
#include "coreclient.h"
#include "paths.h"

#include <QCoreApplication>
#include <QLocalServer>
#include <QLocalSocket>
#include <QSignalSpy>
#include <QTemporaryDir>
#include <QTest>

#include <memory>

using namespace tvshell;

namespace {

// A minimal core: accepts one connection and replies to each line it receives
// with a line this test chose, in order.
class FakeCore : public QObject
{
    Q_OBJECT
public:
    explicit FakeCore(const QString &path)
    {
        QLocalServer::removeServer(path);
        m_server.listen(path);
        connect(&m_server, &QLocalServer::newConnection, this, [this] {
            m_conn = m_server.nextPendingConnection();
            // Capture the SOCKET, not `this->m_conn`. dropConnection() nulls
            // m_conn while the socket is still alive and can still emit, and a
            // lambda reading m_conn would then dereference null — which showed
            // up as heap corruption in a LATER test, not as a crash here.
            QLocalSocket *conn = m_conn;
            connect(conn, &QLocalSocket::readyRead, this, [this, conn] {
                m_received += conn->readAll();
            });
        });
    }

    bool isListening() const { return m_server.isListening(); }
    // Accepting a connection is asynchronous, so every test that writes a reply
    // must first QTRY_VERIFY this rather than assume the accept already ran.
    bool hasConnection() const { return m_conn != nullptr; }
    QByteArray received() const { return m_received; }

    void reply(const QByteArray &line)
    {
        QVERIFY(m_conn);
        m_conn->write(line + '\n');
        m_conn->flush();
    }

    // Write raw bytes, framing and all — for the desync cases.
    void writeRaw(const QByteArray &bytes)
    {
        QVERIFY(m_conn);
        m_conn->write(bytes);
        m_conn->flush();
    }

    // Hang up. Deliberately ONLY disconnectFromServer(): the socket is parented
    // to the server and the server deletes it, so calling close() or unwiring it
    // here races that ownership. Every test hangs up before its FakeCore goes out
    // of scope, because destroying a QLocalServer with a live peer aborts inside
    // Qt on this build.
    void dropConnection()
    {
        if (!m_conn)
            return;
        m_conn->disconnectFromServer();
        m_conn = nullptr;
    }

private:
    QLocalServer m_server;
    QLocalSocket *m_conn = nullptr;
    QByteArray m_received;
};

} // namespace

class TstCoreClient : public QObject
{
    Q_OBJECT

private Q_SLOTS:

    // ---- paths ------------------------------------------------------------

    // The env override wins, and the default reproduces the Rust constant
    // verbatim. Spelled out rather than referenced so a rename in
    // core/src/config.rs fails this test instead of silently splitting the two
    // trees' idea of where the socket is.
    void socketPath()
    {
        QCOMPARE(QString::fromStdString(defaultCoreSocketPath("/tmp/x.sock", 1000)),
                 QStringLiteral("/tmp/x.sock"));
        QCOMPARE(QString::fromStdString(defaultCoreSocketPath("", 1000)),
                 QStringLiteral("/run/user/1000/tv-shell-core.sock"));
        // An exported-but-empty variable means unset, not "the empty path".
        QCOMPARE(QString::fromStdString(defaultCoreSocketPath("", 0)),
                 QStringLiteral("/run/user/0/tv-shell-core.sock"));
    }

    void catalogPathPrecedence()
    {
        QCOMPARE(QString::fromStdString(defaultCatalogPath("/a/b.json", "/xdg", "/home/u")),
                 QStringLiteral("/a/b.json"));
        QCOMPARE(QString::fromStdString(defaultCatalogPath("", "/xdg", "/home/u")),
                 QStringLiteral("/xdg/tv-shell/shell.json"));
        QCOMPARE(QString::fromStdString(defaultCatalogPath("", "", "/home/u")),
                 QStringLiteral("/home/u/.config/tv-shell/shell.json"));
        // Nothing resolvable is EMPTY, not a guess. A caller must be able to
        // tell "there is no path" from "here is a path that does not exist".
        QCOMPARE(QString::fromStdString(defaultCatalogPath("", "", "")), QString());
    }

    // ---- K1: replies pair with commands in send order ----------------------

    void repliesPairInOrder()
    {
        QTemporaryDir dir;
        const QString path = dir.filePath(QStringLiteral("core.sock"));
        FakeCore core(path);
        QVERIFY(core.isListening());

        CoreClient client;
        client.setSocketPath(path);
        QSignalSpy replies(&client, &CoreClient::replyReceived);
        client.connectToCore();
        QTRY_VERIFY(client.isConnected());
        QTRY_VERIFY(core.hasConnection());

        QVERIFY(client.request(QStringLiteral("ping")));
        QVERIFY(client.request(QStringLiteral("screen-state")));
        QTRY_COMPARE(core.received(), QByteArray("ping\nscreen-state\n"));

        core.reply("ok");
        core.reply("{\"focusable_apps\":[]}");
        QTRY_COMPARE(replies.count(), 2);

        QCOMPARE(replies.at(0).at(0).toString(), QStringLiteral("ping"));
        QCOMPARE(replies.at(0).at(1).toString(), QStringLiteral("ok"));
        QCOMPARE(replies.at(1).at(0).toString(), QStringLiteral("screen-state"));
        QCOMPARE(replies.at(1).at(1).toString(), QStringLiteral("{\"focusable_apps\":[]}"));
        core.dropConnection();
    }

    // Two replies arriving in ONE read must still be split and paired. This is
    // the case a naive "one readyRead, one reply" implementation gets wrong, and
    // it is invisible on a fast local socket until it is not.
    void coalescedRepliesAreSplit()
    {
        QTemporaryDir dir;
        const QString path = dir.filePath(QStringLiteral("core.sock"));
        FakeCore core(path);
        CoreClient client;
        client.setSocketPath(path);
        QSignalSpy replies(&client, &CoreClient::replyReceived);
        client.connectToCore();
        QTRY_VERIFY(client.isConnected());
        QTRY_VERIFY(core.hasConnection());

        QVERIFY(client.request(QStringLiteral("a")));
        QVERIFY(client.request(QStringLiteral("b")));
        QTRY_COMPARE(core.received(), QByteArray("a\nb\n"));
        core.writeRaw("first\nsecond\n");

        QTRY_COMPARE(replies.count(), 2);
        QCOMPARE(replies.at(0).at(1).toString(), QStringLiteral("first"));
        QCOMPARE(replies.at(1).at(0).toString(), QStringLiteral("b"));
        core.dropConnection();
    }

    // A reply split ACROSS reads is one reply, not two and not a truncation.
    void partialReplyIsBuffered()
    {
        QTemporaryDir dir;
        const QString path = dir.filePath(QStringLiteral("core.sock"));
        FakeCore core(path);
        CoreClient client;
        client.setSocketPath(path);
        QSignalSpy replies(&client, &CoreClient::replyReceived);
        client.connectToCore();
        QTRY_VERIFY(client.isConnected());
        QTRY_VERIFY(core.hasConnection());
        QVERIFY(client.request(QStringLiteral("screen-state")));
        QTRY_COMPARE(core.received(), QByteArray("screen-state\n"));

        core.writeRaw("{\"focusable");
        QTest::qWait(20);
        QCOMPARE(replies.count(), 0);
        core.writeRaw("_apps\":[9003]}\n");
        QTRY_COMPARE(replies.count(), 1);
        QCOMPARE(replies.at(0).at(1).toString(), QStringLiteral("{\"focusable_apps\":[9003]}"));
        core.dropConnection();
    }

    // ---- K2: a control character is refused, never sent ---------------------

    void newlineInCommandIsRefused()
    {
        QTemporaryDir dir;
        const QString path = dir.filePath(QStringLiteral("core.sock"));
        FakeCore core(path);
        CoreClient client;
        client.setSocketPath(path);
        QSignalSpy refused(&client, &CoreClient::requestRefused);
        client.connectToCore();
        QTRY_VERIFY(client.isConnected());

        QVERIFY(!client.request(QStringLiteral("launch 1\nhome")));
        QCOMPARE(refused.count(), 1);
        // Nothing reached the wire: the desync this rule prevents starts with a
        // byte being written, so the assertion is about bytes, not about the
        // return value.
        QTest::qWait(20);
        QCOMPARE(core.received(), QByteArray());

        // A carriage return is the same hazard.
        QVERIFY(!client.request(QStringLiteral("show 1\rhome")));
        QCOMPARE(refused.count(), 2);
        core.dropConnection();
    }

    // ---- K3: an over-long command is refused, never truncated ---------------

    void oversizedCommandIsRefused()
    {
        QTemporaryDir dir;
        const QString path = dir.filePath(QStringLiteral("core.sock"));
        FakeCore core(path);
        CoreClient client;
        client.setSocketPath(path);
        QSignalSpy refused(&client, &CoreClient::requestRefused);
        client.connectToCore();
        QTRY_VERIFY(client.isConnected());

        // Exactly at the limit once the newline is added: accepted.
        const QString atLimit(CoreClient::MaxLine - 1, QLatin1Char('x'));
        QVERIFY(client.request(atLimit));
        QTRY_COMPARE(core.received().size(), qsizetype(CoreClient::MaxLine));
        // One byte more: refused.
        const QString tooLong(CoreClient::MaxLine, QLatin1Char('x'));
        QVERIFY(!client.request(tooLong));
        QCOMPARE(refused.count(), 1);
        // And nothing further reached the wire: the rule is about bytes, not
        // about the return value.
        QTest::qWait(20);
        QCOMPARE(core.received().size(), qsizetype(CoreClient::MaxLine));
        core.dropConnection();
        QTRY_VERIFY(!client.isConnected());
    }

    // ---- K4: a disconnect clears the queue ---------------------------------

    // The failure this prevents: command A is in flight, the core restarts, and
    // A's reply arrives on the NEW connection and is paired with command B.
    // Every reply after that is off by one, forever, and nothing looks broken.
    void disconnectClearsPendingCommands()
    {
        QTemporaryDir dir;
        const QString path = dir.filePath(QStringLiteral("core.sock"));
        CoreClient client;
        QSignalSpy replies(&client, &CoreClient::replyReceived);

        {
            FakeCore core(path);
            client.setSocketPath(path);
            client.connectToCore();
            QTRY_VERIFY(client.isConnected());
            QVERIFY(client.request(QStringLiteral("screen-state")));
            QTRY_COMPARE(core.received(), QByteArray("screen-state\n"));
            core.dropConnection();
            QTRY_VERIFY(!client.isConnected());
        }

        FakeCore core2(path);
        client.connectToCore();
        QTRY_VERIFY(client.isConnected());
        QTRY_VERIFY(core2.hasConnection());
        core2.reply("ok");
        QTest::qWait(50);
        // The stale command is gone, so the stray reply pairs with nothing and
        // is dropped rather than mis-attributed.
        QCOMPARE(replies.count(), 0);

        // And the connection still works for a command sent AFTER the reconnect.
        QVERIFY(client.request(QStringLiteral("ping")));
        QTRY_COMPARE(core2.received(), QByteArray("ping\n"));
        core2.reply("ok");
        QTRY_COMPARE(replies.count(), 1);
        QCOMPARE(replies.at(0).at(0).toString(), QStringLiteral("ping"));
        core2.dropConnection();
    }

    // ---- teardown ----------------------------------------------------------

    // Destroying a CONNECTED client must be safe and quiet.
    //
    // This defends a fix, and the bug it defends against is worth stating because
    // it is invisible by every other means. Qt destroys a connected QLocalSocket
    // MEMBER by running its close path from inside ~CoreClient, at which point
    // the derived object is already half-destroyed -- and its slots ran anyway.
    // Without the explicit `m_socket.disconnect(this)` in the destructor this
    // corrupts the heap, and it does NOT fail here: it surfaces later, as a glibc
    // abort attributed to whichever test happens to run next. An
    // AddressSanitizer build of this same suite reports ZERO errors, because the
    // fault is in Qt's signal dispatch rather than in a heap access ASan
    // instruments.
    //
    // So the test is a loop rather than a single destruction: one teardown can
    // corrupt quietly and be tolerated, twenty cannot. Verified by mutation --
    // removing the destructor's body fails this.
    void destroyingAConnectedClientIsQuiet()
    {
        QTemporaryDir dir;
        const QString path = dir.filePath(QStringLiteral("core.sock"));
        FakeCore core(path);
        QVERIFY(core.isListening());

        for (int i = 0; i < 20; ++i) {
            auto client = std::make_unique<CoreClient>();
            client->setSocketPath(path);
            client->connectToCore();
            QTRY_VERIFY(client->isConnected());
            // In flight on purpose: a pending command is what gives the
            // half-destroyed object something to do on the way out.
            QVERIFY(client->request(QStringLiteral("screen-state")));
            client.reset();
        }
        // Reaching here at all is the assertion; make it explicit so a reader
        // does not mistake this for a test with no expectations.
        QVERIFY(true);
        core.dropConnection();
    }

    // A request with no connection is refused rather than silently queued: a
    // queue that survives "the core is not running" would replay stale intent
    // whenever it came back.
    void requestWithoutConnectionIsRefused()
    {
        CoreClient client;
        client.setSocketPath(QStringLiteral("/nonexistent/tv-shell-core.sock"));
        QSignalSpy refused(&client, &CoreClient::requestRefused);
        QVERIFY(!client.request(QStringLiteral("ping")));
        QCOMPARE(refused.count(), 1);
    }
};

QTEST_MAIN(TstCoreClient)
#include "tst_coreclient.moc"
