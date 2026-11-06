# Contributing

- Open pull requests against `main` and say what they change and what you tested it on (distribution, GPU, Proton version).
- Problems that are not Linux-specific (the game itself, the runtime, the recompiler) are best reported to
  [featureJosh's Pokopia Recomp](https://github.com/featureJosh/pokopia-recomp) as well, where that code is maintained.
- Code for this game goes in `recomp/games/pkp/` and `runtime/games/pkp/`; the rest is shared with other games.
- What differs on Linux lives in `launcher/src-tauri/src/host.rs`; the launcher's Linux tests in the `linux_tests` modules at the
  end of the files they test (`cargo test` in `launcher/src-tauri`).
- Never commit anything from a game dump or made from one: `exefs`/`romfs` files, `pkp.exe`, shader caches, saves, logs.

## Keeping up with Pokopia Recomp

Each release of this edition is based on a Pokopia Recomp release. To move to a new one:

1. `git remote add upstream https://github.com/featureJosh/pokopia-recomp.git` (once), then `git fetch upstream --tags`.
2. Merge the release **tag** (for example `git merge v0.7.0`), not upstream's `main`, so the source matches the release files
   the download is built from.
3. Resolve conflicts keeping the Linux changes; run `cargo test` and a full Set up on Linux.
4. Set the release in `linux/package.sh` (`UPSTREAM_VERSION` and the zip's SHA-256 from upstream's `SHA256SUMS.txt`) and the
   launcher's version (`launcher/package.json`, `launcher/src-tauri/Cargo.toml`, `launcher/src-tauri/tauri.conf.json`:
   `<upstream>-linux.1`), add a section to [CHANGELOG.md](CHANGELOG.md), and tag `v<upstream>-linux.<n>`.

## Building

- **The release download:** `linux/package.sh` (needs podman). It builds both launchers in an Ubuntu 22.04 container
  (`linux/Containerfile`: Node and Rust pinned by version and checksum) and puts them together with the official Pokopia Recomp
  release files into `linux/build/dist/`.
- **Only the launcher, for development:** in any distribution (or a Fedora toolbox on Bazzite) with Rust, Node.js and the
  webkit2gtk-4.1, gtk3 and libsoup3 development packages: `npm ci && npm run tauri build -- --no-bundle` in `launcher/`.
- **The runtime and the game build tools** come from Pokopia Recomp's release; building them from source is described in
  `runtime/` and `recomp/`.
