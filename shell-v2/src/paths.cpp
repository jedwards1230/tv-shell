#include "paths.h"

namespace tvshell {

namespace {
// An env var that is SET BUT EMPTY means "unset" everywhere here. The
// alternative — treating "" as a real path — turns a shell that exported the
// variable without a value into a shell that dials the empty path and reports a
// connection error, when what it means is "use the default".
bool unset(const std::string &v)
{
    return v.empty();
}
} // namespace

std::string defaultCoreSocketPath(const std::string &sockEnv, std::uint32_t uid)
{
    if (!unset(sockEnv))
        return sockEnv;
    return "/run/user/" + std::to_string(uid) + "/tv-shell-core.sock";
}

std::string defaultCatalogPath(const std::string &catalogEnv, const std::string &xdgConfigHome,
                        const std::string &home)
{
    if (!unset(catalogEnv))
        return catalogEnv;
    if (!unset(xdgConfigHome))
        return xdgConfigHome + "/tv-shell/shell.json";
    if (!unset(home))
        return home + "/.config/tv-shell/shell.json";
    return {};
}

} // namespace tvshell
