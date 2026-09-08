// Where the shell's two external files live. PURE: no filesystem, no Qt GUI.
//
// Both functions take their environment as arguments rather than calling
// getenv(), which is what makes them testable without a process environment and
// without a display. The one caller that does read the real environment is
// `ShellConfig`, and it is a single line there.
//
// The socket path deliberately reproduces `core/src/config.rs::socket_path()`:
// `$TV_SHELL_CORE_SOCK`, else `/run/user/<uid>/tv-shell-core.sock`. It is
// duplicated rather than shared because the shell is a Qt/C++ tree and the core
// is a Rust crate; the duplication is pinned by a test that spells the same
// default the Rust constant does, so a rename on either side fails loudly here.
#pragma once

#include <cstdint>
#include <string>

namespace tvshell {

// The core's IPC socket. `sockEnv` is `$TV_SHELL_CORE_SOCK` (empty when unset).
std::string defaultCoreSocketPath(const std::string &sockEnv, std::uint32_t uid);

// The shell's catalog file — display metadata for launchable apps.
//
// `catalogEnv` is `$TV_SHELL_SHELL_JSON`, `xdgConfigHome` is `$XDG_CONFIG_HOME`
// and `home` is `$HOME`; all three may be empty. An empty result means "no path
// could be resolved at all" (no env, no XDG, no HOME), which the caller reports
// as an empty catalog rather than reading some fallback it made up.
std::string defaultCatalogPath(const std::string &catalogEnv, const std::string &xdgConfigHome,
                        const std::string &home);

} // namespace tvshell
