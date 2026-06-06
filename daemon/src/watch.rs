//! File-watch actor (Linux-only): watches `~/.config/game-shell/settings.json`
//! for external modifications and notifies the input runtime via a dedicated
//! [`tokio::sync::Notify`] channel (not the global event broadcast bus).
//!
//! **Design**: the daemon is the sole writer of `settings.json` (via
//! `config::set_config` and `config::save_bindings`). Those functions call
//! `config::note_self_write()` before every `std::fs::write`, bumping a
//! module-level `AtomicU64`. The watch task records the generation after each
//! handled batch. On a debounced inotify event it re-reads the counter: if the
//! generation advanced since the last emit, the write was daemon-originated and
//! the event is suppressed. External edits never bump the counter, so they
//! always fire.
//!
//! **Directory watch**: atomic write-then-rename external editors (editors,
//! `echo >`, `ansible.copy`) replace the inode. Watching the FILE directly
//! goes deaf after the first rename; we watch the **parent directory** and
//! filter events whose path basename == `settings.json`.
//!
//! **Degradation**: if the watcher cannot be established (e.g. inotify
//! unavailable), we log a warning and return — live-reload degrades to
//! restart-required, matching the behaviour of the D-Bus actors when their
//! service is absent.

#[cfg(target_os = "linux")]
pub async fn run(config_changed: std::sync::Arc<tokio::sync::Notify>) {
    use crate::config;
    use notify_debouncer_full::{new_debouncer, notify::RecursiveMode, notify::Watcher};
    use std::sync::mpsc as std_mpsc;
    use std::time::Duration;

    let settings_path = config::settings_path();
    let watch_dir = match settings_path.parent() {
        Some(d) => d.to_path_buf(),
        None => {
            tracing::warn!("watch: settings path has no parent directory, live-reload disabled");
            return;
        }
    };

    // Ensure the config directory exists before we try to watch it (first-run
    // may precede any write).
    if let Err(e) = std::fs::create_dir_all(&watch_dir) {
        tracing::warn!(
            "watch: could not create config dir {}: {e}",
            watch_dir.display()
        );
        // Not fatal — the watch itself may still succeed.
    }

    // Build the debounced watcher with a ~200ms window feeding a std mpsc.
    let (std_tx, std_rx) = std_mpsc::channel();
    let mut debouncer = match new_debouncer(Duration::from_millis(200), None, std_tx) {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!("watch: could not create file debouncer: {e}, live-reload disabled");
            return;
        }
    };

    // Watch the parent directory (not the file) so rename-based atomic writes
    // (write-temp + rename) are detected even after an inode replacement.
    // notify-debouncer-full 0.3: the watch() method is on the inner Watcher,
    // accessed via debouncer.watcher().
    if let Err(e) = debouncer
        .watcher()
        .watch(&watch_dir, RecursiveMode::NonRecursive)
    {
        tracing::warn!(
            "watch: could not watch {}: {e}, live-reload disabled",
            watch_dir.display()
        );
        return;
    }

    tracing::info!(
        "watch: watching {} for external settings.json changes",
        watch_dir.display()
    );

    // Keep debouncer alive for the duration of the watch loop.
    let _debouncer = debouncer;

    // Snapshot the self-write generation at task start.
    let mut last_seen_gen = config::self_write_gen();

    // Bridge the blocking std::mpsc receiver to the async runtime:
    // spawn a dedicated OS thread that drains std_rx and forwards batches over
    // a tokio mpsc channel, so the async loop can `await` without blocking the
    // executor.
    let (fwd_tx, mut fwd_rx) = tokio::sync::mpsc::channel::<
        Result<
            Vec<notify_debouncer_full::DebouncedEvent>,
            Vec<notify_debouncer_full::notify::Error>,
        >,
    >(16);

    std::thread::Builder::new()
        .name("watch-recv".into())
        .spawn(move || {
            while let Ok(batch) = std_rx.recv() {
                if fwd_tx.blocking_send(batch).is_err() {
                    break; // async side dropped (shutdown)
                }
            }
        })
        .ok();

    while let Some(batch) = fwd_rx.recv().await {
        // Filter: only act on events whose path is settings.json.
        // DebouncedEvent in notify-debouncer-full 0.3 has `event` (notify::Event)
        // and `time` fields; paths are in ev.event.paths (not ev.path).
        let touched = match &batch {
            Ok(events) => events.iter().any(|ev| {
                ev.event
                    .paths
                    .iter()
                    .any(|p| p.file_name().is_some_and(|n| n == "settings.json"))
            }),
            Err(errs) => {
                for e in errs {
                    tracing::debug!("watch: inotify error: {e}");
                }
                false
            }
        };

        if !touched {
            continue;
        }

        // Self-write suppression: if the daemon's generation advanced since
        // we last processed a batch, the write was daemon-originated —
        // suppress this event but update last_seen so a later external edit
        // still fires.
        let cur_gen = config::self_write_gen();
        if cur_gen != last_seen_gen {
            tracing::debug!(
                "watch: suppressing settings.json event (self-write gen {} → {})",
                last_seen_gen,
                cur_gen
            );
            last_seen_gen = cur_gen;
            continue;
        }

        // External edit confirmed — notify the input runtime via the
        // dedicated config-changed channel. Using a separate Notify avoids
        // adding a permanent receiver on the global Event broadcast bus,
        // which would defeat receiver_count()-based fast-paths (#163).
        tracing::info!(
            "watch: external settings.json change detected, notifying input runtime"
        );
        config_changed.notify_waiters();
        // last_seen_gen stays the same (no daemon write happened in this window).
    }

    tracing::debug!("watch: file-watch loop exited");
}
