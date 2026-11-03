Temporary base64 staging for PFD icons.

GitHub MCP `push_files` / `create_or_update_file` UTF-8-encodes string content, which corrupts PNG/ICO/ICNS. A one-shot Actions workflow (`materialize-pfd-icons.yml`) decodes these into real binaries on `feat/app-icon`, then removes this staging directory and itself.
