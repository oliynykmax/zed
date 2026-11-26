# Lean InfoView

A dock panel that shows the Lean 4 infoview, the same React app as VS Code's Lean extension (`@leanprover/infoview`), in a `WKWebView`. It needs GPUI native views, so it exists on macOS only. Other platforms keep `editor: show lean goal`.

## Run it

1. Install Lean through elan:

   ```sh
   curl https://raw.githubusercontent.com/leanprover/elan/master/elan-init.sh -sSf | sh -s -- -y
   ```

2. Build Zed. The Metal shaders need the full Xcode toolchain:

   ```sh
   DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer cargo build -p zed
   ```

3. Start the build. `--user-data-dir` keeps its settings and extensions apart from your main Zed:

   ```sh
   target/debug/zed --user-data-dir ~/.zed-lean-dev path/to/lean-project
   ```

4. Install the Lean 4 extension: run `zed: extensions`, search for "Lean 4", and install it.

5. Turn off the extension's `lean-proxy`, which the panel was not tested with. If Zed cannot find `lake` on its `PATH`, also set the binary. In the settings file (`zed: open settings file`):

   ```json
   {
     "lsp": {
       "lean4-lsp": {
         "binary": { "path": "/Users/you/.elan/bin/lake", "arguments": ["serve", "--"] },
         "settings": { "proxy": { "enabled": false } }
       }
     }
   }
   ```

   For a single `.lean` file outside a Lake project, use `"path": "/Users/you/.elan/bin/lean", "arguments": ["--server"]` instead.

6. Trust the project when Zed asks. Zed starts no language servers in a project it does not trust.

7. Open a `.lean` file. The panel opens on the right the first time, and follows the cursor. `lean: toggle infoview` closes and opens it.

## Debugging

- The page's errors and warnings go to the Zed log with the prefix `Lean infoview:`. Run with `RUST_LOG=info` to see them in the terminal.
- The web view is inspectable. In Safari, turn on the Develop menu, then pick the Zed process under Develop to get the Web Inspector.

## Update the infoview

The infoview build is committed in `assets/vendor`, so building Zed needs no Node. To move to another release, set `version` and `integrity` in `script/vendor-infoview` (the integrity is the `dist.integrity` from `npm view @leanprover/infoview@<version>`), then run the script.

## Not done yet

- macOS only. The panel is a `WKWebView`.
- Local projects only. Remote projects get no infoview.
- Changing a display setting does not repaint the goals already shown. This is a limit of the upstream infoview.
- No custom CSS for the infoview.
- The lines the server is still processing are marked in the gutter only, not in the scrollbar.
