--- Difftastic side-by-side diff viewer for Neovim.
local M = {}

local binary = require("difftastic-nvim.binary")
local diff = require("difftastic-nvim.diff")
local tree = require("difftastic-nvim.tree")
local highlight = require("difftastic-nvim.highlight")
local keymaps = require("difftastic-nvim.keymaps")

--- Default configuration
M.config = {
    download = false,
    vcs = "jj",
    --- Highlight mode: "treesitter" (full syntax) or "difftastic" (no syntax, colored changes only)
    highlight_mode = "treesitter",
    --- When true, next_hunk at last hunk wraps to next file (and prev_hunk to prev file)
    hunk_wrap_file = false,
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
            enable = true,
            dir_open = "",
            dir_closed = "",
        },
    },
}

--- Current diff state
M.state = {
    current_file_idx = 1,
    files = {},
    tree_win = nil,
    tree_buf = nil,
    left_win = nil,
    left_buf = nil,
    right_win = nil,
    right_buf = nil,
    original_buf = nil,
}

--- Initialize the plugin with user options.
--- @param opts table|nil User configuration
function M.setup(opts)
    opts = opts or {}

    -- Merge config
    if opts.download ~= nil then
        M.config.download = opts.download
    end
    if opts.vcs then
        M.config.vcs = opts.vcs
    end
    if opts.highlight_mode then
        M.config.highlight_mode = opts.highlight_mode
    end
    if opts.hunk_wrap_file ~= nil then
        M.config.hunk_wrap_file = opts.hunk_wrap_file
    end
    if opts.keymaps then
        -- Manual merge to preserve explicit false/nil values (tbl_extend ignores nil)
        for k, v in pairs(opts.keymaps) do
            M.config.keymaps[k] = v
        end
    end
    if opts.tree then
        if opts.tree.icons then
            M.config.tree.icons = vim.tbl_extend("force", M.config.tree.icons, opts.tree.icons)
        end
        if opts.tree.width then
            M.config.tree.width = opts.tree.width
        end
    end

    highlight.setup(opts.highlights)
    binary.ensure_exists(M.config.download)
end

--- Open diff view for a revision/commit range.
--- @param revset string jj revset or git commit range
function M.open(revset)
    if M.state.tree_win or M.state.left_win or M.state.right_win then
        M.close()
    end

    local result = binary.get().run_diff(revset, M.config.vcs)
    if not result.files or #result.files == 0 then
        vim.notify("No changes found", vim.log.levels.INFO)
        return
    end

    M.state.files = result.files
    M.state.current_file_idx = 1

    local original_win = vim.api.nvim_get_current_win()
    M.state.original_buf = vim.api.nvim_get_current_buf()

    tree.open(M.state)
    diff.open(M.state)
    keymaps.setup(M.state)

    -- Close original window if it's not one of our diff windows
    if
        vim.api.nvim_win_is_valid(original_win)
        and original_win ~= M.state.tree_win
        and original_win ~= M.state.left_win
        and original_win ~= M.state.right_win
    then
        vim.api.nvim_win_close(original_win, false)
    end

    local first_idx = tree.first_file_in_display_order()
    if first_idx then
        M.show_file(first_idx)
    end
end

--- Close the diff view.
function M.close()
    local wins = {}
    for _, win in ipairs({ M.state.tree_win, M.state.left_win, M.state.right_win }) do
        if win and vim.api.nvim_win_is_valid(win) then
            table.insert(wins, win)
        end
    end

    if #wins == 0 then
        return
    end

    for _, win in ipairs(wins) do
        if vim.api.nvim_win_is_valid(win) then
            if #vim.api.nvim_list_wins() > 1 then
                vim.api.nvim_win_close(win, true)
            else
                vim.api.nvim_set_current_win(win)
                if M.state.original_buf and vim.api.nvim_buf_is_valid(M.state.original_buf) then
                    vim.api.nvim_win_set_buf(win, M.state.original_buf)
                else
                    vim.cmd("enew")
                end
            end
        end
    end

    M.state = {
        current_file_idx = 1,
        files = {},
        tree_win = nil,
        tree_buf = nil,
        left_win = nil,
        left_buf = nil,
        right_win = nil,
        right_buf = nil,
        original_buf = nil,
    }
end

--- Show a specific file by index.
--- @param idx number File index (1-based)
function M.show_file(idx)
    if idx < 1 or idx > #M.state.files then
        return
    end
    M.state.current_file_idx = idx
    diff.render(M.state, M.state.files[idx])
    tree.highlight_current(M.state)
end

--- Navigate to the next file.
function M.next_file()
    local next_idx = tree.next_file_in_display_order(M.state.current_file_idx)
    if next_idx then
        M.show_file(next_idx)
    end
end

--- Navigate to the previous file.
function M.prev_file()
    local prev_idx = tree.prev_file_in_display_order(M.state.current_file_idx)
    if prev_idx then
        M.show_file(prev_idx)
    end
end

--- Navigate to the next hunk.
--- If hunk_wrap_file is enabled and at the last hunk, wraps to the first hunk of the next file.
function M.next_hunk()
    local jumped = diff.next_hunk(M.state)
    if not jumped and M.config.hunk_wrap_file then
        local next_idx = tree.next_file_in_display_order(M.state.current_file_idx)
        if next_idx then
            M.show_file(next_idx)
            vim.defer_fn(function()
                diff.first_hunk(M.state)
            end, 10)
        else
            -- At last file, wrap to first file
            local first_idx = tree.first_file_in_display_order()
            if first_idx then
                M.show_file(first_idx)
                vim.defer_fn(function()
                    diff.first_hunk(M.state)
                end, 10)
            end
        end
    end
end

--- Navigate to the previous hunk.
--- If hunk_wrap_file is enabled and at the first hunk, wraps to the last hunk of the previous file.
function M.prev_hunk()
    local jumped = diff.prev_hunk(M.state)
    if not jumped and M.config.hunk_wrap_file then
        local prev_idx = tree.prev_file_in_display_order(M.state.current_file_idx)
        if prev_idx then
            M.show_file(prev_idx)
            vim.defer_fn(function()
                diff.last_hunk(M.state)
            end, 10)
        else
            -- At first file, wrap to last file
            local last_idx = tree.last_file_in_display_order()
            if last_idx then
                M.show_file(last_idx)
                vim.defer_fn(function()
                    diff.last_hunk(M.state)
                end, 10)
            end
        end
    end
end

--- Update binary to latest release.
function M.update()
    binary.update()
end

return M
