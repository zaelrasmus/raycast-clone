Installing on another PC

If that PC has Rust and the MSVC build tools — clone the repo and run:

powershell -ExecutionPolicy Bypass -File scripts\install.ps1

That builds it, copies it to %LOCALAPPDATA%\Programs\Lumen, puts it on PATH, adds a Start Menu entry and starts it at login.

If it doesn't — you don't need a toolchain at all. The binary is fully self-contained: the font, icons, themes, default sheets and example extensions are all compiled into the 11 MB lumen.exe. So:

1. Copy %LOCALAPPDATA%\Programs\Lumen\lumen.exe to the other PC
2. Run it once — it creates %APPDATA%\Lumen\config with defaults
3. For autostart, drop a shortcut to it in shell:startup (Win+R → shell:startup)

Your settings don't travel with the binary. To carry them over, copy %APPDATA%\Lumen\config\ — that's your config, themes, sheets, scripts and snippets. Everything in there is plain JSON you can keep in a repo or a synced folder.
