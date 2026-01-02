-- Minimal init for running tests
-- Add plugin to runtime path
local plugin_root = vim.fn.fnamemodify(debug.getinfo(1, "S").source:sub(2), ":h:h")
vim.opt.rtp:prepend(plugin_root)

-- Try to load plenary if available
local ok, _ = pcall(require, "plenary")
if not ok then
    print("Warning: plenary.nvim not found. Some tests may fail.")
end
