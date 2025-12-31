# `difftastic.nvim`

A Neovim plugin that displays [`difftastic`](https://github.com/Wilfred/difftastic)'s structural diffs in a side-by-side
view with syntax highlighting.

<p align="center">
  <img src="assets/header.png" alt="difftastic.nvim" />
</p>

## Features

- Side-by-side diff view with synchronized scrolling
- Hierarchical file tree sidebar with directory collapsing
- Syntax highlighting for the source language
- Filler lines to visually indicate alignment gaps
- Support for both [jj](https://github.com/martinvonz/jj) and [git](https://git-scm.com/) version control

## Installation

### Requirements

- Neovim 0.9+
- [nui.nvim](https://github.com/MunifTanjim/nui.nvim)
- [difftastic](https://github.com/Wilfred/difftastic) (`difft` command)
- [jj](https://github.com/martinvonz/jj) or [git](https://git-scm.com/) version control
- Rust toolchain (only if building from source)

> [!WARNING]
>
> This plugin requires difftastic with `aligned_lines` support in JSON output. This feature is available in
> [this fork](https://github.com/clabby/difftastic/tree/cl/add-aligned-lines) until
> [PR #936](https://github.com/Wilfred/difftastic/pull/936) is merged upstream.
>
> To install the fork:
> ```sh
> # Clone with 'jj'
> jj git clone https://github.com/clabby/difftastic.git \
>     --colocate \
>     -b cl/add-aligned-lines
>
> # Or, clone with 'git'
> git clone https://github.com/clabby/difftastic.git -b cl/add-aligned-lines
>
> # Install 'difft' with the 'aligned_lines' feature
> cd difftastic && cargo install --path .
> ```

### lazy.nvim (recommended)

```lua
{
    "clabby/difftastic.nvim",
    dependencies = { "MunifTanjim/nui.nvim" },
    config = function()
        require("difftastic-nvim").setup({
            download = true, -- Auto-download pre-built binary
        })
    end,
}
```

### Building from source

If you prefer to build locally or pre-built binaries aren't available for your platform:

```lua
{
    "clabby/difftastic.nvim",
    dependencies = { "MunifTanjim/nui.nvim" },
    config = function()
        require("difftastic-nvim").setup()
    end,
}
```

Requires a Rust toolchain. The plugin automatically builds from source on first use if the library isn't found.

## Usage

### Commands

| Command | Description |
|---------|-------------|
| `:Difft <ref>` | Open diff view for a jj revset or git commit/range |
| `:DifftClose` | Close the diff view |

### Examples (jj)

```vim
" Diff the current change
:Difft @

" Diff the parent of the current change
:Difft @-

" Diff a specific revision
:Difft abc123
```

### Examples (git)

```vim
" Diff the last commit
:Difft HEAD

" Diff a specific commit
:Difft abc123

" Diff a commit range
:Difft main..HEAD
```

## Keybindings

All keybindings are buffer-local and configurable via `setup()`. Defaults:

| Key | Action |
|-----|--------|
| `]f` | Next file |
| `[f` | Previous file |
| `]c` | Next hunk |
| `[c` | Previous hunk |
| `<Tab>` | Toggle focus between file tree and diff |
| `<CR>` | Open file under cursor (in file tree) |
| `q` | Close diff view |

Filler lines (`╱╱╱`) indicate where content exists on one side but not the other.

## Configuration

```lua
require("difftastic-nvim").setup({
    download = false,            -- Auto-download pre-built binary (default: false)
    vcs = "jj",                  -- "jj" (default) or "git"
    highlight_mode = "treesitter", -- "treesitter" (default) or "difftastic"
    keymaps = {
        next_file = "]f",
        prev_file = "[f",
        next_hunk = "]c",
        prev_hunk = "[c",
        close = "q",
        focus_tree = "<Tab>",
        focus_diff = "<Tab>",
        select = "<CR>",
    },
    tree = {
        width = 40,
        icons = {
            enable = true,    -- use nvim-web-devicons if available
            dir_open = "",
            dir_closed = "",
        },
    },
    highlights = {
        -- Treesitter mode (background colors)
        DifftAdded = { bg = "#2d4a3e" },
        DifftRemoved = { bg = "#4a2d2d" },
        DifftAddedInline = { bg = "#3d6a4e" },
        DifftRemovedInline = { bg = "#6a3d3d" },
        -- Difftastic mode (foreground colors)
        DifftAddedFg = { fg = "#9ece6a" },
        DifftRemovedFg = { fg = "#f7768e" },
        DifftAddedInlineFg = { fg = "#9ece6a", bold = true },
        DifftRemovedInlineFg = { fg = "#f7768e", bold = true },
        -- Shared
        DifftFiller = { fg = "#3b4261" },
        DifftFileAdded = { fg = "#9ece6a" },
        DifftFileDeleted = { fg = "#f7768e" },
        DifftTreeCurrent = { bg = "#3b4261", bold = true },
        DifftDirectory = { fg = "#7aa2f7", bold = true },
    },
})
```

All options are optional. Only specify what you want to override.

### Highlight Modes

The `highlight_mode` option controls how syntax highlighting is applied:

- **`treesitter`** (default): Full syntax highlighting via Neovim's treesitter. Changes are shown with background colors.
- **`difftastic`**: Minimal highlighting like the CLI. No syntax colors; changes are shown with foreground colors (green/red) to make diffs more prominent.

## Highlight Groups

**Treesitter mode** (background colors):

| Group | Description |
|-------|-------------|
| `DifftAdded` | Added lines background |
| `DifftRemoved` | Removed lines background |
| `DifftAddedInline` | Inline added text |
| `DifftRemovedInline` | Inline removed text |

**Difftastic mode** (foreground colors):

| Group | Description |
|-------|-------------|
| `DifftAddedFg` | Added text (green) |
| `DifftRemovedFg` | Removed text (red) |
| `DifftAddedInlineFg` | Inline added (green, bold) |
| `DifftRemovedInlineFg` | Inline removed (red, bold) |

**Shared**:

| Group | Description |
|-------|-------------|
| `DifftFiller` | Filler lines for alignment gaps |
| `DifftDirectory` | Directory names in tree |
| `DifftFileAdded` | Added file in tree |
| `DifftFileDeleted` | Deleted file in tree |
| `DifftTreeCurrent` | Current file highlight in tree |

## License


[MIT](./LICENSE.md)
