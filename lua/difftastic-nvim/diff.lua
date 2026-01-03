--- Side-by-side diff display with synchronized scrolling.
local M = {}

local FILLER = string.rep("╱", 500)

--- Line positions where hunks start (1-indexed)
--- @type number[]
M.hunk_positions = {}

--- Maps difftastic language names to Vim filetypes
local FILETYPES = {
    Rust = "rust",
    Lua = "lua",
    TOML = "toml",
    JSON = "json",
    JavaScript = "javascript",
    TypeScript = "typescript",
    Python = "python",
    Go = "go",
    C = "c",
    ["C++"] = "cpp",
    Java = "java",
    Ruby = "ruby",
    Shell = "sh",
    Bash = "bash",
    Markdown = "markdown",
    YAML = "yaml",
    HTML = "html",
    CSS = "css",
}

--- Set buffer options for diff buffers.
--- @param buf number Buffer handle
local function setup_diff_buffer(buf)
    vim.bo[buf].buftype = "nofile"
    vim.bo[buf].bufhidden = "wipe"
    vim.bo[buf].swapfile = false
    vim.bo[buf].modifiable = false
end

--- Set window options for diff windows.
--- @param win number Window handle
local function setup_diff_window(win)
    vim.wo[win].scrollbind = true
    vim.wo[win].cursorbind = true
    vim.wo[win].number = true
    vim.wo[win].signcolumn = "no"
end

--- Open the side-by-side diff panes.
--- @param state table Plugin state
function M.open(state)
    vim.cmd("vsplit")
    state.right_win = vim.api.nvim_get_current_win()
    state.right_buf = vim.api.nvim_create_buf(false, true)
    vim.api.nvim_win_set_buf(state.right_win, state.right_buf)

    vim.cmd("wincmd h")
    vim.cmd("vsplit")
    state.left_win = vim.api.nvim_get_current_win()
    state.left_buf = vim.api.nvim_create_buf(false, true)
    vim.api.nvim_win_set_buf(state.left_win, state.left_buf)

    setup_diff_buffer(state.left_buf)
    setup_diff_buffer(state.right_buf)
    setup_diff_window(state.left_win)
    setup_diff_window(state.right_win)
end

--- Render a file's diff content into the left/right panes.
--- @param state table Plugin state
--- @param file table File data with rows, hunk_starts, language
function M.render(state, file)
    local config = require("difftastic-nvim").config
    local rows = file.rows or {}

    M.hunk_positions = {}
    for _, pos in ipairs(file.hunk_starts or {}) do
        table.insert(M.hunk_positions, pos + 1)
    end

    if #rows == 0 then
        vim.bo[state.left_buf].modifiable = true
        vim.bo[state.right_buf].modifiable = true
        vim.api.nvim_buf_set_lines(state.left_buf, 0, -1, false, { "-- Empty --" })
        vim.api.nvim_buf_set_lines(state.right_buf, 0, -1, false, { "-- Empty --" })
        vim.bo[state.left_buf].modifiable = false
        vim.bo[state.right_buf].modifiable = false
        return
    end

    local left_lines, right_lines = {}, {}
    for _, row in ipairs(rows) do
        table.insert(left_lines, row.left.content)
        table.insert(right_lines, row.right.content)
    end

    vim.bo[state.left_buf].modifiable = true
    vim.bo[state.right_buf].modifiable = true
    vim.api.nvim_buf_set_lines(state.left_buf, 0, -1, false, left_lines)
    vim.api.nvim_buf_set_lines(state.right_buf, 0, -1, false, right_lines)
    vim.bo[state.left_buf].modifiable = false
    vim.bo[state.right_buf].modifiable = false

    -- Apply syntax highlighting based on mode
    local use_treesitter = config.highlight_mode ~= "difftastic"
    if use_treesitter then
        local ft = FILETYPES[file.language]
        if ft then
            vim.defer_fn(function()
                if vim.api.nvim_buf_is_valid(state.left_buf) then
                    vim.bo[state.left_buf].filetype = ft
                end
                if vim.api.nvim_buf_is_valid(state.right_buf) then
                    vim.bo[state.right_buf].filetype = ft
                end
            end, 10)
        end
    end

    local left_ns = vim.api.nvim_create_namespace("difft-left")
    local right_ns = vim.api.nvim_create_namespace("difft-right")
    vim.api.nvim_buf_clear_namespace(state.left_buf, left_ns, 0, -1)
    vim.api.nvim_buf_clear_namespace(state.right_buf, right_ns, 0, -1)

    -- Choose highlight groups based on mode
    -- treesitter mode: background colors (same for full line and inline)
    -- difftastic mode: foreground colors (like CLI, bold for inline)
    local removed_hl = use_treesitter and "DifftRemoved" or "DifftRemovedFg"
    local removed_inline_hl = use_treesitter and "DifftRemoved" or "DifftRemovedInlineFg"
    local added_hl = use_treesitter and "DifftAdded" or "DifftAddedFg"
    local added_inline_hl = use_treesitter and "DifftAdded" or "DifftAddedInlineFg"

    -- Apply diff highlights (additions/removals)
    for i, row in ipairs(rows) do
        local line = i - 1

        for _, hl in ipairs(row.left.highlights) do
            local group = hl["end"] == -1 and removed_hl or removed_inline_hl
            vim.api.nvim_buf_add_highlight(state.left_buf, left_ns, group, line, hl.start, hl["end"])
        end

        for _, hl in ipairs(row.right.highlights) do
            local group = hl["end"] == -1 and added_hl or added_inline_hl
            vim.api.nvim_buf_add_highlight(state.right_buf, right_ns, group, line, hl.start, hl["end"])
        end

        if row.left.is_filler then
            vim.api.nvim_buf_set_extmark(state.left_buf, left_ns, line, 0, {
                virt_text = { { FILLER, "DifftFiller" } },
                virt_text_pos = "overlay",
            })
        end

        if row.right.is_filler then
            vim.api.nvim_buf_set_extmark(state.right_buf, right_ns, line, 0, {
                virt_text = { { FILLER, "DifftFiller" } },
                virt_text_pos = "overlay",
            })
        end
    end

    vim.api.nvim_win_set_cursor(state.left_win, { 1, 0 })
    vim.api.nvim_win_set_cursor(state.right_win, { 1, 0 })
end

--- Get the current diff window (left or right).
--- @param state table Plugin state
--- @return number|nil Window handle or nil if invalid
local function get_diff_win(state)
    local current = vim.api.nvim_get_current_win()
    local win = current == state.right_win and state.right_win or state.left_win
    if win and vim.api.nvim_win_is_valid(win) then
        return win
    end
    return nil
end

--- Jump to the next hunk.
--- @param state table Plugin state
--- @return boolean True if jumped to a hunk, false if at/past last hunk
function M.next_hunk(state)
    if #M.hunk_positions == 0 then
        return false
    end
    local win = get_diff_win(state)
    if not win then
        return false
    end

    local line = vim.api.nvim_win_get_cursor(win)[1]
    for _, pos in ipairs(M.hunk_positions) do
        if pos > line then
            vim.api.nvim_win_set_cursor(win, { pos, 0 })
            return true
        end
    end
    return false
end

--- Jump to the previous hunk.
--- @param state table Plugin state
--- @return boolean True if jumped to a hunk, false if at/before first hunk
function M.prev_hunk(state)
    if #M.hunk_positions == 0 then
        return false
    end
    local win = get_diff_win(state)
    if not win then
        return false
    end

    local line = vim.api.nvim_win_get_cursor(win)[1]
    for i = #M.hunk_positions, 1, -1 do
        if M.hunk_positions[i] < line then
            vim.api.nvim_win_set_cursor(win, { M.hunk_positions[i], 0 })
            return true
        end
    end
    return false
end

--- Jump to the first hunk.
--- @param state table Plugin state
function M.first_hunk(state)
    if #M.hunk_positions == 0 then
        return
    end
    local win = get_diff_win(state)
    if win then
        vim.api.nvim_win_set_cursor(win, { M.hunk_positions[1], 0 })
    end
end

--- Jump to the last hunk.
--- @param state table Plugin state
function M.last_hunk(state)
    if #M.hunk_positions == 0 then
        return
    end
    local win = get_diff_win(state)
    if win then
        vim.api.nvim_win_set_cursor(win, { M.hunk_positions[#M.hunk_positions], 0 })
    end
end

return M
