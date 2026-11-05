# Local window-state path support

This source reuses `tauri-plugin-window-state` 2.4.1 from the existing adopted Cargo archive at `D:/Cache/Cargo/registry/cache/index.crates.io-1949cf8c6b5b557f/tauri-plugin-window-state-2.4.1.crate`. The archive is 95,023 bytes and its SHA256 is `73736611e14142408d15353e21e3cca2f12a3cfb523ad0ce85999b6d2ef1a704`, matching the existing package checksum in the application lockfile. The archive checksum and every extracted byte were checked before making the local edit.

The package retains its upstream version and `Apache-2.0 OR MIT` licensing. License texts, Rust sources, build script, command permission definitions, guest API source and the build script's required `api-iife.js` runtime asset come directly from the archive. Registry bookkeeping, the crate's separate Cargo lockfile and unrelated development assets are omitted.

The local change is confined to `src/lib.rs`. `Builder::with_state_dir(PathBuf)` makes both state restoration and saving use an explicitly selected directory. Save operations create that selected directory. Existing callers that omit the option retain the upstream path behavior. Window geometry, monitor checks, event handling, command names, permissions and serialized state shape retain the upstream implementation.

The application validates its selected D-drive directory before initializing Tauri and passes that directory through this new method. A failed selected directory is never replaced with Tauri's default configuration location.
