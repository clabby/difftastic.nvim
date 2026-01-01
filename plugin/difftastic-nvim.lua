if vim.g.loaded_difftastic_nvim then
    return
end
vim.g.loaded_difftastic_nvim = true

-- Generate helptags if needed
local source = debug.getinfo(1, "S").source:sub(2)
local doc_dir = vim.fn.fnamemodify(source, ":h:h") .. "/doc"
if vim.fn.isdirectory(doc_dir) == 1 then
    vim.cmd.helptags(doc_dir)
end

vim.api.nvim_create_user_command("Difft", function(opts)
    local args = opts.args
    if args == "" then
        -- No args: show unstaged changes
        require("difftastic-nvim").open(nil)
    elseif args == "--staged" then
        -- Show staged changes
        require("difftastic-nvim").open("--staged")
    else
        -- Revset/commit range
        local revset = args:gsub("^['\"](.+)['\"]$", "%1")
        require("difftastic-nvim").open(revset)
    end
end, {
    nargs = "?",
    desc = "Open difftastic diff view (no args = unstaged, --staged = staged, or revset/commit)",
})

vim.api.nvim_create_user_command("DifftClose", function()
    require("difftastic-nvim").close()
end, {
    desc = "Close difftastic diff view",
})

vim.api.nvim_create_user_command("DifftUpdate", function()
    require("difftastic-nvim").update()
end, {
    desc = "Update difftastic-nvim binary to latest release",
})
