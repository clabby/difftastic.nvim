--- Buffer-local keymaps for diff navigation.
local M = {}

--- Set up keymaps for diff buffers.
--- @param buf number Buffer handle
--- @param state table Plugin state
local function setup_diff_keymaps(buf, state)
    local difft = require("difftastic-nvim")
    local keys = difft.config.keymaps

    if keys.next_file then
        vim.keymap.set("n", keys.next_file, difft.next_file, { buffer = buf })
    end
    if keys.prev_file then
        vim.keymap.set("n", keys.prev_file, difft.prev_file, { buffer = buf })
    end
    if keys.next_hunk then
        vim.keymap.set("n", keys.next_hunk, difft.next_hunk, { buffer = buf })
    end
    if keys.prev_hunk then
        vim.keymap.set("n", keys.prev_hunk, difft.prev_hunk, { buffer = buf })
    end
    if keys.close then
        vim.keymap.set("n", keys.close, difft.close, { buffer = buf })
    end
    if keys.goto_file then
        vim.keymap.set("n", keys.goto_file, difft.goto_file, { buffer = buf })
    end
    if keys.focus_tree then
        vim.keymap.set("n", keys.focus_tree, function()
            if state.tree_win and vim.api.nvim_win_is_valid(state.tree_win) then
                vim.api.nvim_set_current_win(state.tree_win)
            end
        end, { buffer = buf })
    end
end

--- Set up keymaps for tree buffer.
--- @param state table Plugin state
local function setup_tree_keymaps(state)
    local difft = require("difftastic-nvim")
    local keys = difft.config.keymaps
    local buf = state.tree_buf

    if keys.focus_diff then
        vim.keymap.set("n", keys.focus_diff, function()
            if state.left_win and vim.api.nvim_win_is_valid(state.left_win) then
                vim.api.nvim_set_current_win(state.left_win)
            end
        end, { buffer = buf })
    end
    if keys.next_file then
        vim.keymap.set("n", keys.next_file, difft.next_file, { buffer = buf })
    end
    if keys.prev_file then
        vim.keymap.set("n", keys.prev_file, difft.prev_file, { buffer = buf })
    end
end

--- Setup all keymaps for the diff view.
--- @param state table Plugin state
function M.setup(state)
    for _, buf in ipairs({ state.left_buf, state.right_buf }) do
        if buf and vim.api.nvim_buf_is_valid(buf) then
            setup_diff_keymaps(buf, state)
        end
    end

    if state.tree_buf and vim.api.nvim_buf_is_valid(state.tree_buf) then
        setup_tree_keymaps(state)
    end
end

return M
