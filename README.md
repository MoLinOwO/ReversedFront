# ReversedFront Desktop

This project wraps the latest RF web build in the original Tauri/Rust desktop
application style. It keeps the latest frontend assets and loads the Mod
bundle after the official frontend.

## Development

```powershell
npm install
npm run mod:build
npm run dev
```

The Tauri shell serves the frontend on `127.0.0.1:8765`, provides the native
commands used by the Mod, and downloads missing `passionfruit` resources to
the per-user cache instead of embedding the multi-gigabyte media folder in
the installer.

## Mod online updates

The desktop app checks the `main` branch of
`https://github.com/MoLinOwO/ReversedFront_Public` after startup. When a new
commit is found, the control panel shows an update prompt. Confirming it
downloads the compiled Mod bundle and data files into the per-user data
directory; the app reloads afterwards and uses the updated files. If GitHub is
unavailable, the bundled or previously downloaded Mod continues to work.

## Desktop application updates

The desktop app checks the latest GitHub Release in the same repository. Push a
version tag such as `v3.1.1` to trigger the repository's GitHub Actions build;
the desktop app then detects the matching platform installer from that release.
The application no longer depends on a separate file-hosting update source.

The release workflow builds Windows, Linux, and universal macOS packages on
GitHub. The tag is synchronized into the application metadata before building,
so the installed app version and the Release version remain consistent. The
published Mod bundle is already compiled; Mod source folders are excluded from
the public repository and from the installer.
