# Configuration

To override global configuration parameters, create a `config.toml` file located in your config directory:

- Linux and Mac: `~/.config/helix/config.toml`
- Windows: `%AppData%\helix\config.toml`

> 💡 You can easily open the config file by typing `:config-open` within Helix normal mode.

Example config:

```toml
theme = "onedark"

[editor]
line-number = "relative"
mouse = false

[editor.cursor-shape]
insert = "bar"
normal = "block"
select = "underline"

[editor.file-picker]
hidden = false
```

Set `restore-session = false` to disable restoring the previous workspace's
buffers, selections, and scroll positions on startup (and automatic session
saves on exit). The session is stored under Helix's data directory, keyed by
the workspace path. `:session-save` and `:session-clear` control it manually.

The sidebar width is configured with:

```toml
[editor.sidebar]
width = 35
min-width = 20
max-width = 50
git-status = true
```

The sidebar is auto-hidden when the terminal cannot leave at least 40 columns
for the editor. File-tree ignore rules reuse `[editor.file-explorer]`.

You can use a custom configuration file by specifying it with the `-c` or
`--config` command line argument, for example `hx -c path/to/custom-config.toml`.
You can reload the config file by issuing the `:config-reload` command. Alternatively, on Unix operating systems, you can reload it by sending the USR1
signal to the Helix process, such as by using the command `pkill -USR1 hx`.

Finally, you can have a `config.toml` and a `languages.toml` local to a project by putting it under a `.helix` directory in your repository.
Its settings will be merged with the configuration directory and the built-in configuration.

