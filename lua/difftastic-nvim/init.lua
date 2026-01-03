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
        goto_file = "gf",
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
        -- Manual merge to preserve explicit false values (tbl_extend ignores them)
        -- Note: nil values are skipped by pairs(), so they keep the default
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
--- @param revset string|nil jj revset or git commit range (nil = unstaged, "--staged" = staged)
function M.open(revset)
    if M.state.tree_win or M.state.left_win or M.state.right_win then
        M.close()
    end

    local result
    if revset == nil then
        result = binary.get().run_diff_unstaged(M.config.vcs)
    elseif revset == "--staged" then
        result = binary.get().run_diff_staged(M.config.vcs)
    else
        result = binary.get().run_diff(revset, M.config.vcs)
    end
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

--- Show a file and jump to a specific hunk after rendering completes.
--- @param idx number File index to show
--- @param hunk_fn function Hunk function to call (e.g., diff.first_hunk or diff.last_hunk)
local function show_file_and_jump_to_hunk(idx, hunk_fn)
    M.show_file(idx)
    vim.schedule(function()
        hunk_fn(M.state)
    end)
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
--- Otherwise, wraps to the first hunk of the current file.
function M.next_hunk()
    local jumped = diff.next_hunk(M.state)
    if not jumped then
        if M.config.hunk_wrap_file then
            local next_idx = tree.next_file_in_display_order(M.state.current_file_idx)
            if next_idx then
                show_file_and_jump_to_hunk(next_idx, diff.first_hunk)
            else
                -- At last file, wrap to first file
                local first_idx = tree.first_file_in_display_order()
                if first_idx then
                    show_file_and_jump_to_hunk(first_idx, diff.first_hunk)
                end
            end
        else
            -- Wrap within current file
            diff.first_hunk(M.state)
        end
    end
end

--- Navigate to the previous hunk.
--- If hunk_wrap_file is enabled and at the first hunk, wraps to the last hunk of the previous file.
--- Otherwise, wraps to the last hunk of the current file.
function M.prev_hunk()
    local jumped = diff.prev_hunk(M.state)
    if not jumped then
        if M.config.hunk_wrap_file then
            local prev_idx = tree.prev_file_in_display_order(M.state.current_file_idx)
            if prev_idx then
                show_file_and_jump_to_hunk(prev_idx, diff.last_hunk)
            else
                -- At first file, wrap to last file
                local last_idx = tree.last_file_in_display_order()
                if last_idx then
                    show_file_and_jump_to_hunk(last_idx, diff.last_hunk)
                end
            end
        else
            -- Wrap within current file
            diff.last_hunk(M.state)
        end
    end
end

--- Go to the file at the current cursor position in an editable buffer.
--- Opens in a previous tabpage if one exists, otherwise creates a new tab.
--- Only works from the right pane (new/working version of the file).
function M.goto_file()
    local state = M.state
    local current_win = vim.api.nvim_get_current_win()

    -- Only works from right pane (new version), not tree or left pane
    if current_win ~= state.right_win or current_win == state.tree_win then
        return
    end

    local file = state.files[state.current_file_idx]
    if not file then
        return
    end

    -- Deleted files have no right-side content to navigate to
    if file.status == "deleted" then
        return
    end

    -- Get current cursor position (row is 1-indexed, col is 0-indexed)
    local cursor = vim.api.nvim_win_get_cursor(current_win)
    local row, col = cursor[1], cursor[2]
    local aligned = file.aligned_lines and file.aligned_lines[row]

    -- Find the target line number (right side = new version)
    local target_line
    if aligned and aligned[2] then
        -- Direct mapping exists
        target_line = aligned[2] + 1 -- 0-indexed to 1-indexed
    else
        -- Filler line - find nearest non-filler line
        -- Search upward first, then downward
        for offset = 1, #file.aligned_lines do
            -- Check above
            if row - offset >= 1 then
                local above = file.aligned_lines[row - offset]
                if above and above[2] then
                    target_line = above[2] + 1
                    break
                end
            end
            -- Check below
            if row + offset <= #file.aligned_lines then
                local below = file.aligned_lines[row + offset]
                if below and below[2] then
                    target_line = below[2] + 1
                    break
                end
            end
        end
    end

    -- Fallback to line 1 if no mapping found
    target_line = target_line or 1

    local filepath = file.path

    -- Find previous tabpage or create new one
    local tabs = vim.api.nvim_list_tabpages()
    local current_idx = vim.fn.tabpagenr()
    local target_tab = current_idx > 1 and tabs[current_idx - 1] or nil

    if target_tab then
        vim.api.nvim_set_current_tabpage(target_tab)
    else
        vim.cmd("tabnew")
    end

    -- Open file and jump to line and column
    vim.cmd("edit " .. vim.fn.fnameescape(filepath))
    -- Clamp column to line length to avoid errors on shorter lines
    local line_content = vim.api.nvim_buf_get_lines(0, target_line - 1, target_line, false)[1] or ""
    local target_col = math.min(col, math.max(0, #line_content - 1))
    vim.api.nvim_win_set_cursor(0, { target_line, target_col })
end

--- Update binary to latest release.
function M.update()
    binary.update()
end

return M
