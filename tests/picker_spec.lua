local picker = require("difftastic-nvim.picker")

describe("picker preview highlights", function()
    local buf
    local win

    before_each(function()
        vim.cmd("enew")
        win = vim.api.nvim_get_current_win()
        buf = vim.api.nvim_get_current_buf()
    end)

    it("overlays highlight on header and description lines", function()
        local lines = {
            "○ xqrwlozy ben@clab.by 1 hour ago \27[32m9023e373\27[0m",
            "│  (no description set)",
            "○ wpmrqlvy ben@clab.by 1 hour ago 484bfb04",
        }
        vim.api.nvim_buf_set_lines(buf, 0, -1, false, lines)

        picker._apply_preview_hover_highlight(buf, win, lines, "9023e373a337c54aaa66ac5cb5b0d7622a136beb")

        local marks = vim.api.nvim_buf_get_extmarks(buf, -1, 0, -1, { details = true })
        local hit = {}
        for _, mark in ipairs(marks) do
            local row = mark[2]
            local details = mark[4] or {}
            if details.hl_group == "DifftPickerPreviewHover" then
                hit[row] = true
            end
        end

        assert.is_true(hit[0])
        assert.is_true(hit[1])
    end)

    it("only highlights header when next line is another commit header", function()
        local lines = {
            "○ xqrwlozy ben@clab.by 1 hour ago 9023e373",
            "○ wpmrqlvy ben@clab.by 1 hour ago 484bfb04",
        }
        vim.api.nvim_buf_set_lines(buf, 0, -1, false, lines)

        picker._apply_preview_hover_highlight(buf, win, lines, "9023e373a337c54aaa66ac5cb5b0d7622a136beb")

        local marks = vim.api.nvim_buf_get_extmarks(buf, -1, 0, -1, { details = true })
        local rows = {}
        for _, mark in ipairs(marks) do
            local row = mark[2]
            local details = mark[4] or {}
            if details.hl_group == "DifftPickerPreviewHover" then
                rows[row] = true
            end
        end

        assert.is_true(rows[0])
        assert.is_nil(rows[1])
    end)
end)
