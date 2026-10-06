# SNlang VS Code / Cursor Extension

<p align="center">
  <img src="icon.png" alt="SNlang" width="128" height="128"/>
</p>

Syntax highlighting and basic language support for `.sn` files.

## Installation (Cursor — recommended)

You are almost certainly using **Cursor**, which keeps extensions in `~/.cursor/extensions` (not `~/.vscode/extensions`). Installing only into VS Code will **not** show the SNlang icon in Cursor.

**Option A — install from `.vsix`**

```bash
cd vscode-snlang
npx --yes @vscode/vsce package
"/Applications/Cursor.app/Contents/Resources/app/bin/cursor" --install-extension snlang-1.0.0.vsix
```

Then in Cursor: **Developer: Reload Window** (Command Palette).

Confirm under Extensions that **SNlang** is listed, and open any `.sn` file — explorer file icons use `icon.png` when the active file-icon theme supports language icons (default theme does).

**Option B — copy folder**

```bash
mkdir -p ~/.cursor/extensions
rm -rf ~/.cursor/extensions/snlang.snlang-1.0.0
cp -R vscode-snlang ~/.cursor/extensions/snlang.snlang-1.0.0
# leave out any .vsix inside the copy if present
```

Reload Cursor.

## Installation (VS Code)

```bash
cd vscode-snlang
npx --yes @vscode/vsce package
code --install-extension snlang-1.0.0.vsix
```

Or copy to `~/.vscode/extensions/snlang.snlang-1.0.0` and reload.

## Features

- Syntax highlighting for `.sn` files
- Comment support (`//` and `/* */`)
- Extension + language file icon (`icon.png`)

## Icon

`icon.png` is a **256×256 PNG** generated from the repo root `logo.svg`:

```bash
rsvg-convert -w 256 -h 256 ../logo.svg -o icon.png
```

VS Code / Cursor require PNG for extension and language icons (SVG is not supported there).

## Files

- `package.json` — extension manifest (`publisher`: `snlang`, `name`: `snlang`, `"icon": "icon.png"`)
- `icon.png` — extension and file-type icon
- `language-configuration.json` — language settings
- `syntaxes/snlang.json` — TextMate grammar
