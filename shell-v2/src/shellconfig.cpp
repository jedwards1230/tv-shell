#include "shellconfig.h"

#include "paths.h"

#include <QFile>
#include <QFileInfo>

namespace tvshell {

namespace {
QString env(const char *name)
{
    return QString::fromLocal8Bit(qgetenv(name));
}
} // namespace

ShellConfig::ShellConfig(QObject *parent)
    : QObject(parent)
{
    m_catalogPath = QString::fromStdString(defaultCatalogPath(env("TV_SHELL_SHELL_JSON").toStdString(),
                                                       env("XDG_CONFIG_HOME").toStdString(),
                                                       env("HOME").toStdString()));
}

void ShellConfig::setCatalogPath(const QString &path)
{
    if (m_catalogPath == path)
        return;
    m_catalogPath = path;
    Q_EMIT catalogPathChanged();
}

void ShellConfig::reload()
{
    if (m_catalogPath.isEmpty()) {
        // No env, no XDG_CONFIG_HOME, no HOME. Nothing to read and nothing the
        // user did wrong; an empty catalog is the honest result.
        setCatalogText(QString());
        setLastError(QString());
        return;
    }
    QFile file(m_catalogPath);
    if (!file.exists()) {
        setCatalogText(QString());
        setLastError(QString());
        return;
    }
    if (!file.open(QIODevice::ReadOnly | QIODevice::Text)) {
        setCatalogText(QString());
        setLastError(file.errorString());
        return;
    }
    setCatalogText(QString::fromUtf8(file.readAll()));
    setLastError(QString());
}

void ShellConfig::setCatalogText(const QString &text)
{
    if (m_catalogText == text)
        return;
    m_catalogText = text;
    Q_EMIT catalogTextChanged();
}

void ShellConfig::setLastError(const QString &error)
{
    if (m_lastError == error)
        return;
    m_lastError = error;
    Q_EMIT lastErrorChanged();
}

} // namespace tvshell
