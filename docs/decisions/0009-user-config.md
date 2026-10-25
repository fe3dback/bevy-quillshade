# 0009. User config

*The location is superseded by [0018](0018-welcome-about-config-location.md).*

**Decision.** `$XDG_CONFIG_HOME/shader-editor/config.ron` (otherwise `~/.config/…`, on Windows `%APPDATA%\shader-editor\config.ron`). Stores the window size, the list of recent projects (10) and the open tabs with the active tab for each project. Written atomically and only when changed. Without an argument the editor opens the last project. User settings are never written into the project.
